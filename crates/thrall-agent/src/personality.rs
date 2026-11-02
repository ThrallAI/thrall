//! Personality: how Thrall talks to each person. Everyone starts on the standard one Thrall ships
//! with; each person (children included) can pick another preset or write their own, like a phone
//! cover, and it only ever changes theirs. A personality is tone only: it's added after the rules,
//! and says it can't change them.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thrall_protocol::PersonalityPreset;

use crate::constants::{
    PERSONALITY_CUSTOM, PERSONALITY_FILE, PERSONALITY_INTRO, PERSONALITY_MAX_CHARS,
    PERSONALITY_PRESETS, PERSONALITY_STANDARD,
};
use crate::state::AppState;

/// A person's choice, kept in their own data folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Personality {
    /// A preset's id, or `custom`.
    pub preset: String,
    /// Their own words, used with `custom`.
    pub custom: String,
}

impl Default for Personality {
    fn default() -> Self {
        Self {
            preset: PERSONALITY_STANDARD.to_owned(),
            custom: String::new(),
        }
    }
}

/// The presets on offer, for the settings screen.
pub fn presets() -> Vec<PersonalityPreset> {
    PERSONALITY_PRESETS
        .iter()
        .map(|(id, name, description, _)| PersonalityPreset {
            id: (*id).to_owned(),
            name: (*name).to_owned(),
            description: (*description).to_owned(),
        })
        .collect()
}

fn file(dir: &Path) -> PathBuf {
    dir.join(PERSONALITY_FILE)
}

/// This person's personality (the standard one when they haven't chosen, or the file is unreadable).
pub fn load(state: &AppState) -> Personality {
    state
        .my_dir()
        .and_then(|d| std::fs::read_to_string(file(&d)).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Tidy a choice: a known preset (else the standard one), and own words capped.
pub fn tidy(preset: &str, custom: &str) -> Personality {
    let preset = preset.trim();
    let known = preset == PERSONALITY_CUSTOM || PERSONALITY_PRESETS.iter().any(|p| p.0 == preset);
    Personality {
        preset: if known { preset } else { PERSONALITY_STANDARD }.to_owned(),
        custom: custom.trim().chars().take(PERSONALITY_MAX_CHARS).collect(),
    }
}

/// Save this person's choice. Err in words when it can't be kept.
pub fn save(state: &AppState, chosen: &Personality) -> Result<(), String> {
    let dir = state
        .my_dir()
        .ok_or("this Thrall has no data folder to keep it in")?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("couldn't save it: {e}"))?;
    let text =
        serde_json::to_string_pretty(chosen).map_err(|e| format!("couldn't save it: {e}"))?;
    std::fs::write(file(&dir), text).map_err(|e| format!("couldn't save it: {e}"))
}

/// What goes into the system prompt: nothing for the standard personality (the rules already set
/// its tone), else the chosen style, framed so it can't override anything.
pub fn prompt(chosen: &Personality) -> Option<String> {
    let style = if chosen.preset == PERSONALITY_CUSTOM {
        chosen.custom.trim()
    } else {
        PERSONALITY_PRESETS
            .iter()
            .find(|p| p.0 == chosen.preset)
            .map(|p| p.3)?
    };
    (!style.is_empty()).then(|| format!("{PERSONALITY_INTRO}\n{style}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_personality_adds_nothing() {
        assert_eq!(prompt(&Personality::default()), None);
    }

    #[test]
    fn a_preset_or_own_words_come_after_the_rule_that_they_change_tone_only() {
        let friendly = prompt(&tidy("friendly", "")).unwrap();
        assert!(friendly.starts_with(PERSONALITY_INTRO));
        let own = prompt(&tidy(PERSONALITY_CUSTOM, "  Talk like a pirate.  ")).unwrap();
        assert!(own.ends_with("Talk like a pirate."));
        assert_eq!(prompt(&tidy(PERSONALITY_CUSTOM, "   ")), None);
    }

    #[test]
    fn unknown_presets_fall_back_and_own_words_are_capped() {
        assert_eq!(tidy("nope", "").preset, PERSONALITY_STANDARD);
        let long = "x".repeat(PERSONALITY_MAX_CHARS * 2);
        assert_eq!(
            tidy(PERSONALITY_CUSTOM, &long).custom.chars().count(),
            PERSONALITY_MAX_CHARS
        );
    }

    #[test]
    fn every_preset_but_standard_has_words() {
        for (id, name, description, text) in PERSONALITY_PRESETS {
            assert!(!name.is_empty() && !description.is_empty(), "{id}");
            assert_eq!(text.is_empty(), *id == PERSONALITY_STANDARD, "{id}");
        }
    }
}
