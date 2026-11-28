//! Unleashed: the owner's switch that stops their chats asking, even about high-risk actions,
//! except wiping or formatting a disk (thrall_gates::unleashed_gate). Off by default. Turning it
//! on needs the person to type `UNLEASH`; turning it off (Chain him, `/chain`) never does, and
//! holds at once, even mid-turn.

use thrall_core::constants::CONFIG_FILE;
use thrall_core::ids::now_ms;
use thrall_protocol::{Event, EventFrame};

use crate::constants::{UNLEASH_CONFIRM, UNLEASH_NEEDS_CONFIRM, UNLEASH_OWNER_ONLY};
use crate::state::AppState;

/// Turn Unleashed on or off for the engine: saved in the config, used at once, and told to every
/// client of the owner (`safety.changed`). `confirm` must be `UNLEASH` to turn it on.
pub fn set(state: &AppState, on: bool, confirm: &str) -> Result<(), String> {
    if state.user_id != state.owner_id {
        return Err(UNLEASH_OWNER_ONLY.to_owned());
    }
    if on && confirm.trim() != UNLEASH_CONFIRM {
        return Err(UNLEASH_NEEDS_CONFIRM.to_owned());
    }
    // Off holds at once, even if the config can't be saved (it's said, and asked again later).
    if !on {
        state.set_unleashed(false);
    }
    if let Some(dir) = &state.data_dir {
        thrall_core::config::set_unleashed(&dir.join(CONFIG_FILE), on)
            .map_err(|e| format!("Couldn't save it: {e}"))?;
    }
    state.set_unleashed(on);
    tracing::warn!(unleashed = on, "Unleashed switched");
    let _ = state.notices.send(EventFrame::new(
        "",
        None,
        now_ms(),
        Event::SafetyChanged { unleashed: on },
    ));
    Ok(())
}
