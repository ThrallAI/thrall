//! Questions, the vibe way. Thrall decides what it sensibly can, carries on, and lists what it
//! assumed at the end. Only what can't be undone, costs money, or could take the work two very
//! different ways is worth stopping for (`[agent] questions`), and then all on one card at the
//! start, each with Thrall's recommended answer picked. Nobody answering in time means those
//! answers are used; Full auto, scheduled jobs and helpers never stop to ask.

use std::time::Duration;

use serde_json::Value;
use thrall_core::config::Questions;
use thrall_core::constants::id_prefix;
use thrall_core::ids::{new_id, now_ms};
use thrall_gates::Mode;
use thrall_protocol::{Event, QuestionItem};
use thrall_tools::tool::arg_str;
use thrall_tools::{Cancel, ToolOutput};

use crate::checks::Track;
use crate::constants::{
    ANSWERED_BY_TIMEOUT, QUESTION_ALREADY_ASKED, QUESTION_MAX_CHARS, QUESTION_MAX_ITEMS,
    QUESTION_MAX_OPTIONS, QUESTIONS_FULL_AUTO, QUESTIONS_NEVER, QUESTIONS_WHEN_IT_MATTERS,
    QUESTIONS_WHEN_UNSURE,
};
use crate::error::AgentError;
use crate::prompts::Reply;
use crate::state::AppState;
use crate::turn::Emitter;

/// What the system prompt says about asking, for this setting and mode.
pub(crate) fn note(questions: Questions, mode: Mode) -> &'static str {
    if mode == Mode::FullAuto {
        return QUESTIONS_FULL_AUTO;
    }
    match questions {
        Questions::Never => QUESTIONS_NEVER,
        Questions::WhenItMatters => QUESTIONS_WHEN_IT_MATTERS,
        Questions::WhenUnsure => QUESTIONS_WHEN_UNSURE,
    }
}

/// The questions in an `ask_user` call, tidied: one to four, each with its recommended answer.
fn parse(args: &Value) -> Result<Vec<QuestionItem>, String> {
    let list = args
        .get("questions")
        .and_then(Value::as_array)
        .ok_or("give `questions`: a list of {text, options?, recommended}")?;
    let clip = |s: &str| {
        s.trim()
            .chars()
            .take(QUESTION_MAX_CHARS)
            .collect::<String>()
    };
    let mut items = Vec::new();
    for q in list.iter().take(QUESTION_MAX_ITEMS) {
        let text = clip(arg_str(q, "text"));
        let recommended = clip(arg_str(q, "recommended"));
        if text.is_empty() || recommended.is_empty() {
            return Err("every question needs `text` and your `recommended` answer".to_owned());
        }
        let mut options: Vec<String> = q
            .get("options")
            .and_then(Value::as_array)
            .map(|o| {
                o.iter()
                    .filter_map(Value::as_str)
                    .map(clip)
                    .filter(|s| !s.is_empty())
                    .take(QUESTION_MAX_OPTIONS)
                    .collect()
            })
            .unwrap_or_default();
        // The recommended answer is always one of the choices.
        if !options.is_empty() && !options.contains(&recommended) {
            options.truncate(QUESTION_MAX_OPTIONS - 1);
            options.insert(0, recommended.clone());
        }
        items.push(QuestionItem {
            text,
            options,
            recommended,
        });
    }
    if items.is_empty() {
        return Err("ask at least one question".to_owned());
    }
    Ok(items)
}

/// One answer per question: what was given, or the recommended answer where nothing was.
fn complete(given: Vec<String>, items: &[QuestionItem]) -> Vec<String> {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            given
                .get(i)
                .map(|a| a.trim().to_owned())
                .filter(|a| !a.is_empty())
                .unwrap_or_else(|| item.recommended.clone())
        })
        .collect()
}

