//! The experience log: short lessons Thrall notes when the person corrects it, or when a step
//! failed and it found what works ("on this computer, use py, not python"). Each person has their
//! own, in their data folder; the newest are added to every turn's instructions, and the person
//! sees and deletes them in Settings → Learning.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thrall_core::constants::id_prefix;
use thrall_core::ids::{new_id, now_ms};
use thrall_protocol::LessonInfo;

use crate::constants::{
    EXPERIENCE_FILE, EXPERIENCE_IN_PROMPT, EXPERIENCE_INTRO, EXPERIENCE_KEPT, LESSON_MAX_CHARS,
};
use crate::state::AppState;

/// One lesson.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lesson {
    /// Its id (`experience.forget {lesson}`).
    pub id: String,
    /// The lesson, one line.
    pub text: String,
    /// When it was noted, Unix ms.
    pub at: i64,
}

fn file(state: &AppState) -> Option<PathBuf> {
    state.my_dir().map(|d| d.join(EXPERIENCE_FILE))
}

/// This person's lessons, oldest first.
pub fn list(state: &AppState) -> Vec<Lesson> {
    file(state)
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save(state: &AppState, lessons: &[Lesson]) -> Result<(), String> {
    let path = file(state).ok_or("this Thrall has no data folder to keep lessons in")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("couldn't keep the lesson: {e}"))?;
    }
    let text = serde_json::to_string_pretty(lessons).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("couldn't keep the lesson: {e}"))
}

/// A lesson tidied to one line and capped; empty when there's nothing to keep.
pub fn tidy(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(LESSON_MAX_CHARS)
        .collect()
}

/// Note a lesson (the same one again is kept once, as the newest). Only the newest
/// [`EXPERIENCE_KEPT`] are kept.
pub fn note(state: &AppState, text: &str) -> Result<Lesson, String> {
    let text = tidy(text);
    if text.is_empty() {
        return Err("the lesson is empty: say in one line what to do differently".to_owned());
    }
    let mut lessons = list(state);
    lessons.retain(|l| !l.text.eq_ignore_ascii_case(&text));
    let lesson = Lesson {
        id: new_id(id_prefix::LESSON),
        text,
        at: now_ms(),
    };
    lessons.push(lesson.clone());
    let extra = lessons.len().saturating_sub(EXPERIENCE_KEPT);
    lessons.drain(..extra);
    save(state, &lessons)?;
    Ok(lesson)
}

/// Delete a lesson. False if there's no such lesson.
pub fn forget(state: &AppState, id: &str) -> Result<bool, String> {
    let mut lessons = list(state);
    let before = lessons.len();
    lessons.retain(|l| l.id != id);
    if lessons.len() == before {
        return Ok(false);
    }
    save(state, &lessons)?;
    Ok(true)
}

/// The lessons for the settings screen, newest first.
pub fn listed(state: &AppState) -> Vec<LessonInfo> {
    list(state)
        .into_iter()
        .rev()
        .map(|l| LessonInfo {
            lesson: l.id,
            text: l.text,
            at: l.at,
        })
        .collect()
}

/// The newest lessons, for the system prompt.
pub fn prompt(lessons: &[Lesson]) -> Option<String> {
    if lessons.is_empty() {
        return None;
    }
    let from = lessons.len().saturating_sub(EXPERIENCE_IN_PROMPT);
    let lines: Vec<String> = lessons[from..]
        .iter()
        .map(|l| format!("- {}", l.text))
        .collect();
    Some(format!("{EXPERIENCE_INTRO}\n{}", lines.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lesson(n: usize) -> Lesson {
        Lesson {
            id: format!("ls_{n}"),
            text: format!("lesson {n}"),
            at: 0,
        }
    }

    #[test]
    fn lessons_are_one_line_and_capped() {
        assert_eq!(tidy("  use py\n  not python  "), "use py not python");
        assert_eq!(
            tidy(&"x".repeat(LESSON_MAX_CHARS * 2)).chars().count(),
            LESSON_MAX_CHARS
        );
        assert_eq!(tidy(" \n "), "");
    }

    #[test]
    fn only_the_newest_lessons_go_into_the_prompt() {
        assert_eq!(prompt(&[]), None);
        let many: Vec<Lesson> = (0..EXPERIENCE_IN_PROMPT + 5).map(lesson).collect();
        let text = prompt(&many).unwrap();
        assert!(text.starts_with(EXPERIENCE_INTRO));
        assert!(!text.contains("- lesson 4\n"), "the oldest are left out");
        assert!(text.ends_with(&format!("- lesson {}", EXPERIENCE_IN_PROMPT + 4)));
    }
}
