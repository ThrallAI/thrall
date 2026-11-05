//! Companion PCs connected now, and the calls a session's tools send to its PC.
//!
//! A session whose work runs on a companion PC remembers that PC (`sessions.pc`). While one of its
//! turns runs, its PC tools (`thrall_tools::companion::runs_on_pc`) go to that PC's runner
//! instead of running here. When the PC isn't connected they fail with a plain reason; they never
//! run on this computer instead.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;
use thrall_core::constants::id_prefix;
use thrall_core::ids::new_id;
use thrall_tools::companion::Call;
use tokio::sync::{mpsc, oneshot};

use crate::constants::{PC_DISCONNECTED, PC_NOT_CONNECTED};

/// One call on its way to a PC's runner: the connection serving the runner sends it and answers
/// `reply` (and `output`, for live lines) from what the runner says.
pub struct Outgoing {
    /// The call's id.
    pub id: String,
    /// What to do.
    pub call: Call,
    /// The session's folder, on the PC.
    pub workspace: String,
    /// The session.
    pub session: String,
    /// Whose session it is.
    pub person: String,
    /// The runner's answer, or why it failed.
    pub reply: oneshot::Sender<Result<Value, String>>,
    /// Live output, when the call makes some.
    pub output: Option<mpsc::UnboundedSender<String>>,
}

/// A connected runner.
struct Link {
    /// Tells this link apart from a newer one from the same PC.
    token: u64,
    /// The PC's Thrall folder (where its sessions work unless one picks another).
    home: String,
    /// The connection serving it.
    tx: mpsc::UnboundedSender<Outgoing>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The runners connected now (by PC), and which PC each running session's work goes to.
#[derive(Default)]
pub struct Companions {
    links: Mutex<HashMap<String, Link>>,
    /// Sessions working on a PC, by session id.
    sessions: Mutex<HashMap<String, String>>,
    next: AtomicU64,
}

impl Companions {
    /// A PC's runner connected (`device` is its id); the connection gets calls on `tx`. Returns a
    /// token for `detach`. A newer link from the same PC replaces the older one.
    pub fn attach(&self, device: &str, home: &str, tx: mpsc::UnboundedSender<Outgoing>) -> u64 {
        let token = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        lock(&self.links).insert(
            device.to_owned(),
            Link {
                token,
                home: home.to_owned(),
                tx,
            },
        );
        tracing::info!(device, "a companion PC's runner connected");
        token
    }

    /// That runner's connection closed (only if it's still the PC's newest).
    pub fn detach(&self, device: &str, token: u64) {
        let mut links = lock(&self.links);
        if links.get(device).is_some_and(|l| l.token == token) {
            links.remove(device);
            tracing::info!(device, "a companion PC's runner disconnected");
        }
    }

    /// Whether `device`'s runner is connected.
    pub fn connected(&self, device: &str) -> bool {
        lock(&self.links)
            .get(device)
            .is_some_and(|l| !l.tx.is_closed())
    }

    /// The PC's Thrall folder, while its runner is connected.
    pub fn home(&self, device: &str) -> Option<String> {
        lock(&self.links).get(device).map(|l| l.home.clone())
    }

    /// While a turn of `session` runs: its PC (`None`: it works here).
    pub fn bind(&self, session: &str, pc: Option<&str>) {
        let mut sessions = lock(&self.sessions);
        match pc {
            Some(pc) => {
                sessions.insert(session.to_owned(), pc.to_owned());
            }
            None => {
                sessions.remove(session);
            }
        }
    }

    /// The PC `session`'s tools run on, if any.
    pub fn pc_of(&self, session: &str) -> Option<String> {
        lock(&self.sessions).get(session).cloned()
    }

    /// Send `call` to `device`'s runner and wait for its answer.
    pub async fn call(
        &self,
        device: &str,
        person: &str,
        session: &str,
        workspace: &str,
        call: Call,
        output: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<Value, String> {
        self.call_as(
            &new_id(id_prefix::REQUEST),
            device,
            person,
            session,
            workspace,
            call,
            output,
        )
        .await
    }

    /// [`Companions::call`] with the call's id chosen by the caller (so it can be stopped).
    #[allow(clippy::too_many_arguments)]
    pub async fn call_as(
        &self,
        id: &str,
        device: &str,
        person: &str,
        session: &str,
        workspace: &str,
        call: Call,
        output: Option<mpsc::UnboundedSender<String>>,
    ) -> Result<Value, String> {
        let tx = lock(&self.links)
            .get(device)
            .map(|l| l.tx.clone())
            .ok_or_else(|| PC_NOT_CONNECTED.to_owned())?;
        let (reply, answer) = oneshot::channel();
        tx.send(Outgoing {
            id: id.to_owned(),
            call,
            workspace: workspace.to_owned(),
            session: session.to_owned(),
            person: person.to_owned(),
            reply,
            output,
        })
        .map_err(|_| PC_NOT_CONNECTED.to_owned())?;
        answer.await.map_err(|_| PC_DISCONNECTED.to_owned())?
    }
}
