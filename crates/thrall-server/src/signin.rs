//! Signing browsers and PCs in as a person, without passwords. The owner makes a one-time sign-in
//! link for someone; the browser that opens it trades the code (`POST /v1/signin`) for its own
//! device token, kept in that browser, and presents it in `hello` like the access token. Only the
//! token's SHA-256 is stored. Signing a device out (or removing its person) closes its open
//! connections. The access token in the data folder stays the owner's.

use std::sync::Mutex;
use std::time::Instant;

use sha2::{Digest, Sha256};
use thrall_agent::AgentError;
use thrall_core::constants::id_prefix;
use thrall_core::ids::{new_id, now_ms};
use thrall_store::{Device, Person};
use tokio::sync::watch;

use crate::connection::Shared;
use crate::constants::{
    DEVICE_NAME_MAX_CHARS, FALLBACK_DEVICE_NAME, SIGNIN_CODE_BYTES, SIGNIN_CODE_LIFETIME,
    SIGNIN_MAX_OPEN,
};
use crate::token;

/// A sign-in link waiting to be opened.
struct Code {
    code: String,
    person: String,
    expires: Instant,
}

/// Open sign-in codes, and the signal that a device was signed out.
pub struct SignIns {
    codes: Mutex<Vec<Code>>,
    /// Bumped when devices are signed out, so their open connections check whether they may stay.
    pub signed_out: watch::Sender<u64>,
}

impl Default for SignIns {
    fn default() -> Self {
        Self {
            codes: Mutex::new(Vec::new()),
            signed_out: watch::channel(0).0,
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl SignIns {
    /// A fresh one-time code for `person` (the caller checked they exist) and when it stops
    /// working (Unix ms). `None` when there are no random numbers.
    pub fn create(&self, person: &str) -> Option<(String, i64)> {
        let mut bytes = [0u8; SIGNIN_CODE_BYTES];
        getrandom::fill(&mut bytes).ok()?;
        let code: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let mut codes = lock(&self.codes);
        codes.retain(|c| c.expires > Instant::now());
        while codes.len() >= SIGNIN_MAX_OPEN {
            codes.remove(0);
        }
        codes.push(Code {
            code: code.clone(),
            person: person.to_owned(),
            expires: Instant::now() + SIGNIN_CODE_LIFETIME,
        });
        let expires = now_ms() + i64::try_from(SIGNIN_CODE_LIFETIME.as_millis()).unwrap_or(0);
        Some((code, expires))
    }

    /// Use a code up: whose it was, if it's open.
    fn take(&self, code: &str) -> Option<String> {
        let mut codes = lock(&self.codes);
        codes.retain(|c| c.expires > Instant::now());
        let at = codes.iter().position(|c| token::matches(&c.code, code))?;
        Some(codes.remove(at).person)
    }

    /// Tell open connections that devices were signed out.
    pub fn bump(&self) {
        self.signed_out.send_modify(|n| *n += 1);
    }
}

/// SHA-256 of a device token, as hex (what the database keeps).
fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A browser's name, tidied.
fn device_name(name: &str) -> String {
    let name: String = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(DEVICE_NAME_MAX_CHARS)
        .collect();
    if name.is_empty() {
        FALLBACK_DEVICE_NAME.to_owned()
    } else {
        name
    }
}

/// A person who may sign in: exists and wasn't removed.
async fn active(shared: &Shared, person: &str) -> Result<Option<Person>, AgentError> {
    let found = shared.hub.state().store.get_person(person).await?;
    Ok(found.filter(|p| p.removed_at.is_none()))
}

/// Trade a sign-in code for a new device token: the token and who it signs in as. `None` when
/// the code isn't open (wrong, used, expired) or its person was removed since.
pub async fn redeem(
    shared: &Shared,
    code: &str,
    name: &str,
) -> Result<Option<(String, Person)>, AgentError> {
    let Some(person) = shared.signins.take(code) else {
        return Ok(None);
    };
    let Some(person) = active(shared, &person).await? else {
        return Ok(None);
    };
    let Ok(device_token) = token::generate() else {
        return Ok(None);
    };
    let device = Device {
        id: new_id(id_prefix::DEVICE),
        user_id: person.id.clone(),
        name: device_name(name),
        created_at: now_ms(),
        last_seen_at: None,
    };
    shared
        .hub
        .state()
        .store
        .add_device(&device, &hash(&device_token))
        .await?;
    tracing::info!(
        person = person.id,
        device = device.id,
        "a browser signed in"
    );
    Ok(Some((device_token, person)))
}

/// Who a local client is from the token it shows: the access token is the owner; a device token
/// is its person (and the device's id). `None` for anything else.
pub async fn local_person(
    shared: &Shared,
    given: &str,
) -> Result<Option<(Person, Option<String>)>, AgentError> {
    let state = shared.hub.state();
    if token::matches(given, &shared.token) {
        return Ok(active(shared, &state.owner_id).await?.map(|p| (p, None)));
    }
    if given.is_empty() {
        return Ok(None);
    }
    let Some(device) = state.store.use_device(&hash(given), now_ms()).await? else {
        return Ok(None);
    };
    Ok(active(shared, &device.user_id)
        .await?
        .map(|p| (p, Some(device.id))))
}

/// Sign every device of a removed person out and close their connections.
pub async fn sign_out_everywhere(shared: &Shared, person: &str) -> Result<(), AgentError> {
    shared
        .hub
        .state()
        .store
        .remove_devices_of(person, now_ms())
        .await?;
    shared.signins.bump();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_work_once() {
        let signins = SignIns::default();
        let (code, expires) = signins.create("pe_1").unwrap();
        assert_eq!(code.len(), SIGNIN_CODE_BYTES * 2);
        assert!(expires > now_ms());
        assert_eq!(signins.take("nope"), None);
        assert_eq!(signins.take(&code).as_deref(), Some("pe_1"));
        assert_eq!(signins.take(&code), None);
    }

    #[test]
    fn only_so_many_codes_wait() {
        let signins = SignIns::default();
        let (first, _) = signins.create("pe_1").unwrap();
        for _ in 0..SIGNIN_MAX_OPEN {
            signins.create("pe_2").unwrap();
        }
        assert_eq!(signins.take(&first), None, "the oldest code went");
    }

    #[test]
    fn device_names_are_tidied() {
        assert_eq!(device_name("  Edge \n on  Windows "), "Edge on Windows");
        assert_eq!(device_name(""), FALLBACK_DEVICE_NAME);
        assert_eq!(
            device_name(&"x".repeat(200)).chars().count(),
            DEVICE_NAME_MAX_CHARS
        );
    }

    #[test]
    fn tokens_are_stored_hashed() {
        let h = hash("secret");
        assert_eq!(h.len(), 64);
        assert_ne!(h, "secret");
        assert_eq!(h, hash("secret"));
    }
}
