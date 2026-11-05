//! The people who use this Thrall, for the owner's people requests: list, add, rename, remove,
//! and each person's terms (own key, model, monthly limit) and own key. Removing someone never
//! deletes what they made; it stops them connecting, forgets their phones and signs their
//! browsers out.

use thrall_agent::AgentError;
use thrall_core::constants::{PERSON_NAME_MAX_CHARS, id_prefix, person_role};
use thrall_core::ids::{new_id, now_ms};
use thrall_protocol::{PersonInfo, PersonRules, Reply};
use thrall_store::PersonTerms;

use crate::connection::Shared;
use crate::constants::{CHILD_BANDS, MICROS_PER_DOLLAR, MODEL_ID_MAX_CHARS, MONTHLY_LIMIT_MAX_USD};

/// Why a people request was refused (shown to the owner as is).
pub type Refusal = &'static str;

/// Everyone, the owner first, with what each has spent since `since` (all of it when absent).
pub async fn listed(shared: &Shared, since: Option<i64>) -> Result<Reply, AgentError> {
    let state = shared.hub.state();
    let mut people = state.store.list_people().await?;
    people.sort_by_key(|p| p.role != person_role::OWNER);
    let mut items = Vec::with_capacity(people.len());
    for p in people {
        let key_set = p.role != person_role::OWNER && {
            let keys = state.own_keys.clone();
            let person = p.id.clone();
            tokio::task::spawn_blocking(move || keys.has(&person))
                .await
                .unwrap_or(false)
        };
        items.push(PersonInfo {
            phones: shared.remote.phone_count(&p.id, &state.owner_id),
            spent: state.store.spent_since(&p.id, since.unwrap_or(0)).await?,
            own_key: p.own_key,
            key_set,
            model: p.model,
            monthly_limit: p.monthly_limit_micros.map(|m| m as f64 / MICROS_PER_DOLLAR),
            rules: serde_json::from_str(&p.rules).unwrap_or_default(),
            person: p.id,
            name: p.name,
            role: p.role,
            added: p.created_at,
        });
    }
    Ok(Reply::PeopleListed { items })
}

/// A name as given, tidied; `Err` when it can't be used.
fn clean_name(name: &str) -> Result<String, Refusal> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err("A person needs a name.");
    }
    if name.chars().count() > PERSON_NAME_MAX_CHARS {
        return Err("That name is too long.");
    }
    if name.chars().any(char::is_control) {
        return Err("That name has characters it can't have.");
    }
    Ok(name)
}

/// Add a member.
pub async fn add(shared: &Shared, name: &str) -> Result<Result<Reply, Refusal>, AgentError> {
    let name = match clean_name(name) {
        Ok(n) => n,
        Err(why) => return Ok(Err(why)),
    };
    let store = &shared.hub.state().store;
    let id = new_id(id_prefix::PERSON);
    store.ensure_user(&id, &name, now_ms()).await?;
    store.set_role(&id, person_role::MEMBER).await?;
    tracing::info!(person = id, "person added");
    Ok(Ok(listed(shared, None).await?))
}

/// Rename someone.
pub async fn rename(
    shared: &Shared,
    person: &str,
    name: &str,
) -> Result<Result<Reply, Refusal>, AgentError> {
    let name = match clean_name(name) {
        Ok(n) => n,
        Err(why) => return Ok(Err(why)),
    };
    if !shared
        .hub
        .state()
        .store
        .rename_person(person, &name)
        .await?
    {
        return Ok(Err("There's no such person."));
    }
    Ok(Ok(listed(shared, None).await?))
}

/// Remove someone (never the owner): they can't connect and their phones are forgotten.
pub async fn remove(shared: &Shared, person: &str) -> Result<Result<Reply, Refusal>, AgentError> {
    let state = shared.hub.state();
    match state.store.get_person(person).await? {
        Some(p) if p.removed_at.is_none() && p.role == person_role::OWNER => {
            return Ok(Err("The person who runs this Thrall can't be removed."));
        }
        Some(p) if p.removed_at.is_none() => {}
        _ => return Ok(Err("There's no such person.")),
    }
    state.store.remove_person(person, now_ms()).await?;
    shared.remote.forget_person(person);
    crate::signin::sign_out_everywhere(shared, person).await?;
    tracing::info!(person, "person removed");
    Ok(Ok(listed(shared, None).await?))
}

/// Set what someone (never the owner) is allowed. Takes effect from their next step; their phones
/// are disconnected straight away when phones are turned off.
pub async fn set_rules(
    shared: &Shared,
    person: &str,
    rules: PersonRules,
) -> Result<Result<Reply, Refusal>, AgentError> {
    let state = shared.hub.state();
    match state.store.get_person(person).await? {
        Some(p) if p.removed_at.is_none() && p.role == person_role::OWNER => {
            return Ok(Err(
                "The person who runs this Thrall is always allowed everything.",
            ));
        }
        Some(p) if p.removed_at.is_none() => {}
        _ => return Ok(Err("There's no such person.")),
    }
    if let Some(band) = &rules.child
        && !CHILD_BANDS.contains(&band.as_str())
    {
        return Ok(Err("A child's age is under 13, 13 to 15, or 16 to 17."));
    }
    let text = serde_json::to_string(&rules).unwrap_or_else(|_| "{}".to_owned());
    state.store.set_person_rules(person, &text).await?;
    state.companions.set_rules(person, rules.clone());
    shared.remote.phones_allowed(person, !rules.no_phones);
    tracing::info!(person, "a person's controls changed");
    Ok(Ok(listed(shared, None).await?))
}