/// The questions and their answers, for the model.
fn listed(items: &[QuestionItem], answers: &[String]) -> String {
    items
        .iter()
        .zip(answers)
        .map(|(q, a)| format!("- {}: {a}", q.text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run an `ask_user` call: show the card and wait, or go straight ahead where nobody is asked.
pub(crate) async fn ask(
    emitter: &Emitter<'_>,
    state: &AppState,
    args: &Value,
    mode: Mode,
    track: &mut Track,
    cancel: &Cancel,
) -> Result<ToolOutput, AgentError> {
    let items = match parse(args) {
        Ok(items) => items,
        Err(why) => return Ok(ToolOutput::error(why)),
    };
    let picks: Vec<String> = items.iter().map(|i| i.recommended.clone()).collect();
    let nobody = mode == Mode::FullAuto
        || state.config.agent.questions == Questions::Never
        || state.is_unattended(emitter.session_id)
        || track.in_helper
        || !emitter.records();
    if nobody {
        return Ok(ToolOutput::ok(
            format!(
                "Nobody is asked here, so go ahead with your recommended answers and list them \
                 under 'Assumptions I made':\n{}",
                listed(&items, &picks)
            ),
            "Went with its own answers",
        ));
    }
    if track.asked {
        return Ok(ToolOutput::error(QUESTION_ALREADY_ASKED));
    }
    track.asked = true;

    let question = new_id(id_prefix::REQUEST);
    let answer = state.prompts.register(&question, &state.user_id);
    let wait = Duration::from_secs(u64::from(state.config.agent.question_wait_minutes.max(1)) * 60);
    let expires_at = now_ms() + i64::try_from(wait.as_millis()).unwrap_or(i64::MAX);
    emitter
        .emit(Event::QuestionAsk {
            question: question.clone(),
            items: items.clone(),
            why: arg_str(args, "why").trim().to_owned(),
            expires_at,
        })
        .await?;
    let mut cancel = cancel.clone();
    let (answers, answered_by, stopped) = tokio::select! {
        answer = answer => match answer {
            Ok(a) => match a.reply {
                Reply::Question(given) => (complete(given, &items), a.answered_by, false),
                _ => (picks.clone(), a.answered_by, false),
            },
            Err(_) => (picks.clone(), ANSWERED_BY_TIMEOUT.to_owned(), false),
        },
        () = tokio::time::sleep(wait) => {
            state.prompts.cancel(&question);
            (picks.clone(), ANSWERED_BY_TIMEOUT.to_owned(), false)
        }
        () = cancel.cancelled() => {
            state.prompts.cancel(&question);
            (picks.clone(), String::new(), true)
        }
    };
    emitter
        .emit(Event::QuestionResolved {
            question,
            answers: answers.clone(),
            answered_by: answered_by.clone(),
        })
        .await?;
    if stopped {
        return Ok(ToolOutput::error(
            "stopped before the questions were answered",
        ));
    }
    if answered_by == ANSWERED_BY_TIMEOUT {
        return Ok(ToolOutput::ok(
            format!(
                "Nobody answered within {} minutes, so go ahead with your recommended answers and \
                 list them under 'Assumptions I made':\n{}",
                wait.as_secs() / 60,
                listed(&items, &answers)
            ),
            "No answer: went with its own picks",
        ));
    }
    Ok(ToolOutput::ok(
        format!(
            "The user answered:\n{}\n\nCarry on with these.",
            listed(&items, &answers)
        ),
        "Answered",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn questions_are_tidied_and_the_pick_is_a_choice() {
        let items = parse(&json!({"questions": [
            {"text": " Which database? ", "options": ["Postgres", "MySQL"], "recommended": "SQLite"},
            {"text": "Name?", "recommended": "thrall"}
        ]}))
        .unwrap();
        assert_eq!(items[0].text, "Which database?");
        assert_eq!(items[0].options, ["SQLite", "Postgres", "MySQL"]);
        assert!(items[1].options.is_empty());
    }

    #[test]
    fn a_question_without_a_recommendation_is_refused() {
        assert!(parse(&json!({"questions": [{"text": "Which?"}]})).is_err());
        assert!(parse(&json!({"questions": []})).is_err());
    }

    #[test]
    fn missing_answers_fall_back_to_the_recommendation() {
        let items = parse(&json!({"questions": [
            {"text": "A?", "recommended": "a"}, {"text": "B?", "recommended": "b"}
        ]}))
        .unwrap();
        assert_eq!(complete(vec!["x".into()], &items), ["x", "b"]);
        assert_eq!(complete(vec![" ".into(), "y".into()], &items), ["a", "y"]);
    }

    #[test]
    fn full_auto_never_asks() {
        assert_eq!(
            note(Questions::WhenUnsure, Mode::FullAuto),
            QUESTIONS_FULL_AUTO
        );
        assert_eq!(note(Questions::Never, Mode::Ask), QUESTIONS_NEVER);
    }
}