/// A model id as given: trimmed, `None` when empty; `Err` when it can't be one.
fn clean_model(model: Option<&str>) -> Result<Option<String>, Refusal> {
    let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) else {
        return Ok(None);
    };
    if model.chars().count() > MODEL_ID_MAX_CHARS
        || model.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err("That isn't a model id (like openai/gpt-5-mini).");
    }
    Ok(Some(model.to_owned()))
}

/// A monthly limit in dollars as millionths of a dollar; `Err` when it can't be one.
fn clean_limit(limit: Option<f64>) -> Result<Option<i64>, Refusal> {
    match limit {
        None => Ok(None),
        Some(usd) if usd.is_finite() && usd > 0.0 && usd <= MONTHLY_LIMIT_MAX_USD => {
            Ok(Some((usd * MICROS_PER_DOLLAR).round() as i64))
        }
        Some(_) => Err("A monthly limit is an amount of dollars above zero."),
    }
}

/// Set what someone's turns run on and may spend. The owner always uses the engine's key and
/// model (changed in Keys and Models); their limit, if they set one, guards against runaway
/// spending.
pub async fn set_terms(
    shared: &Shared,
    person: &str,
    own_key: bool,
    model: Option<&str>,
    monthly_limit: Option<f64>,
) -> Result<Result<Reply, Refusal>, AgentError> {
    let state = shared.hub.state();
    let owner = match state.store.get_person(person).await? {
        Some(p) if p.removed_at.is_none() => p.role == person_role::OWNER,
        _ => return Ok(Err("There's no such person.")),
    };
    let model = match clean_model(model) {
        Ok(m) => m,
        Err(why) => return Ok(Err(why)),
    };
    let monthly_limit_micros = match clean_limit(monthly_limit) {
        Ok(l) => l,
        Err(why) => return Ok(Err(why)),
    };
    if owner && own_key {
        return Ok(Err(
            "You use this Thrall's own key; change it with `thrall key set`.",
        ));
    }
    if owner && model.is_some() {
        return Ok(Err("Change your model in Settings → Models."));
    }
    let terms = PersonTerms {
        own_key,
        model,
        monthly_limit_micros,
    };
    if !state.store.set_person_terms(person, &terms).await? {
        return Ok(Err("There's no such person."));
    }
    state.set_terms(person, terms);
    tracing::info!(person, own_key, "person's terms set");
    Ok(Ok(listed(shared, None).await?))
}

/// Keep (or, with an empty key, forget) a person's own AI key. The caller checked the asker may.
pub async fn set_key(
    shared: &Shared,
    person: &str,
    key: &str,
) -> Result<Result<(), String>, AgentError> {
    let state = shared.hub.state();
    match state.store.get_person(person).await? {
        Some(p) if p.removed_at.is_none() && p.role == person_role::OWNER => {
            return Ok(Err(
                "Your key is this Thrall's own; set it with `thrall key set`.".to_owned(),
            ));
        }
        Some(p) if p.removed_at.is_none() => {}
        _ => return Ok(Err("There's no such person.".to_owned())),
    }
    let key = thrall_core::secrets::clean_pasted(key);
    let keys = state.own_keys.clone();
    let who = person.to_owned();
    let done = tokio::task::spawn_blocking(move || {
        if key.is_empty() {
            keys.clear(&who)
        } else {
            keys.set(&who, &key)
        }
    })
    .await
    .unwrap_or_else(|_| Err("Couldn't keep the key.".to_owned()));
    if done.is_ok() {
        tracing::info!(person, "person's own key changed");
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_tidied_and_checked() {
        assert_eq!(clean_name("  Sam   Smith ").as_deref(), Ok("Sam Smith"));
        assert!(clean_name("   ").is_err());
        assert!(clean_name(&"a".repeat(PERSON_NAME_MAX_CHARS + 1)).is_err());
        assert!(clean_name(&"a".repeat(PERSON_NAME_MAX_CHARS)).is_ok());
    }

    #[test]
    fn models_and_limits_are_checked() {
        assert_eq!(clean_model(Some("  ")), Ok(None));
        assert_eq!(
            clean_model(Some(" openai/gpt-5-mini ")),
            Ok(Some("openai/gpt-5-mini".to_owned()))
        );
        assert!(clean_model(Some("two words")).is_err());
        assert_eq!(clean_limit(None), Ok(None));
        assert_eq!(clean_limit(Some(5.0)), Ok(Some(5_000_000)));
        assert!(clean_limit(Some(0.0)).is_err());
        assert!(clean_limit(Some(f64::NAN)).is_err());
        assert!(clean_limit(Some(MONTHLY_LIMIT_MAX_USD + 1.0)).is_err());
    }
}
