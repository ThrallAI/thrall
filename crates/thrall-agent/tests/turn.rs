//! Whole turns against a fake provider and an in-memory store.
// Integration tests are their own crate, so the in-tests unwrap exemption doesn't reach helpers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use futures::{StreamExt, stream};
use thrall_agent::{AppState, Reply, commands, session, turn};
use thrall_core::Config;
use thrall_llm::{
    ChatRequest, ChatRole, LlmError, LlmProvider, ReplyStream, StreamEvent, TokenUsage,
    ToolCallDelta,
};
use thrall_protocol::{
    ErrorCode, Event, EventFrame, PermissionChoice, Role, StepStatus, TurnStatus,
};
use thrall_store::{SqliteStore, Store};
use tokio::sync::mpsc;

type Script = Vec<Result<StreamEvent, LlmError>>;

/// Plays one scripted reply per call (the last one repeats) and remembers every request.
struct FakeProvider {
    scripts: Mutex<VecDeque<Script>>,
    seen: Mutex<Vec<ChatRequest>>,
    /// Replies for helpers (requests whose system prompt is the helper's), played in order.
    helper: Mutex<VecDeque<Script>>,
    /// What `accepts_images` answers.
    sees: Mutex<Option<bool>>,
    /// Pictures made (`make_image` calls).
    made: Mutex<Vec<thrall_llm::ImageRequest>>,
    /// What `credit_left` answers.
    credit: Mutex<Option<f64>>,
}

impl LlmProvider for FakeProvider {
    fn name(&self) -> &str {
        "Fake"
    }

    fn credit_left(&self) -> futures::future::BoxFuture<'_, Option<f64>> {
        let credit = *self.credit.lock().unwrap();
        Box::pin(async move { credit })
    }

    fn accepts_images<'a>(
        &'a self,
        _model: &'a str,
    ) -> futures::future::BoxFuture<'a, Option<bool>> {
        let sees = *self.sees.lock().unwrap();
        Box::pin(async move { sees })
    }

    fn prices<'a>(
        &'a self,
        _model: &'a str,
    ) -> futures::future::BoxFuture<'a, Option<thrall_llm::ModelPrices>> {
        Box::pin(async {
            Some(thrall_llm::ModelPrices {
                prompt: 0.000_000_5,
                completion: 0.000_003,
                image_output: Some(0.000_06),
            })
        })
    }

    fn speak(
        &self,
        request: thrall_llm::SpeechRequest,
    ) -> futures::future::BoxFuture<'_, Result<thrall_llm::SpeechReply, LlmError>> {
        let voice = request.voice;
        Box::pin(async move {
            assert!(!voice.is_empty());
            // One second of silence at 24 kHz.
            let mut wav = b"RIFF".to_vec();
            wav.resize(44 + 48_000, 0);
            Ok(thrall_llm::SpeechReply {
                wav,
                usage: TokenUsage {
                    input_tokens: 30,
                    cached_tokens: 0,
                    output_tokens: 50,
                    cost_usd: Some(0.0002),
                },
            })
        })
    }

    fn make_image(
        &self,
        request: thrall_llm::ImageRequest,
    ) -> futures::future::BoxFuture<'_, Result<thrall_llm::ImageReply, LlmError>> {
        self.made.lock().unwrap().push(request);
        Box::pin(async {
            Ok(thrall_llm::ImageReply {
                images: vec![thrall_llm::ImagePart {
                    mime: "image/png".into(),
                    // A real (1x1) PNG.
                    base64: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==".into(),
                }],
                text: String::new(),
                usage: TokenUsage {
                    input_tokens: 20,
                    cached_tokens: 0,
                    output_tokens: 1290,
                    cost_usd: Some(0.0774),
                },
            })
        })
    }

    fn stream(&self, request: ChatRequest) -> ReplyStream {
        let for_helper = request
            .messages
            .first()
            .is_some_and(|m| m.content.starts_with("You are a helper"));
        // The picture model answers on its own (the title call runs alongside and takes scripts).
        let for_vision = request.model == PICTURE_MODEL;
        let model = request.model.clone();
        self.seen.lock().unwrap().push(request);
        if for_vision {
            return stream::iter(reply(&[PICTURE_DESCRIPTION])).boxed();
        }
        if model.starts_with("busy/") {
            return stream::iter(vec![Err(LlmError::RateLimited {
                provider: "OpenRouter".into(),
                retry_after_secs: None,
            })])
            .boxed();
        }
        if for_helper {
            let mut h = self.helper.lock().unwrap();
            if let Some(script) = h.pop_front() {
                return stream::iter(script).boxed();
            }
        }
        let mut scripts = self.scripts.lock().unwrap();
        let script = if scripts.len() > 1 {
            scripts.pop_front().unwrap()
        } else {
            scripts.front().cloned().unwrap_or_default()
        };
        stream::iter(script).boxed()
    }
}

fn usage() -> Result<StreamEvent, LlmError> {
    Ok(StreamEvent::Usage(TokenUsage {
        input_tokens: 10,
        cached_tokens: 0,
        output_tokens: 4,
        cost_usd: Some(0.002),
    }))
}

fn reply(parts: &[&str]) -> Script {
    let mut script: Script = parts
        .iter()
        .map(|p| Ok(StreamEvent::Text((*p).to_owned())))
        .collect();
    script.push(Ok(StreamEvent::Finished(Some("stop".into()))));
    script.push(usage());
    script
}

fn tool_call(id: &str, name: &str, arguments: &str) -> Script {
    vec![
        Ok(StreamEvent::ToolCall(ToolCallDelta {
            index: 0,
            id: Some(id.into()),
            name: Some(name.into()),
            arguments: arguments.into(),
        })),
        Ok(StreamEvent::Finished(Some("tool_calls".into()))),
        usage(),
    ]
}

async fn setup(scripts: Vec<Script>) -> (AppState, Arc<FakeProvider>) {
    let store = SqliteStore::open_in_memory().await.unwrap();
    store.ensure_user("u_test", "Test", 0).await.unwrap();
    let provider = Arc::new(FakeProvider {
        scripts: Mutex::new(scripts.into()),
        seen: Mutex::new(Vec::new()),
        helper: Mutex::new(VecDeque::new()),
        sees: Mutex::new(None),
        made: Mutex::new(Vec::new()),
        credit: Mutex::new(None),
    });
    let state = AppState::new(
        Config::default(),
        Arc::new(store),
        provider.clone(),
        "u_test".into(),
    )
    .unwrap();
    (state, provider)
}

fn workspace(name: &str) -> String {
    let dir = std::env::temp_dir().join(format!("thrall-agent-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("hello.txt"), "hi\n").unwrap();
    dir.to_string_lossy().into_owned()
}

/// Run a turn, answering any permission request with `answer`.
async fn run_answering(
    state: &AppState,
    session_id: &str,
    text: &str,
    answer: PermissionChoice,
) -> (turn::TurnSummary, Vec<EventFrame>) {
    let (tx, mut rx) = mpsc::channel::<EventFrame>(1024);
    let collect = async {
        let mut frames = Vec::new();
        while let Some(frame) = rx.recv().await {
            if let Event::PermissionRequest { request, .. } = &frame.event {
                assert!(state.prompts.answer(
                    request,
                    &state.user_id,
                    Reply::Permission(answer),
                    "test"
                ));
            }
            frames.push(frame);
        }
        frames
    };
    let go = async move {
        let summary = turn::run(state, session_id, text, &tx).await.unwrap();
        drop(tx);
        summary
    };
    tokio::join!(go, collect)
}

async fn run(
    state: &AppState,
    session_id: &str,
    text: &str,
) -> (turn::TurnSummary, Vec<EventFrame>) {
    run_answering(state, session_id, text, PermissionChoice::No).await
}

fn kinds(frames: &[EventFrame]) -> Vec<&'static str> {
    frames
        .iter()
        .filter(|f| f.seq.is_some())
        .map(|f| f.event.type_name())
        .collect()
}

#[tokio::test]
async fn a_turn_streams_records_and_gets_a_model_title() {
    let (state, _) = setup(vec![
        reply(&["Hel", "lo", "!"]),
        reply(&["\"Saying hello.\""]),
    ])
    .await;
    let s = session::create(&state, None).await.unwrap();
    let (summary, frames) = run(&state, &s.id, "say hello").await;

    assert_eq!(summary.status, TurnStatus::Done);
    assert_eq!(summary.reply, "Hello!");
    assert_eq!(summary.usage.output_tokens, 4);

    // Streamed text adds up to the reply, and deltas carry no seq.
    let streamed: String = frames
        .iter()
        .filter_map(|f| match &f.event {
            Event::MessageDelta { text, .. } => {
                assert!(f.seq.is_none());
                Some(text.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(streamed, "Hello!");

    // Recorded events have gap-free seqs and the expected shape.
    let seqs: Vec<u64> = frames.iter().filter_map(|f| f.seq).collect();
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>());
    assert_eq!(
        kinds(&frames),
        vec![
            "turn.started",
            "message.started",
            "message.completed",
            "message.started",
            "message.completed",
            "turn.completed",
            "session.updated"
        ]
    );
    assert!(frames.iter().any(|f| matches!(
        f.event,
        Event::MessageStarted {
            role: Role::User,
            ..
        }
    )));

    // The log replays exactly what was sent.
    let stored = state
        .store
        .events_after("u_test", &s.id, 0, 100)
        .await
        .unwrap();
    assert_eq!(stored.len(), seqs.len());

    // The model's title replaced the placeholder; context and cost were saved.
    let saved = session::get(&state, &s.id).await.unwrap();
    assert_eq!(saved.title, "Saying hello");
    let context = state.store.context_messages("u_test", &s.id).await.unwrap();
    assert_eq!(context.len(), 2);
    let totals = state
        .store
        .usage_totals("u_test", Some(&s.id))
        .await
        .unwrap();
    assert_eq!(totals.input_tokens, 20, "turn call + title call");
}

#[tokio::test]
async fn the_second_turn_sees_the_first() {
    let (state, provider) = setup(vec![reply(&["ok"]), reply(&["A title"]), reply(&["ok"])]).await;
    let s = session::create(&state, None).await.unwrap();
    run(&state, &s.id, "first").await;
    run(&state, &s.id, "second").await;

    let seen = provider.seen.lock().unwrap();
    assert_eq!(seen.len(), 3, "no second title call");
    let last = seen.last().unwrap();
    let roles: Vec<ChatRole> = last.messages.iter().map(|m| m.role).collect();
    assert_eq!(
        roles,
        vec![
            ChatRole::System,
            ChatRole::User,
            ChatRole::Assistant,
            ChatRole::User
        ]
    );
    assert_eq!(last.messages[1].content, "first");
    assert_eq!(last.messages[3].content, "second");
    assert!(!last.tools.is_empty(), "tools are offered");
}

#[tokio::test]
async fn tools_run_and_their_results_go_back_to_the_model() {
    let (state, provider) = setup(vec![
        tool_call("c1", "list_dir", "{}"),
        reply(&["There's one file."]),
        reply(&["Listing files"]),
    ])
    .await;
    let s = session::create(&state, Some(workspace("tools")))
        .await
        .unwrap();
    let (summary, frames) = run(&state, &s.id, "what's here?").await;

    assert_eq!(summary.status, TurnStatus::Done);
    assert_eq!(summary.reply, "There's one file.");
    let step = frames.iter().find_map(|f| match &f.event {
        Event::StepCompleted {
            status, summary, ..
        } => Some((*status, summary.clone())),
        _ => None,
    });
    assert_eq!(step, Some((StepStatus::Ok, "1 entries".into())));

    // The second call carried the tool call and its result.
    let second = provider.seen.lock().unwrap()[1].clone();
    let roles: Vec<ChatRole> = second.messages.iter().map(|m| m.role).collect();
    assert_eq!(
        roles,
        vec![
            ChatRole::System,
            ChatRole::User,
            ChatRole::Assistant,
            ChatRole::Tool
        ]
    );
    assert_eq!(second.messages[2].tool_calls[0].name, "list_dir");
    assert!(second.messages[3].content.contains("hello.txt"));

    // Context keeps the whole exchange for the next turn.
    let context = state.store.context_messages("u_test", &s.id).await.unwrap();
    let stored_roles: Vec<&str> = context.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(stored_roles, vec!["user", "assistant", "tool", "assistant"]);
}

#[tokio::test]
async fn shell_asks_and_a_no_stops_it() {
    let marker = std::env::temp_dir().join(format!("thrall-denied-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let command = format!(
        "{{\"command\": \"echo x > '{}'\"}}",
        marker.to_string_lossy().replace('\\', "/")
    );
    let (state, _) = setup(vec![
        tool_call("c1", "shell", &command),
        reply(&["Okay, I won't."]),
        reply(&["Declined command"]),
    ])
    .await;
    let s = session::create(&state, Some(workspace("deny")))
        .await
        .unwrap();
    let (summary, frames) = run_answering(&state, &s.id, "make a file", PermissionChoice::No).await;

    assert_eq!(summary.status, TurnStatus::Done);
    let k = kinds(&frames);
    let at = |name: &str| k.iter().position(|x| *x == name).unwrap();
    assert!(at("step.started") < at("permission.request"));
    assert!(at("permission.request") < at("permission.resolved"));
    assert!(at("permission.resolved") < at("step.completed"));
    assert!(frames.iter().any(|f| matches!(
        f.event,
        Event::StepCompleted {
            status: StepStatus::Denied,
            ..
        }
    )));
    assert!(!marker.exists(), "a denied command must not run");
}

#[tokio::test]
async fn shell_runs_after_a_yes() {
    let (state, _) = setup(vec![
        tool_call("c1", "shell", r#"{"command": "echo approved-run"}"#),
        reply(&["Done."]),
        reply(&["Echo test"]),
    ])
    .await;
    let s = session::create(&state, Some(workspace("allow")))
        .await
        .unwrap();
    let (_, frames) = run_answering(&state, &s.id, "echo", PermissionChoice::Yes).await;
    let output: String = frames
        .iter()
        .filter_map(|f| match &f.event {
            Event::StepOutput { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(output.contains("approved-run"), "live output: {output}");
}

#[tokio::test]
async fn unknown_tools_fail_the_step_not_the_turn() {
    let (state, _) = setup(vec![
        tool_call("c1", "teleport", "{}"),
        reply(&["Sorry."]),
        reply(&["Teleporting"]),
    ])
    .await;
    let s = session::create(&state, None).await.unwrap();
    let (summary, frames) = run(&state, &s.id, "teleport me").await;
    assert_eq!(summary.status, TurnStatus::Done);
    assert!(frames.iter().any(|f| matches!(
        f.event,
        Event::StepCompleted {
            status: StepStatus::Error,
            ..
        }
    )));
}

#[tokio::test]
async fn a_provider_failure_ends_the_turn_cleanly() {
    let mut script = reply(&["Partial"]);
    script.truncate(1);
    script.push(Err(LlmError::Network {
        provider: "Fake".into(),
        detail: "connection reset".into(),
    }));
    let (state, _) = setup(vec![script]).await;
    let s = session::create(&state, None).await.unwrap();
    let (summary, frames) = run(&state, &s.id, "hi").await;

    assert_eq!(summary.status, TurnStatus::Failed);
    assert_eq!(summary.reply, "Partial");
    let error = frames.iter().find_map(|f| match &f.event {
        Event::Error { code, retry, .. } => Some((*code, *retry)),
        _ => None,
    });
    assert_eq!(error, Some((ErrorCode::ProviderUnavailable, true)));
    assert!(matches!(
        frames.last().map(|f| &f.event),
        Some(Event::TurnCompleted {
            status: TurnStatus::Failed,
            ..
        })
    ));
    // The failed reply is kept out of the model's context, and the placeholder title stays.
    let context = state.store.context_messages("u_test", &s.id).await.unwrap();
    assert_eq!(context.len(), 1);
    assert_eq!(session::get(&state, &s.id).await.unwrap().title, "hi");
}

#[tokio::test]
async fn unknown_session_is_a_clear_error() {
    let (state, _) = setup(vec![reply(&["x"])]).await;
    let (tx, _rx) = mpsc::channel(8);
    let err = turn::run(&state, "s_missing", "hi", &tx).await.unwrap_err();
    assert!(err.to_string().contains("thrall sessions"));
}

#[tokio::test]
async fn commands_answer_in_markdown() {
    let (state, _) = setup(vec![reply(&["x"])]).await;
    let s = session::create(&state, None).await.unwrap();
    let help = commands::run(&state, &s.id, "help", "").await.unwrap();
    assert!(help.contains("/cost"));
    commands::run(&state, &s.id, "model", "test/other")
        .await
        .unwrap();
    assert_eq!(state.model(), "test/other");
    commands::run(&state, &s.id, "title", "Renamed")
        .await
        .unwrap();
    assert_eq!(session::get(&state, &s.id).await.unwrap().title, "Renamed");
    let err = commands::run(&state, &s.id, "nope", "").await.unwrap_err();
    assert!(err.to_string().contains("/help"));
}

// ---------- modes, gates, plan mode, interrupts, checks ----------

/// Run a turn, answering prompts with `reply`, and calling `during` on each event (for
/// interrupting or typing mid-turn).
async fn run_with(
    state: &AppState,
    session_id: &str,
    text: &str,
    reply: Reply,
    during: impl Fn(&AppState, &Event),
) -> (turn::TurnSummary, Vec<EventFrame>) {
    let (tx, mut rx) = mpsc::channel::<EventFrame>(1024);
    let collect = async {
        let mut frames = Vec::new();
        while let Some(frame) = rx.recv().await {
            match &frame.event {
                Event::PermissionRequest { request, .. }
                | Event::PlanProposed { plan: request, .. }
                | Event::QuestionAsk {
                    question: request, ..
                } => {
                    state
                        .prompts
                        .answer(request, &state.user_id, reply.clone(), "test");
                }
                _ => {}
            }
            during(state, &frame.event);
            frames.push(frame);
        }
        frames
    };
    let go = async move {
        let summary = turn::run(state, session_id, text, &tx).await.unwrap();
        drop(tx);
        summary
    };
    tokio::join!(go, collect)
}

fn yes() -> Reply {
    Reply::Permission(PermissionChoice::Yes)
}

fn checks_off(state: &mut AppState) {
    state.config.agent.verify = false;
    state.config.agent.claim_check = false;
}

#[tokio::test]
async fn edits_ask_with_a_diff_and_always_covers_the_next_one() {
    let (mut state, _) = setup(vec![
        tool_call("c1", "read_file", r#"{"path": "hello.txt"}"#),
        tool_call(
            "c2",
            "edit_file",
            r#"{"path": "hello.txt", "old_string": "hi", "new_string": "hey"}"#,
        ),
        tool_call(
            "c3",
            "write_file",
            r#"{"path": "second.txt", "content": "two"}"#,
        ),
        reply(&["Done."]),
        reply(&["Editing files"]),
    ])
    .await;
    checks_off(&mut state);
    let ws = workspace("always");
    let s = session::create(&state, Some(ws.clone())).await.unwrap();
    let (_, frames) = run_with(
        &state,
        &s.id,
        "edit it",
        Reply::Permission(PermissionChoice::Always),
        |_, _| {},
    )
    .await;

    let requests: Vec<_> = frames
        .iter()
        .filter_map(|f| match &f.event {
            Event::PermissionRequest { diff, choices, .. } => Some((diff.clone(), choices.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(requests.len(), 1, "the second edit is covered by always");
    let (diff, choices) = &requests[0];
    assert!(diff.as_ref().unwrap().unified.contains("+hey"));
    assert!(choices.contains(&PermissionChoice::Always));
    assert_eq!(
        std::fs::read_to_string(format!("{ws}/hello.txt")).unwrap(),
        "hey\n"
    );
    assert!(std::path::Path::new(&format!("{ws}/second.txt")).exists());
}

#[tokio::test]
async fn edits_need_a_read_first() {
    let (mut state, _) = setup(vec![
        tool_call(
            "c1",
            "edit_file",
            r#"{"path": "hello.txt", "old_string": "hi", "new_string": "hey"}"#,
        ),
        reply(&["I'll read it first."]),
        reply(&["Edit attempt"]),
    ])
    .await;
    checks_off(&mut state);
    let s = session::create(&state, Some(workspace("guard")))
        .await
        .unwrap();
    let (_, frames) = run_with(&state, &s.id, "edit", yes(), |_, _| {}).await;
    assert!(
        !frames
            .iter()
            .any(|f| matches!(f.event, Event::PermissionRequest { .. })),
        "a doomed edit isn't put to the user"
    );
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepCompleted { status: StepStatus::Error, summary, .. } if summary.contains("read hello.txt")
    )));
}

#[tokio::test]
async fn a_question_card_waits_for_answers_and_is_asked_once() {
    let (mut state, _) = setup(vec![
        tool_call(
            "c1",
            "ask_user",
            r#"{"why": "It decides the whole design", "questions": [
                {"text": "Which database?", "options": ["Postgres"], "recommended": "SQLite"},
                {"text": "Name?", "recommended": "notes"}]}"#,
        ),
        tool_call(
            "c2",
            "ask_user",
            r#"{"questions": [{"text": "Colour?", "recommended": "blue"}]}"#,
        ),
        reply(&["Done."]),
        reply(&["Asking"]),
    ])
    .await;
    checks_off(&mut state);
    let s = session::create(&state, Some(workspace("ask")))
        .await
        .unwrap();
    let answer = Reply::Question(vec!["Postgres".into()]);
    let (_, frames) = run_with(&state, &s.id, "build it", answer, |_, _| {}).await;

    let asked: Vec<_> = frames
        .iter()
        .filter_map(|f| match &f.event {
            Event::QuestionAsk { items, why, .. } => Some((items.clone(), why.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(asked.len(), 1, "one card per task");
    assert_eq!(asked[0].0[0].options, ["SQLite", "Postgres"]);
    assert_eq!(asked[0].1, "It decides the whole design");
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::QuestionResolved { answers, answered_by, .. }
            if answers == &["Postgres", "notes"] && answered_by == "test"
    )));
    let statuses: Vec<StepStatus> = frames
        .iter()
        .filter_map(|f| match f.event {
            Event::StepCompleted { status, .. } => Some(status),
            _ => None,
        })
        .collect();
    assert_eq!(statuses, vec![StepStatus::Ok, StepStatus::Error]);
}

#[tokio::test]
async fn full_auto_goes_with_thralls_own_answers() {
    let (mut state, _) = setup(vec![
        tool_call(
            "c1",
            "ask_user",
            r#"{"questions": [{"text": "Which?", "recommended": "this one"}]}"#,
        ),
        reply(&["Done."]),
        reply(&["Deciding"]),
    ])
    .await;
    checks_off(&mut state);
    let s = session::create(&state, Some(workspace("ask-auto")))
        .await
        .unwrap();
    session::set_mode(&state, &s.id, thrall_agent::Mode::FullAuto)
        .await
        .unwrap();
    let (_, frames) = run_with(&state, &s.id, "go", yes(), |_, _| {}).await;
    assert!(
        !kinds(&frames).contains(&"question.ask"),
        "Full auto never asks"
    );
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepCompleted { status: StepStatus::Ok, summary, .. } if summary == "Went with its own answers"
    )));
}
#[tokio::test]
async fn a_noted_lesson_is_kept_and_followed_from_then_on() {
    let (mut state, provider) = setup(vec![
        tool_call(
            "c1",
            "experience_note",
            r#"{"lesson": "  Use py,\n not python  "}"#,
        ),
        reply(&["Noted."]),
        reply(&["Lessons"]),
        reply(&["Using py."]),
    ])
    .await;
    checks_off(&mut state);
    let dir = workspace("lessons");
    state.data_dir = Some(std::path::PathBuf::from(&dir).join("data"));
    let s = session::create(&state, Some(dir.clone())).await.unwrap();
    let (_, frames) = run_with(&state, &s.id, "that's wrong, use py", yes(), |_, _| {}).await;
    assert!(
        !kinds(&frames).contains(&"permission.request"),
        "noted without asking"
    );
    let kept = thrall_agent::experience::list(&state);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].text, "Use py, not python");

    run_with(&state, &s.id, "run the script", yes(), |_, _| {}).await;
    let last = provider.seen.lock().unwrap().last().cloned().unwrap();
    assert!(last.messages[0].content.contains("- Use py, not python"));
}
#[tokio::test]
async fn plan_mode_blocks_changes_until_the_plan_is_approved() {
    let (mut state, provider) = setup(vec![
        tool_call("c1", "write_file", r#"{"path": "x.txt", "content": "x"}"#),
        tool_call(
            "c2",
            "exit_plan_mode",
            r##"{"plan": "# Plan\n- write x.txt"}"##,
        ),
        tool_call("c3", "write_file", r#"{"path": "x.txt", "content": "x"}"#),
        reply(&["Done."]),
        reply(&["Planning"]),
    ])
    .await;
    checks_off(&mut state);
    let ws = workspace("plan");
    let s = session::create(&state, Some(ws.clone())).await.unwrap();
    session::set_mode(&state, &s.id, thrall_agent::Mode::Plan)
        .await
        .unwrap();
    let approve = Reply::Plan {
        approve: true,
        auto_edits: true,
    };
    let (_, frames) = run_with(&state, &s.id, "make x", approve, |_, _| {}).await;

    let k = kinds(&frames);
    assert!(k.contains(&"plan.proposed") && k.contains(&"plan.resolved"));
    let statuses: Vec<StepStatus> = frames
        .iter()
        .filter_map(|f| match f.event {
            Event::StepCompleted { status, .. } => Some(status),
            _ => None,
        })
        .collect();
    assert_eq!(
        statuses,
        vec![StepStatus::Denied, StepStatus::Ok, StepStatus::Ok]
    );
    assert!(std::path::Path::new(&format!("{ws}/x.txt")).exists());
    assert_eq!(
        session::get(&state, &s.id).await.unwrap().mode,
        "auto_edits"
    );
    // The model was told it was in plan mode.
    let first = provider.seen.lock().unwrap()[0].clone();
    assert!(first.messages[0].content.contains("Plan mode is on"));
}

#[tokio::test]
async fn esc_stops_a_running_command_and_the_turn() {
    let command = if cfg!(windows) {
        "Start-Sleep -Seconds 30"
    } else {
        "sleep 30"
    };
    let (mut state, _) = setup(vec![
        tool_call("c1", "shell", &format!(r#"{{"command": "{command}"}}"#)),
        reply(&["Should not get here."]),
    ])
    .await;
    checks_off(&mut state);
    let s = session::create(&state, Some(workspace("esc")))
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let sid = s.id.clone();
    let (summary, frames) = run_with(&state, &s.id, "wait", yes(), move |st, e| {
        if matches!(e, Event::PermissionResolved { .. }) {
            let st_id = sid.clone();
            // Interrupt shortly after the command starts.
            std::thread::sleep(std::time::Duration::from_millis(800));
            st.interrupt(&st_id);
        }
    })
    .await;
    assert_eq!(summary.status, TurnStatus::Interrupted);
    assert!(started.elapsed().as_secs() < 15);
    assert!(frames.iter().any(|f| matches!(
        f.event,
        Event::StepCompleted {
            status: StepStatus::Error,
            ..
        }
    )));
    assert!(!state.is_running(&s.id));
}

#[tokio::test]
async fn stop_ends_a_tool_that_never_answers() {
    // A server that takes the connection and never replies: web_fetch would wait its full 30 s,
    // and doesn't watch for Stop itself (like a browser page frozen by a pop-up).
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        let _held: Vec<_> = listener.incoming().take(4).collect();
        std::thread::sleep(std::time::Duration::from_secs(60));
    });
    let (mut state, _) = setup(vec![
        tool_call("c1", "web_fetch", &format!(r#"{{"url": "{url}"}}"#)),
        reply(&["Should not get here."]),
    ])
    .await;
    checks_off(&mut state);
    let s = session::create(&state, Some(workspace("stuck")))
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let sid = s.id.clone();
    let (summary, frames) = run_with(&state, &s.id, "fetch", yes(), move |st, e| {
        if matches!(e, Event::StepStarted { .. }) {
            std::thread::sleep(std::time::Duration::from_millis(500));
            st.interrupt(&sid);
        }
    })
    .await;
    assert_eq!(summary.status, TurnStatus::Interrupted);
    assert!(
        started.elapsed().as_secs() < 15,
        "Stop ended it after the grace, not the tool's own 30 s: {:?}",
        started.elapsed()
    );
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepCompleted {
            status: StepStatus::Error,
            ..
        }
    )));
    assert!(!state.is_running(&s.id));
}

#[tokio::test]
async fn messages_typed_mid_turn_join_the_next_step() {
    let (mut state, provider) = setup(vec![
        tool_call("c1", "list_dir", "{}"),
        reply(&["Got it."]),
        reply(&["Listing"]),
    ])
    .await;
    checks_off(&mut state);
    let s = session::create(&state, Some(workspace("inbox")))
        .await
        .unwrap();
    let sid = s.id.clone();
    run_with(&state, &s.id, "list", yes(), move |st, e| {
        if matches!(e, Event::StepStarted { .. }) {
            assert!(st.add_to_running_turn(&sid, "also count them"));
        }
    })
    .await;
    let second = provider.seen.lock().unwrap()[1].clone();
    let last = second.messages.last().unwrap();
    assert_eq!(last.role, ChatRole::User);
    assert_eq!(last.content, "also count them");
}

#[tokio::test]
async fn unchecked_edits_trigger_one_verify_round() {
    let (state, provider) = setup(vec![
        tool_call("c1", "write_file", r#"{"path": "new.txt", "content": "x"}"#),
        reply(&["Created new.txt."]),
        tool_call("c2", "read_file", r#"{"path": "new.txt"}"#),
        reply(&["Checked: new.txt is right."]),
        reply(&["New file"]),
    ])
    .await;
    let s = session::create(&state, Some(workspace("verify")))
        .await
        .unwrap();
    let (summary, frames) = run_with(&state, &s.id, "make new.txt", yes(), |_, _| {}).await;
    assert_eq!(summary.reply, "Checked: new.txt is right.");
    assert!(
        frames
            .iter()
            .any(|f| matches!(&f.event, Event::Notice { text, .. } if text.contains("checking")))
    );
    let third = provider.seen.lock().unwrap()[2].clone();
    assert!(
        third
            .messages
            .last()
            .unwrap()
            .content
            .starts_with("[Thrall check]")
    );
}

#[tokio::test]
async fn false_claims_are_sent_back_once() {
    let (state, provider) = setup(vec![
        reply(&["I created answer.txt with the result."]),
        reply(&["Sorry, I didn't create a file; here's the answer: 42."]),
        reply(&["Answer"]),
    ])
    .await;
    let s = session::create(&state, Some(workspace("claim")))
        .await
        .unwrap();
    let (summary, _) = run_with(&state, &s.id, "what's the answer?", yes(), |_, _| {}).await;
    assert!(summary.reply.contains("42"));
    let second = provider.seen.lock().unwrap()[1].clone();
    assert!(
        second
            .messages
            .last()
            .unwrap()
            .content
            .contains("answer.txt")
    );
}

#[tokio::test]
async fn secrets_never_leave_in_replies() {
    let (mut state, _) = setup(vec![
        reply(&["Your key is sk-or-v1-abcdefghijklmnopqrstuvwxyz0123456789."]),
        reply(&["Key talk"]),
    ])
    .await;
    checks_off(&mut state);
    let s = session::create(&state, None).await.unwrap();
    let (summary, frames) = run_with(&state, &s.id, "what's my key", yes(), |_, _| {}).await;
    assert!(!summary.reply.contains("sk-or"));
    let completed = frames.iter().find_map(|f| match &f.event {
        Event::MessageCompleted { text, .. } if text.contains("Your key") => Some(text.clone()),
        _ => None,
    });
    assert!(completed.unwrap().contains("[redacted secret]"));
}

#[tokio::test]
async fn a_second_turn_in_the_same_session_is_refused_while_one_runs() {
    let (state, _) = setup(vec![reply(&["x"])]).await;
    let s = session::create(&state, None).await.unwrap();
    // Pretend a turn is running by sending into a channel nobody reads after the first event.
    assert!(!state.is_running(&s.id));
    assert!(!state.interrupt(&s.id));
    assert!(!state.add_to_running_turn(&s.id, "hi"));
}

#[tokio::test]
async fn a_plan_written_as_a_reply_still_gets_an_approval_card() {
    let (mut state, provider) = setup(vec![
        reply(&["Plan:\n1. Write x.txt"]),
        tool_call("c1", "write_file", r#"{"path": "x.txt", "content": "x"}"#),
        reply(&["Done."]),
        reply(&["Planning x"]),
    ])
    .await;
    checks_off(&mut state);
    let ws = workspace("plan-reply");
    let s = session::create(&state, Some(ws.clone())).await.unwrap();
    session::set_mode(&state, &s.id, thrall_agent::Mode::Plan)
        .await
        .unwrap();
    let approve = Reply::Plan {
        approve: true,
        auto_edits: true,
    };
    let (summary, frames) = run_with(&state, &s.id, "make x", approve, |_, _| {}).await;
    assert!(kinds(&frames).contains(&"plan.proposed"));
    assert_eq!(summary.reply, "Done.");
    assert!(std::path::Path::new(&format!("{ws}/x.txt")).exists());
    let second = provider.seen.lock().unwrap()[1].clone();
    assert!(
        second
            .messages
            .last()
            .unwrap()
            .content
            .contains("approved your plan")
    );
}

// ---------- project memory, @file, skills ----------

#[tokio::test]
async fn project_instructions_and_skills_reach_the_model() {
    let (mut state, provider) = setup(vec![
        tool_call("c1", "skill", r#"{"name": "greeting"}"#),
        reply(&["Hi!"]),
        reply(&["Greeting"]),
    ])
    .await;
    checks_off(&mut state);
    let ws = workspace("memory-file");
    std::fs::write(
        format!("{ws}/THRALL.md"),
        "Always answer in British English.",
    )
    .unwrap();
    let skill_dir = format!("{ws}/.thrall/skills/greeting");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        format!("{skill_dir}/SKILL.md"),
        "---\nname: greeting\ndescription: How to greet people\n---\nSay hi warmly.",
    )
    .unwrap();
    let s = session::create(&state, Some(ws)).await.unwrap();
    run_with(&state, &s.id, "greet me", yes(), |_, _| {}).await;

    let seen = provider.seen.lock().unwrap().clone();
    let system = &seen[0].messages[0].content;
    assert!(system.contains("Always answer in British English."));
    assert!(system.contains("- greeting: How to greet people"));
    let skill_result = seen[1].messages.last().unwrap();
    assert!(skill_result.content.contains("Say hi warmly."));
}

#[tokio::test]
async fn mentioned_files_are_attached_and_count_as_read() {
    let (mut state, provider) = setup(vec![
        tool_call(
            "c1",
            "edit_file",
            r#"{"path": "hello.txt", "old_string": "hi", "new_string": "hey"}"#,
        ),
        reply(&["Changed."]),
        reply(&["Editing hello"]),
    ])
    .await;
    checks_off(&mut state);
    let ws = workspace("mention");
    let s = session::create(&state, Some(ws.clone())).await.unwrap();
    session::set_mode(&state, &s.id, thrall_agent::Mode::AutoEdits)
        .await
        .unwrap();
    let (_, frames) = run_with(&state, &s.id, "change @hello.txt please", yes(), |_, _| {}).await;

    let first = provider.seen.lock().unwrap()[0].clone();
    let user = &first.messages[1].content;
    assert!(user.contains("[Attached: hello.txt]") && user.contains("hi"));
    // What the user sees is what they typed.
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::MessageCompleted { text, .. } if text == "change @hello.txt please"
    )));
    // The mention counted as a read, so the edit went through without a read_file first.
    assert_eq!(
        std::fs::read_to_string(format!("{ws}/hello.txt")).unwrap(),
        "hey\n"
    );
}

#[tokio::test]
async fn the_humaniser_adds_rules_and_checks_documents_on_full() {
    use thrall_core::config::Humanise;
    let (mut state, provider) = setup(vec![
        tool_call(
            "c1",
            "write_file",
            r#"{"path": "notes.md", "content": "Great question! The bridge stands as a testament to grit."}"#,
        ),
        tool_call(
            "c2",
            "write_file",
            r#"{"path": "main.rs", "content": "// Great question! A testament to grit."}"#,
        ),
        reply(&["Written."]),
        reply(&["Notes"]),
    ])
    .await;
    checks_off(&mut state);
    state.config.writing.humanise = Humanise::Full;
    let ws = workspace("humanise");
    let s = session::create(&state, Some(ws)).await.unwrap();
    session::set_mode(&state, &s.id, thrall_agent::Mode::AutoEdits)
        .await
        .unwrap();
    run_with(&state, &s.id, "write the notes", yes(), |_, _| {}).await;

    let seen = provider.seen.lock().unwrap().clone();
    assert!(seen[0].messages[0].content.contains("humaniser is on full"));
    let tool_text = |id: &str| {
        seen[2]
            .messages
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some(id))
            .map(|m| m.content.clone())
            .unwrap()
    };
    let notes = tool_text("c1");
    assert!(
        notes.contains("[Humaniser]") && notes.contains("chatbot talk"),
        "{notes}"
    );
    assert!(
        !tool_text("c2").contains("[Humaniser]"),
        "code is never style-checked"
    );

    // Off (the default): no rules in the prompt.
    let (state, provider) = setup(vec![reply(&["Hi."]), reply(&["Hi"])]).await;
    let s = session::create(&state, None).await.unwrap();
    run(&state, &s.id, "hello").await;
    let first = provider.seen.lock().unwrap()[0].clone();
    assert!(!first.messages[0].content.contains("Writing style"));
}

#[tokio::test]
async fn slash_humanise_asks_for_a_rewrite_and_detector_aware_adds_to_it() {
    let (mut state, provider) = setup(vec![
        reply(&["Rewritten."]),
        reply(&["Humanise"]),
        reply(&["Again."]),
    ])
    .await;
    checks_off(&mut state);
    let s = session::create(&state, None).await.unwrap();
    run(
        &state,
        &s.id,
        "/humanise Great question! It is a testament to grit.",
    )
    .await;
    let user = provider.seen.lock().unwrap()[0].messages[1].content.clone();
    assert!(
        user.contains("ran /humanise") && !user.contains("Detector-aware"),
        "{user}"
    );

    state.config.writing.detector_aware = true;
    run(&state, &s.id, "/humanize It is a testament to grit.").await;
    let seen = provider.seen.lock().unwrap().clone();
    let user = &seen.last().unwrap().messages.last().unwrap().content;
    assert!(
        user.contains("ran /humanise") && user.contains("Detector-aware"),
        "{user}"
    );
}

#[tokio::test]
async fn hooks_can_block_tools_and_add_to_messages() {
    use thrall_core::config::{HookConfig, HookEvent};
    let (mut state, provider) = setup(vec![
        tool_call("c1", "shell", r#"{"command": "echo should-not-run"}"#),
        reply(&["Blocked, fine."]),
        reply(&["Hooks"]),
    ])
    .await;
    checks_off(&mut state);
    let (block, say) = if cfg!(windows) {
        (
            "[Console]::Error.WriteLine('no shell today'); exit 2",
            "Write-Output 'hook says hi'",
        )
    } else {
        ("echo 'no shell today' >&2; exit 2", "echo 'hook says hi'")
    };
    state.config.hooks = vec![
        HookConfig {
            event: HookEvent::PreTool,
            tool: Some("shell".into()),
            command: block.into(),
            timeout_secs: None,
        },
        HookConfig {
            event: HookEvent::UserMessage,
            tool: None,
            command: say.into(),
            timeout_secs: None,
        },
    ];
    let s = session::create(&state, Some(workspace("hooks")))
        .await
        .unwrap();
    let (_, frames) = run_with(&state, &s.id, "run it", yes(), |_, _| {}).await;

    assert!(
        !frames
            .iter()
            .any(|f| matches!(f.event, Event::PermissionRequest { .. })),
        "a blocked tool isn't put to the user"
    );
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepCompleted { status: StepStatus::Denied, summary, .. } if summary == "Blocked by a hook"
    )));
    let seen = provider.seen.lock().unwrap().clone();
    assert!(seen[0].messages[1].content.contains("hook says hi"));
    assert!(
        seen[1]
            .messages
            .last()
            .unwrap()
            .content
            .contains("no shell today")
    );
}

// ---------- helpers, background tasks, scheduler ----------

#[tokio::test]
async fn a_helper_works_in_its_own_context_and_hands_back_its_answer() {
    let (mut state, provider) = setup(vec![
        tool_call(
            "c1",
            "agent",
            r#"{"title": "look around", "task": "List the folder and report."}"#,
        ),
        reply(&["The helper says: one file."]),
        reply(&["Helper test"]),
    ])
    .await;
    checks_off(&mut state);
    *provider.helper.lock().unwrap() = vec![
        tool_call("h1", "list_dir", "{}"),
        reply(&["Found hello.txt."]),
    ]
    .into();
    let s = session::create(&state, Some(workspace("helper")))
        .await
        .unwrap();
    let (summary, frames) = run_with(&state, &s.id, "what's here?", yes(), |_, _| {}).await;
    assert_eq!(summary.reply, "The helper says: one file.");
    // The helper's own step showed in the turn, under the agent step.
    let agent_step = frames
        .iter()
        .find_map(|f| match &f.event {
            Event::StepStarted { tool, step, .. } if tool == "agent" => Some(step.clone()),
            _ => None,
        })
        .unwrap();
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepStarted { tool, helper: Some(h), .. } if tool == "list_dir" && *h == agent_step
    )));
    let seen = provider.seen.lock().unwrap().clone();
    let helper_request = seen
        .iter()
        .find(|r| r.messages[0].content.starts_with("You are a helper"))
        .unwrap();
    assert!(
        !helper_request.tools.iter().any(|t| t.name == "agent"),
        "no helpers of helpers"
    );
    // The parent's next call got only the answer, not the helper's steps.
    let parent_second = seen
        .iter()
        .filter(|r| !r.messages[0].content.starts_with("You are a helper"))
        .nth(1)
        .unwrap();
    assert_eq!(
        parent_second.messages.last().unwrap().content,
        "Found hello.txt."
    );
    assert!(
        !parent_second
            .messages
            .iter()
            .any(|m| m.content.contains("hello.txt\n"))
    );
}

/// One reply asking for several tools at once.
fn tool_calls(calls: &[(&str, &str, &str)]) -> Script {
    let mut script: Script = calls
        .iter()
        .enumerate()
        .map(|(index, (id, name, arguments))| {
            Ok(StreamEvent::ToolCall(ToolCallDelta {
                index: index as u32,
                id: Some((*id).into()),
                name: Some((*name).into()),
                arguments: (*arguments).into(),
            }))
        })
        .collect();
    script.push(Ok(StreamEvent::Finished(Some("tool_calls".into()))));
    script.push(usage());
    script
}

#[tokio::test]
async fn helpers_asked_for_together_work_side_by_side_each_as_its_type() {
    let (mut state, provider) = setup(vec![
        tool_calls(&[
            (
                "c1",
                "agent",
                r#"{"title": "find out", "type": "researcher", "task": "What's the API?"}"#,
            ),
            (
                "c2",
                "agent",
                r#"{"title": "write it", "type": "coder", "task": "Write the client."}"#,
            ),
        ]),
        reply(&["Both done."]),
        reply(&["Helpers test"]),
    ])
    .await;
    checks_off(&mut state);
    *provider.helper.lock().unwrap() =
        vec![reply(&["First answer."]), reply(&["Second answer."])].into();
    let s = session::create(&state, Some(workspace("helpers-together")))
        .await
        .unwrap();
    let (summary, _) = run_with(&state, &s.id, "do both", yes(), |_, _| {}).await;
    assert_eq!(summary.reply, "Both done.");
    let seen = provider.seen.lock().unwrap().clone();
    let helpers: Vec<_> = seen
        .iter()
        .filter(|r| r.messages[0].content.starts_with("You are a helper"))
        .collect();
    assert_eq!(helpers.len(), 2);
    let researcher = helpers
        .iter()
        .find(|r| r.messages[1].content == "What's the API?")
        .unwrap();
    assert!(
        researcher.messages[0]
            .content
            .contains("You are a researcher")
    );
    assert!(
        researcher.messages[0].content.contains("You only look"),
        "a researcher only looks"
    );
    let coder = helpers
        .iter()
        .find(|r| r.messages[1].content == "Write the client.")
        .unwrap();
    assert!(coder.messages[0].content.contains("You are a coder"));
    assert!(!coder.messages[0].content.contains("You only look"));
    // Both results reached Thrall, in the order it asked.
    let parent_second = seen
        .iter()
        .filter(|r| !r.messages[0].content.starts_with("You are a helper"))
        .nth(1)
        .unwrap();
    let results: Vec<_> = parent_second
        .messages
        .iter()
        .filter_map(|m| m.tool_call_id.clone())
        .collect();
    assert_eq!(results, ["c1", "c2"]);
    // Thrall was told which helper types there are.
    assert!(seen[0].messages[0].content.contains("# Helper types"));
}

#[tokio::test]
async fn a_helper_stops_at_its_spending_limit() {
    let (mut state, provider) = setup(vec![
        tool_call(
            "c1",
            "agent",
            r#"{"title": "dig", "task": "Look everywhere."}"#,
        ),
        reply(&["Stopped early."]),
        reply(&["Limit test"]),
    ])
    .await;
    checks_off(&mut state);
    // Each fake call costs $0.002; a one-cent limit stops it after five.
    state.config.agent.helper_cost_limit_cents = 1;
    *provider.helper.lock().unwrap() = (0..20)
        .map(|i| tool_call(&format!("h{i}"), "list_dir", "{}"))
        .collect();
    let s = session::create(&state, Some(workspace("helper-limit")))
        .await
        .unwrap();
    run_with(&state, &s.id, "dig", yes(), |_, _| {}).await;
    let seen = provider.seen.lock().unwrap().clone();
    let helper_calls = seen
        .iter()
        .filter(|r| r.messages[0].content.starts_with("You are a helper"))
        .count();
    assert_eq!(helper_calls, 5);
    let parent_second = seen
        .iter()
        .filter(|r| !r.messages[0].content.starts_with("You are a helper"))
        .nth(1)
        .unwrap();
    assert!(
        parent_second
            .messages
            .last()
            .unwrap()
            .content
            .contains("spending limit")
    );
}

#[tokio::test]
async fn an_unknown_helper_type_is_refused_with_the_ones_there_are() {
    let (mut state, _provider) = setup(vec![
        tool_call(
            "c1",
            "agent",
            r#"{"title": "x", "type": "wizard", "task": "Do magic."}"#,
        ),
        reply(&["OK."]),
        reply(&["Wizard test"]),
    ])
    .await;
    checks_off(&mut state);
    let s = session::create(&state, Some(workspace("helper-wizard")))
        .await
        .unwrap();
    let (_, frames) = run_with(&state, &s.id, "magic", yes(), |_, _| {}).await;
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepCompleted { summary, .. } if summary.contains("no helper type called wizard")
            && summary.contains("researcher")
    )));
}

#[tokio::test]
async fn background_helpers_report_back_when_done() {
    let (mut state, provider) = setup(vec![
        tool_call(
            "c1",
            "agent",
            r#"{"title": "research", "task": "Find things.", "background": true}"#,
        ),
        reply(&["Started it."]),
        reply(&["Background"]),
    ])
    .await;
    checks_off(&mut state);
    *provider.helper.lock().unwrap() = vec![reply(&["Background answer."])].into();
    let state = Arc::new(state);
    thrall_agent::runners::start(state.clone());
    let mut notices = state.subscribe();
    let s = session::create(&state, Some(workspace("bg")))
        .await
        .unwrap();
    let (_, frames) = run_with(
        &state,
        &s.id,
        "research in the background",
        yes(),
        |_, _| {},
    )
    .await;
    let agent_id = frames
        .iter()
        .find_map(|f| match &f.event {
            Event::AgentStarted { agent, .. } => Some(agent.clone()),
            _ => None,
        })
        .unwrap();
    let done = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Ok(frame) = notices.recv().await
                && let Event::AgentCompleted {
                    agent,
                    status,
                    summary,
                    ..
                } = frame.event
                && agent == agent_id
            {
                return (status, summary);
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(done, ("done".to_owned(), "Background answer.".to_owned()));
    let row = state
        .store
        .get_agent("u_test", &agent_id)
        .await
        .unwrap()
        .unwrap();
    assert!(row.state.contains("Background answer."));
}

#[tokio::test]
async fn scheduled_jobs_run_unattended_and_never_ask() {
    let (mut state, _) = setup(vec![
        tool_call("c1", "shell", r#"{"command": "echo hi"}"#),
        reply(&["Couldn't run the command without you."]),
        reply(&["Job"]),
    ])
    .await;
    checks_off(&mut state);
    let state = Arc::new(state);
    let job_session = session::create(&state, Some(workspace("job")))
        .await
        .unwrap();
    state
        .store
        .add_job(
            "u_test",
            thrall_store::Job {
                id: "j_1".into(),
                session_id: Some(job_session.id.clone()),
                title: "echo".into(),
                schedule: "every 1d".into(),
                prompt: "say hi with echo".into(),
                enabled: true,
                last_run_at: None,
                next_run_at: 0,
            },
        )
        .await
        .unwrap();
    let mut notices = state.subscribe();
    thrall_agent::runners::run_due_jobs(&state).await;
    let mut saw_denied = false;
    let mut saw_request = false;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let Ok(frame) = notices.recv().await else {
                continue;
            };
            match frame.event {
                Event::PermissionRequest { .. } => saw_request = true,
                Event::StepCompleted {
                    status: StepStatus::Denied,
                    ref summary,
                    ..
                } if summary == "Not allowed in a scheduled run" => saw_denied = true,
                Event::AgentCompleted { ref agent, .. } if agent == "j_1" => break,
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert!(saw_denied && !saw_request);
    // The job moved on to its next run.
    let jobs = state.store.list_jobs("u_test").await.unwrap();
    assert!(jobs[0].next_run_at > 0 && jobs[0].last_run_at.is_some());
}

// ---------- long-term memory and compaction ----------

/// Vectors from keywords: dimension 0 = cats, 1 = dogs, 2 = always a little.
struct KeywordEmbedder;

impl thrall_llm::Embedder for KeywordEmbedder {
    fn embed<'a>(
        &'a self,
        _model: &'a str,
        inputs: Vec<String>,
    ) -> futures::future::BoxFuture<'a, Result<thrall_llm::Embeddings, LlmError>> {
        Box::pin(async move {
            let vectors = inputs
                .iter()
                .map(|t| {
                    let t = t.to_lowercase();
                    let cat = f32::from(u8::from(t.contains("cat") || t.contains("kitten")));
                    let dog = f32::from(u8::from(t.contains("dog") || t.contains("puppy")));
                    vec![cat, dog, 0.1]
                })
                .collect();
            Ok(thrall_llm::Embeddings {
                vectors,
                tokens: 3,
                cost_usd: None,
            })
        })
    }
}

#[tokio::test]
async fn finished_turns_are_remembered_and_recalled_later() {
    let (mut state, _) = setup(vec![
        reply(&["The blue whale is the largest animal."]),
        reply(&["Whale facts"]),
    ])
    .await;
    checks_off(&mut state);
    let s = session::create(&state, None).await.unwrap();
    run(&state, &s.id, "what is the largest animal?").await;
    let found = state
        .memory
        .search("u_test", "largest animal whale", 5)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, thrall_store::MemoryKind::Episode);
    assert!(found[0].text.contains("blue whale"));
}

#[tokio::test]
async fn facts_on_the_same_topic_replace_each_other() {
    let (state, _) = setup(vec![reply(&["x"])]).await;
    state
        .memory
        .save_fact("u_test", "editor", "Uses Vim")
        .await
        .unwrap();
    let saved = state
        .memory
        .save_fact("u_test", "Editor", "Switched to VS Code")
        .await
        .unwrap();
    assert_eq!(saved.replaced.len(), 1);
    let found = state
        .memory
        .search("u_test", "vim code editor", 5)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].text, "Switched to VS Code");
}

#[tokio::test]
async fn memory_search_finds_things_by_meaning() {
    let (state, _) = setup(vec![reply(&["x"])]).await;
    let state = state.with_embedder(Arc::new(KeywordEmbedder));
    state
        .memory
        .save_fact("u_test", "pets", "Has a cat called Tom")
        .await
        .unwrap();
    state
        .memory
        .save_fact("u_test", "car", "Drives a red hatchback")
        .await
        .unwrap();
    // No word in common with the fact, but the meaning matches.
    let found = state.memory.search("u_test", "kitten", 5).await.unwrap();
    assert_eq!(found.first().map(|r| r.title.as_str()), Some("pets"));
}

#[tokio::test]
async fn old_big_tool_results_are_trimmed_only_when_switched_on() {
    for on in [false, true] {
        let (mut state, provider) = setup(vec![
            tool_call("c1", "read_file", r#"{"path":"big.txt"}"#),
            reply(&["Read it."]),
            reply(&["Big file"]),
            reply(&["ok"]),
        ])
        .await;
        checks_off(&mut state);
        state.config.memory.remember_conversations = false;
        state.config.memory.trim_old_tool_results = on;
        let dir = workspace(&format!("trim-{on}"));
        let big: String = (0..300)
            .map(|i| format!("{i:04} {}\n", "x".repeat(55)))
            .collect();
        std::fs::write(std::path::Path::new(&dir).join("big.txt"), &big).unwrap();
        let s = session::create(&state, Some(dir)).await.unwrap();
        run(&state, &s.id, "read big.txt").await;
        for i in 0..3 {
            run(&state, &s.id, &format!("message {i}")).await;
        }

        // What the model was sent last: the old result, whole or trimmed.
        let last = provider
            .seen
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|r| r.messages.iter().any(|m| m.role == ChatRole::Tool))
            .cloned()
            .unwrap();
        let sent = last
            .messages
            .iter()
            .find(|m| m.role == ChatRole::Tool)
            .unwrap();
        assert_eq!(sent.content.contains("Trimmed by Thrall"), on, "on = {on}");
        assert_eq!(sent.content.contains("0299 "), !on, "on = {on}");
        assert!(sent.content.contains("0000 "), "the start is always sent");

        // The stored result keeps everything either way.
        let stored = state.store.context_messages("u_test", &s.id).await.unwrap();
        let tool = stored.iter().find(|m| m.role == "tool").unwrap();
        assert!(tool.content.contains("0299 "));
        assert!(!tool.content.contains("Trimmed by Thrall"));
    }
}

#[tokio::test]
async fn compact_summarises_the_older_conversation() {
    let (mut state, provider) = setup(vec![reply(&["ok"]), reply(&["Chat"]), reply(&["ok"])]).await;
    checks_off(&mut state);
    state.config.memory.remember_conversations = false;
    let s = session::create(&state, None).await.unwrap();
    for i in 0..6 {
        run(&state, &s.id, &format!("message {i}")).await;
    }
    let before = state
        .store
        .context_messages("u_test", &s.id)
        .await
        .unwrap()
        .len();
    let reply = commands::run(&state, &s.id, "compact", "").await.unwrap();
    assert!(reply.contains("Summarised"), "{reply}");
    let after = state.store.context_messages("u_test", &s.id).await.unwrap();
    assert!(after.len() < before);
    assert!(
        after[0]
            .content
            .contains("[Summary of the earlier conversation]")
    );
    // The most recent messages are untouched and still in order after the summary.
    assert!(after.last().unwrap().content.contains("ok"));
    // The summary, then (facts are saved without asking by default) the facts call.
    let seen = provider.seen.lock().unwrap().clone();
    let summary_request = &seen[seen.len() - 2];
    assert!(
        summary_request.messages[1]
            .content
            .contains("User: message 0")
    );
    let facts_request = seen.last().unwrap();
    let source = &facts_request.messages[1].content;
    assert!(source.contains("- message 0"));
    assert!(!source.contains("Thrall:"), "only the person's own words");
}

#[tokio::test]
async fn facts_the_person_stated_are_kept_after_a_compaction() {
    let mut replies = vec![reply(&["ok"]), reply(&["Chat"])];
    replies.extend((0..5).map(|_| reply(&["ok"])));
    replies.push(reply(&["Summary."]));
    replies.push(reply(&[
        r#"[{"topic": "editor", "content": "Uses VS Code for everything"}]"#,
    ]));
    let (mut state, _) = setup(replies).await;
    checks_off(&mut state);
    state.config.memory.remember_conversations = false;
    let s = session::create(&state, None).await.unwrap();
    for i in 0..6 {
        run(&state, &s.id, &format!("I always use VS Code, message {i}")).await;
    }
    commands::run(&state, &s.id, "compact", "").await.unwrap();
    let facts = state
        .store
        .search_knowledge("u_test", "", 50)
        .await
        .unwrap();
    assert!(
        facts
            .iter()
            .any(|f| f.content == "Uses VS Code for everything"),
        "{facts:?}"
    );
}

#[tokio::test]
async fn an_update_under_a_reworded_topic_still_replaces_the_fact() {
    let (state, _) = setup(vec![reply(&["x"])]).await;
    state
        .memory
        .save_fact("u_test", "favourite drink", "Flat white")
        .await
        .unwrap();
    state
        .memory
        .save_fact("u_test", "editor", "Uses Vim")
        .await
        .unwrap();
    let saved = state
        .memory
        .save_fact("u_test", "favourite drink updated", "Long black")
        .await
        .unwrap();
    assert_eq!(saved.replaced.len(), 1);
    let found = state
        .memory
        .search("u_test", "favourite drink", 5)
        .await
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].text, "Long black");
    // The subject stays under its first topic.
    assert_eq!(found[0].title, "favourite drink");
    // Explicit replacement by id works across different topics too.
    let vim = state.memory.search("u_test", "vim", 5).await.unwrap()[0]
        .id
        .clone();
    let saved = state
        .memory
        .save_fact_replacing("u_test", "code editor", "Uses VS Code", &[vim])
        .await
        .unwrap();
    assert_eq!(saved.replaced.len(), 1);
    assert!(
        state
            .memory
            .search("u_test", "vim", 5)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn attached_files_reach_the_model_and_show_by_name() {
    let (state, provider) = setup(vec![reply(&["Noted."]), reply(&["Shopping list"])]).await;
    let s = session::create(&state, Some(workspace("attach")))
        .await
        .unwrap();
    let dir = std::env::temp_dir().join(format!("thrall-attach-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("list.txt");
    std::fs::write(&file, "eggs, bread").unwrap();

    let (tx, mut rx) = mpsc::channel::<EventFrame>(1024);
    let summary = turn::run_with_files(&state, &s.id, "what's on it?", &[file], &tx)
        .await
        .unwrap();
    drop(tx);
    let mut frames = Vec::new();
    while let Some(f) = rx.recv().await {
        frames.push(f);
    }
    assert_eq!(summary.status, TurnStatus::Done);
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::MessageCompleted { text, files, .. }
            if text == "what's on it?" && files.len() == 1 && files[0].name == "list.txt"
    )));
    let seen = provider.seen.lock().unwrap();
    let user = seen[0]
        .messages
        .iter()
        .rev()
        .find(|m| m.role == ChatRole::User)
        .unwrap();
    assert!(
        user.content
            .contains("[The user attached this file: list.txt, saved at ")
    );
    assert!(user.content.contains("eggs, bread"));
}

const PICTURE_MODEL: &str = "google/gemini-2.5-flash";
const PICTURE_DESCRIPTION: &str = "A tabby cat asleep on a red mat.";

/// Send "what is this?" with a picture; returns every request the model saw and the events.
async fn send_picture(
    sees: Option<bool>,
    vision_model: &str,
    scripts: Vec<Script>,
) -> (Vec<ChatRequest>, Vec<EventFrame>) {
    send_picture_with(sees, vision_model, "", scripts).await
}

/// As `send_picture`, with a backup picture model.
async fn send_picture_with(
    sees: Option<bool>,
    vision_model: &str,
    backup: &str,
    scripts: Vec<Script>,
) -> (Vec<ChatRequest>, Vec<EventFrame>) {
    let (mut state, provider) = setup(scripts).await;
    state.config.provider.vision_model = vision_model.into();
    state.config.provider.vision_backup_model = backup.into();
    *provider.sees.lock().unwrap() = sees;
    // Each run its own folder: these tests run side by side.
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let s = session::create(&state, Some(workspace(&format!("picture-{run}"))))
        .await
        .unwrap();
    let dir = std::env::temp_dir().join(format!("thrall-picture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("cat.png");
    std::fs::write(&file, [0x89, b'P', b'N', b'G']).unwrap();
    let (tx, mut rx) = mpsc::channel::<EventFrame>(1024);
    turn::run_with_files(&state, &s.id, "what is this?", &[file], &tx)
        .await
        .unwrap();
    drop(tx);
    let mut frames = Vec::new();
    while let Some(f) = rx.recv().await {
        frames.push(f);
    }
    let seen = provider.seen.lock().unwrap().clone();
    (seen, frames)
}

fn last_user(request: &ChatRequest) -> &thrall_llm::ChatMessage {
    request
        .messages
        .iter()
        .rev()
        .find(|m| m.role == ChatRole::User)
        .unwrap()
}

#[tokio::test]
async fn without_a_picture_model_pictures_go_to_models_that_can_see() {
    let (seen, _) = send_picture(
        Some(true),
        "",
        vec![reply(&["A cat."]), reply(&["Cat picture"])],
    )
    .await;
    let user = last_user(&seen[0]);
    assert_eq!(user.images.len(), 1);
    assert_eq!(user.images[0].mime, "image/png");
    assert!(
        user.content
            .contains("[The user attached a picture: cat.png, saved at ")
    );
}

#[tokio::test]
async fn the_picture_model_reads_pictures_even_for_a_model_that_can_see() {
    let (seen, frames) = send_picture(
        Some(true),
        PICTURE_MODEL,
        vec![reply(&["It's a tabby cat."]), reply(&["Cat picture"])],
    )
    .await;
    // The picture model gets the picture and the user's question.
    let vision = seen.iter().find(|r| r.model == PICTURE_MODEL).unwrap();
    assert_eq!(vision.messages[0].images.len(), 1);
    assert!(vision.messages[0].content.contains("what is this?"));
    // The chat model gets the description instead of the picture.
    let chat = seen.iter().find(|r| !r.tools.is_empty()).unwrap();
    let user = last_user(chat);
    assert!(user.images.is_empty());
    assert!(
        user.content.contains("A tabby cat asleep on a red mat."),
        "{}",
        user.content
    );
    assert!(!user.content.contains("can't see pictures, so"));
    // It shows as a step after the message.
    let step = frames
        .iter()
        .position(|f| matches!(&f.event, Event::StepStarted { title, .. } if title == "Looking at cat.png"))
        .unwrap();
    let message = frames
        .iter()
        .position(
            |f| matches!(&f.event, Event::MessageCompleted { files, .. } if !files.is_empty()),
        )
        .unwrap();
    assert!(message < step);
}

#[tokio::test]
async fn without_a_picture_model_a_model_that_cant_see_is_told_so() {
    let (seen, _) = send_picture(
        Some(false),
        "",
        vec![reply(&["I can't see it."]), reply(&["Cat picture"])],
    )
    .await;
    let user = last_user(&seen[0]);
    assert!(user.images.is_empty());
    assert!(
        user.content.contains("can't see pictures"),
        "{}",
        user.content
    );
}

#[tokio::test]
async fn a_busy_picture_model_falls_back_to_the_backup() {
    let (seen, frames) = send_picture_with(
        Some(false),
        "busy/vision",
        PICTURE_MODEL,
        vec![reply(&["It's a tabby cat."]), reply(&["Cat picture"])],
    )
    .await;
    let chat = seen.iter().find(|r| !r.tools.is_empty()).unwrap();
    assert!(last_user(chat).content.contains(PICTURE_DESCRIPTION));
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepCompleted { status: StepStatus::Ok, summary, .. }
            if summary.contains("busy/vision was unavailable")
    )));
}

#[tokio::test]
async fn when_no_picture_model_can_read_thrall_says_so_without_suggesting_a_switch() {
    let (seen, frames) = send_picture_with(
        Some(false),
        "busy/vision",
        "busy/backup",
        vec![
            reply(&["Reading it failed; press Retry."]),
            reply(&["Cat picture"]),
        ],
    )
    .await;
    let chat = seen.iter().find(|r| !r.tools.is_empty()).unwrap();
    let user = last_user(chat);
    assert!(user.images.is_empty());
    assert!(
        user.content.contains("picture reader couldn't read"),
        "{}",
        user.content
    );
    assert!(!user.content.contains("/model"), "{}", user.content);
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepCompleted {
            status: StepStatus::Error,
            ..
        }
    )));
}

#[tokio::test]
async fn when_no_picture_model_can_read_a_model_that_sees_gets_the_picture() {
    let (seen, _) = send_picture_with(
        Some(true),
        "busy/vision",
        "busy/backup",
        vec![reply(&["A cat."]), reply(&["Cat picture"])],
    )
    .await;
    let chat = seen.iter().find(|r| !r.tools.is_empty()).unwrap();
    assert_eq!(last_user(chat).images.len(), 1);
}

/// A one-page PDF with only `text` on it (so it reads as scanned: little text per page).
fn write_pdf(path: &std::path::Path, text: &str) {
    let stream = format!("BT /F1 24 Tf 72 720 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_owned(),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{body}\nendobj\n", i + 1));
    }
    let xref = out.len();
    out.push_str(&format!(
        "xref\n0 {}\n0000000000 65535 f \n",
        objects.len() + 1
    ));
    for o in offsets {
        out.push_str(&format!("{o:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    ));
    std::fs::write(path, out).unwrap();
}

#[tokio::test]
async fn a_scanned_pdf_is_transcribed_by_the_picture_reader() {
    let (mut state, provider) = setup(vec![
        reply(&["Here are the questions."]),
        reply(&["Algebra"]),
    ])
    .await;
    state.config.provider.vision_model = PICTURE_MODEL.into();
    state.config.provider.vision_backup_model = String::new();
    let s = session::create(&state, Some(workspace("scanned")))
        .await
        .unwrap();
    let dir = std::env::temp_dir().join(format!("thrall-scanned-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("chapter3.pdf");
    write_pdf(&file, "3A");
    let (tx, mut rx) = mpsc::channel::<EventFrame>(1024);
    turn::run_with_files(&state, &s.id, "what are the questions?", &[file], &tx)
        .await
        .unwrap();
    drop(tx);
    let mut frames = Vec::new();
    while let Some(f) = rx.recv().await {
        frames.push(f);
    }
    let seen = provider.seen.lock().unwrap().clone();
    // The picture reader gets the PDF itself.
    let reader = seen.iter().find(|r| r.model == PICTURE_MODEL).unwrap();
    assert_eq!(reader.messages[0].documents.len(), 1);
    assert_eq!(reader.messages[0].documents[0].mime, "application/pdf");
    // The chat model gets the transcription and where the file is saved.
    let chat = seen.iter().find(|r| !r.tools.is_empty()).unwrap();
    let user = last_user(chat);
    assert!(
        user.content.contains("transcribed from its page images"),
        "{}",
        user.content
    );
    assert!(user.content.contains(PICTURE_DESCRIPTION));
    assert!(user.content.contains("saved at"), "{}", user.content);
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepStarted { title, .. } if title == "Reading chapter3.pdf"
    )));
}

/// Ask for a picture of a cat, answering the price prompt with `answer`.
async fn make_cat(
    answer: PermissionChoice,
    plan: bool,
) -> (Arc<FakeProvider>, Vec<EventFrame>, String) {
    let (mut state, provider) = setup(vec![
        tool_call(
            "c1",
            "make_image",
            r#"{"prompt":"a cat in a top hat","save_as":"cat.png"}"#,
        ),
        // In plan mode a statement would read as a plan; a question doesn't.
        reply(&[if plan {
            "Shall I draw it once the plan is approved?"
        } else {
            "Here's your cat."
        }]),
        reply(&["Cat picture"]),
    ])
    .await;
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = workspace(&format!("make-image-{run}"));
    state.data_dir = Some(std::path::PathBuf::from(&dir).join("data"));
    let s = session::create(&state, Some(dir.clone())).await.unwrap();
    if plan {
        session::set_mode(&state, &s.id, thrall_gates::Mode::Plan)
            .await
            .unwrap();
    }
    let (_, frames) = run_answering(&state, &s.id, "draw me a cat", answer).await;
    (provider, frames, dir)
}

#[tokio::test]
async fn pictures_are_made_after_the_user_approves_the_price() {
    let (provider, frames, dir) = make_cat(PermissionChoice::Yes, false).await;
    // The prompt says what, with which model, and about how much.
    let asked = frames
        .iter()
        .find_map(|f| match &f.event {
            Event::PermissionRequest { summary, .. } => Some(summary.clone()),
            _ => None,
        })
        .unwrap();
    assert!(asked.contains("google/gemini-3.1-flash-image"), "{asked}");
    assert!(asked.contains("about $0.08"), "{asked}");
    assert_eq!(provider.made.lock().unwrap().len(), 1);
    // The step shows the picture, which is saved like an upload and in the folder.
    let files = frames
        .iter()
        .find_map(|f| match &f.event {
            Event::StepCompleted {
                files,
                status: StepStatus::Ok,
                ..
            } if !files.is_empty() => Some(files.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(files[0].kind, "image");
    assert_eq!(files[0].name, "cat.png");
    let uploaded = std::path::Path::new(&dir)
        .join("data")
        .join("uploads")
        .join(&files[0].file)
        .join("cat.png");
    assert!(uploaded.is_file());
    assert!(std::path::Path::new(&dir).join("cat.png").is_file());
    // The picture's cost counts in the turn's total (2 model calls at $0.002 + the picture).
    let cost = frames
        .iter()
        .find_map(|f| match &f.event {
            Event::TurnCompleted { usage, .. } => usage.cost_usd,
            _ => None,
        })
        .unwrap();
    assert!((cost - 0.0814).abs() < 1e-9, "{cost}");
}

#[tokio::test]
async fn no_picture_is_made_or_paid_for_when_the_user_says_no() {
    let (provider, frames, _) = make_cat(PermissionChoice::No, false).await;
    assert!(provider.made.lock().unwrap().is_empty());
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepCompleted {
            status: StepStatus::Denied,
            ..
        }
    )));
}

#[tokio::test]
async fn plan_mode_makes_no_pictures() {
    let (provider, frames, _) = make_cat(PermissionChoice::Yes, true).await;
    assert!(provider.made.lock().unwrap().is_empty());
    assert!(
        !frames
            .iter()
            .any(|f| matches!(&f.event, Event::PermissionRequest { .. }))
    );
}

#[tokio::test]
async fn view_image_describes_a_picture_file_and_shows_it() {
    let (mut state, provider) = setup(vec![
        tool_call(
            "v1",
            "view_image",
            r#"{"path":"cat.png","question":"What colour is the cat?"}"#,
        ),
        reply(&["It's a tabby."]),
        reply(&["Cat"]),
    ])
    .await;
    state.config.provider.vision_model = PICTURE_MODEL.into();
    state.config.provider.vision_backup_model = String::new();
    let dir = workspace("view-image");
    state.data_dir = Some(std::path::PathBuf::from(&dir).join("data"));
    std::fs::write(
        std::path::Path::new(&dir).join("cat.png"),
        base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
        )
        .unwrap(),
    )
    .unwrap();
    let s = session::create(&state, Some(dir.clone())).await.unwrap();
    let (_, frames) = run(&state, &s.id, "what colour is cat.png?").await;
    // The picture reader got the picture and the question.
    let seen = provider.seen.lock().unwrap().clone();
    let reader = seen.iter().find(|r| r.model == PICTURE_MODEL).unwrap();
    assert_eq!(reader.messages[0].images.len(), 1);
    assert!(
        reader.messages[0]
            .content
            .contains("What colour is the cat?")
    );
    // Its description goes back to the chat model, and the step shows the picture.
    let last = seen.iter().rfind(|r| !r.tools.is_empty()).unwrap();
    assert!(
        last.messages
            .iter()
            .any(|m| m.content.contains(PICTURE_DESCRIPTION)),
        "the description reaches the model"
    );
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepCompleted { status: StepStatus::Ok, files, summary, .. }
            if files.len() == 1 && summary.starts_with("Read by")
    )));
}

#[tokio::test]
async fn attached_recordings_are_transcribed() {
    let (mut state, provider) = setup(vec![reply(&["Noted."]), reply(&["Voice note"])]).await;
    state.config.provider.transcribe_model = PICTURE_MODEL.into();
    let dir = workspace("recording");
    let clip = std::path::Path::new(&dir).join("note.mp3");
    std::fs::write(&clip, [0xFF, 0xFB, 0x90, 0x00]).unwrap();
    let s = session::create(&state, Some(dir.clone())).await.unwrap();
    let (tx, mut rx) = mpsc::channel::<EventFrame>(1024);
    turn::run_with_files(&state, &s.id, "what did I say?", &[clip], &tx)
        .await
        .unwrap();
    drop(tx);
    let mut frames = Vec::new();
    while let Some(f) = rx.recv().await {
        frames.push(f);
    }
    let seen = provider.seen.lock().unwrap().clone();
    let listener = seen.iter().find(|r| r.model == PICTURE_MODEL).unwrap();
    assert_eq!(listener.messages[0].audio.len(), 1);
    assert_eq!(listener.messages[0].audio[0].format, "mp3");
    let chat = seen.iter().find(|r| !r.tools.is_empty()).unwrap();
    let user = last_user(chat);
    assert!(
        user.content.contains("note.mp3, transcribed"),
        "{}",
        user.content
    );
    assert!(user.content.contains(PICTURE_DESCRIPTION));
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepStarted { title, .. } if title == "Listening to note.mp3"
    )));
}

#[tokio::test]
async fn long_recordings_are_made_small_and_cut_into_parts() {
    let Some(ffmpeg) = thrall_tools::tools::ffmpeg() else {
        eprintln!("ffmpeg isn't installed; skipped");
        return;
    };
    let (mut state, provider) = setup(vec![reply(&["Noted."]), reply(&["Talk"])]).await;
    state.config.provider.transcribe_model = PICTURE_MODEL.into();
    let dir = workspace("long-recording");
    let clip = std::path::Path::new(&dir).join("talk.mp3");
    // 21 minutes at 32 kbit/s: about 5 MB, over the size sent whole.
    let made = std::process::Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
        .args(["-i", "sine=frequency=440:sample_rate=16000:duration=1260"])
        .args(["-c:a", "libmp3lame", "-b:a", "32k"])
        .arg(&clip)
        .status()
        .unwrap();
    assert!(made.success());
    let s = session::create(&state, Some(dir.clone())).await.unwrap();
    let (tx, mut rx) = mpsc::channel::<EventFrame>(1024);
    turn::run_with_files(&state, &s.id, "what was said?", &[clip], &tx)
        .await
        .unwrap();
    drop(tx);
    let mut frames = Vec::new();
    while let Some(f) = rx.recv().await {
        frames.push(f);
    }
    let seen = provider.seen.lock().unwrap().clone();
    let parts: Vec<_> = seen.iter().filter(|r| r.model == PICTURE_MODEL).collect();
    assert_eq!(parts.len(), 3, "three parts of ten minutes at most");
    assert!(parts.iter().all(|r| r.messages[0].audio[0].format == "mp3"));
    let chat = seen.iter().find(|r| !r.tools.is_empty()).unwrap();
    let user = last_user(chat);
    for at in ["[0:00]\n", "[10:00]\n", "[20:00]\n"] {
        assert!(user.content.contains(at), "{}", user.content);
    }
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepOutput { text, .. } if text.contains("part 3 of 3")
    )));
}

#[tokio::test]
async fn speak_makes_a_recording_to_play() {
    let (mut state, _) = setup(vec![
        tool_call("s1", "speak", r#"{"text":"Good morning","voice":"nova"}"#),
        reply(&["Here it is."]),
        reply(&["Greeting"]),
    ])
    .await;
    let dir = workspace("speak");
    state.data_dir = Some(std::path::PathBuf::from(&dir).join("data"));
    let s = session::create(&state, Some(dir)).await.unwrap();
    let (_, frames) = run(&state, &s.id, "say good morning").await;
    let (files, summary) = frames
        .iter()
        .find_map(|f| match &f.event {
            Event::StepCompleted {
                files,
                summary,
                status: StepStatus::Ok,
                ..
            } if !files.is_empty() => Some((files.clone(), summary.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(files[0].kind, "audio");
    assert!(files[0].name.ends_with(".wav"));
    assert!(summary.contains("1 s · nova"), "{summary}");
}

#[tokio::test]
async fn speak_asks_before_saving_into_the_folder() {
    let (mut state, _) = setup(vec![
        tool_call("s1", "speak", r#"{"text":"Hi","save_as":"hi.wav"}"#),
        reply(&["Okay, not saved."]),
        reply(&["Greeting"]),
    ])
    .await;
    let dir = workspace("speak-save");
    state.data_dir = Some(std::path::PathBuf::from(&dir).join("data"));
    let s = session::create(&state, Some(dir.clone())).await.unwrap();
    let (_, frames) = run(&state, &s.id, "say hi and save it").await;
    assert!(
        frames
            .iter()
            .any(|f| matches!(&f.event, Event::PermissionRequest { .. }))
    );
    assert!(
        !std::path::Path::new(&dir).join("hi.wav").exists(),
        "a no writes nothing"
    );
}

/// Keys for tests (never the real keychain).
struct TestKeys(Option<String>);

impl thrall_agent::keys::KeyStore for TestKeys {
    fn get(&self, name: &str) -> Option<String> {
        (name == "fal").then(|| self.0.clone()).flatten()
    }
}

/// A fake fal.ai queue: the job finishes at once; the video is "MP4".
async fn fake_fal() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let b = base.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let base = b.clone();
            tokio::spawn(async move {
                let mut buf = vec![0; 16384];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let line = String::from_utf8_lossy(&buf[..n])
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_owned();
                let body = if line.starts_with("POST ") {
                    format!(
                        r#"{{"request_id":"r1","status_url":"{base}/status","response_url":"{base}/result"}}"#
                    )
                } else if line.starts_with("GET /status") {
                    r#"{"status":"COMPLETED"}"#.to_owned()
                } else if line.starts_with("GET /result") {
                    format!(r#"{{"video":{{"url":"{base}/video.mp4"}}}}"#)
                } else {
                    "MP4".to_owned()
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    base
}

async fn make_cat_video(key: Option<&str>) -> Vec<EventFrame> {
    let (mut state, _) = setup(vec![
        tool_call(
            "v1",
            "make_video",
            r#"{"prompt":"a cat walking on a beach","duration":5}"#,
        ),
        reply(&["Here's your video."]),
        reply(&["Cat video"]),
    ])
    .await;
    state.keys = Arc::new(TestKeys(key.map(str::to_owned)));
    state.config.provider.fal_base_url = fake_fal().await;
    let dir = workspace(&format!("video-{}", key.is_some()));
    state.data_dir = Some(std::path::PathBuf::from(&dir).join("data"));
    let s = session::create(&state, Some(dir)).await.unwrap();
    let (_, frames) = run_answering(
        &state,
        &s.id,
        "make a video of a cat",
        PermissionChoice::Yes,
    )
    .await;
    frames
}

#[tokio::test]
async fn videos_are_made_after_approving_the_exact_price() {
    let frames = make_cat_video(Some("fal-test-key")).await;
    let asked = frames
        .iter()
        .find_map(|f| match &f.event {
            Event::PermissionRequest { summary, .. } => Some(summary.clone()),
            _ => None,
        })
        .unwrap();
    assert!(
        asked.contains("5 s 720p 16:9 video with sound with Seedance 2.0 · $1.51"),
        "{asked}"
    );
    let files = frames
        .iter()
        .find_map(|f| match &f.event {
            Event::StepCompleted {
                files,
                status: StepStatus::Ok,
                ..
            } if !files.is_empty() => Some(files.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(files[0].kind, "video");
    assert_eq!(files[0].name, "video.mp4");
    // Progress was shown while waiting.
    assert!(frames.iter().any(
        |f| matches!(&f.event, Event::StepOutput { text, .. } if text.contains("Sent to fal.ai"))
    ));
    let cost = frames
        .iter()
        .find_map(|f| match &f.event {
            Event::TurnCompleted { usage, .. } => usage.cost_usd,
            _ => None,
        })
        .unwrap();
    assert!((cost - (1.512 + 0.004)).abs() < 1e-9, "{cost}");
}

#[tokio::test]
async fn without_a_fal_key_nothing_is_asked_or_spent() {
    let frames = make_cat_video(None).await;
    assert!(
        !frames
            .iter()
            .any(|f| matches!(&f.event, Event::PermissionRequest { .. }))
    );
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepCompleted { status: StepStatus::Error, summary, .. } if summary.contains("no fal.ai key")
    )));
}

#[tokio::test]
async fn a_rewind_puts_edited_files_back_and_removes_made_ones() {
    use thrall_protocol::RewindAction;
    let (mut state, _) = setup(vec![
        tool_call("c1", "read_file", r#"{"path": "hello.txt"}"#),
        tool_call(
            "c2",
            "edit_file",
            r#"{"path": "hello.txt", "old_string": "hi", "new_string": "hey"}"#,
        ),
        tool_call(
            "c3",
            "write_file",
            r#"{"path": "made.txt", "content": "new"}"#,
        ),
        tool_call("c4r", "read_file", r#"{"path": "mine.txt"}"#),
        tool_call(
            "c4",
            "write_file",
            r#"{"path": "mine.txt", "content": "thrall's"}"#,
        ),
        reply(&["Done."]),
        reply(&["Editing files"]),
    ])
    .await;
    checks_off(&mut state);
    let ws = workspace("rewind");
    std::fs::write(format!("{ws}/mine.txt"), "original").unwrap();
    let s = session::create(&state, Some(ws.clone())).await.unwrap();
    let (summary, _) = run_with(
        &state,
        &s.id,
        "edit it",
        Reply::Permission(PermissionChoice::Always),
        |_, _| {},
    )
    .await;
    assert_eq!(
        std::fs::read_to_string(format!("{ws}/hello.txt")).unwrap(),
        "hey\n"
    );
    // The person changes one of the files afterwards.
    std::fs::write(format!("{ws}/mine.txt"), "the person's").unwrap();

    let root = std::path::Path::new(&ws);
    let planned = thrall_agent::checkpoints::plan(&state, &s.id, &summary.turn_id, Some(root))
        .await
        .unwrap();
    let shown: Vec<(&str, RewindAction, bool)> = planned
        .iter()
        .map(|p| (p.file.path.as_str(), p.file.action, p.file.changed))
        .collect();
    assert_eq!(
        shown,
        vec![
            ("hello.txt", RewindAction::Restore, false),
            ("made.txt", RewindAction::Delete, false),
            ("mine.txt", RewindAction::Restore, true),
        ]
    );

    let (restored, failed) =
        thrall_agent::checkpoints::restore(planned, &["mine.txt".to_owned()]).await;
    assert_eq!(restored, vec!["hello.txt", "made.txt"]);
    assert!(failed.is_empty());
    assert_eq!(
        std::fs::read_to_string(format!("{ws}/hello.txt")).unwrap(),
        "hi\n"
    );
    assert!(!root.join("made.txt").exists());
    assert_eq!(
        std::fs::read_to_string(format!("{ws}/mine.txt")).unwrap(),
        "the person's",
        "a file left out keeps the person's change"
    );
}

#[tokio::test]
async fn saved_facts_are_announced_and_asked_for_when_saving_freely_is_off() {
    let save = || {
        tool_call(
            "c1",
            "memory_save",
            r#"{"topic": "editor", "content": "uses VS Code"}"#,
        )
    };
    let (mut state, _) = setup(vec![save(), reply(&["Noted."]), reply(&["Editor"])]).await;
    checks_off(&mut state);
    let s = session::create(&state, None).await.unwrap();
    let (_, frames) = run(&state, &s.id, "I use VS Code").await;
    assert!(
        !frames
            .iter()
            .any(|f| matches!(f.event, Event::PermissionRequest { .. })),
        "facts save without asking by default"
    );
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::MemorySaved { topic, content, .. } if topic == "editor" && content == "uses VS Code"
    )));

    // Off: Thrall asks first, every time, and saying no saves nothing.
    let (mut state, _) = setup(vec![save(), reply(&["Fine."]), reply(&["Editor"])]).await;
    checks_off(&mut state);
    state.config.memory.save_facts = false;
    let s = session::create(&state, None).await.unwrap();
    let (_, frames) = run(&state, &s.id, "I use VS Code").await;
    let asked = frames.iter().find_map(|f| match &f.event {
        Event::PermissionRequest {
            summary, choices, ..
        } => Some((summary.clone(), choices.clone())),
        _ => None,
    });
    let (summary, choices) = asked.expect("asked before saving");
    assert!(summary.contains("editor") && summary.contains("uses VS Code"));
    assert!(!choices.contains(&PermissionChoice::Always));
    assert!(
        !frames
            .iter()
            .any(|f| matches!(f.event, Event::MemorySaved { .. }))
    );
}

#[tokio::test]
async fn a_risky_learned_skill_is_put_to_the_person_first() {
    let (mut state, _) = setup(vec![
        tool_call(
            "c1",
            "skill_propose",
            r#"{"name": "Clean Up", "description": "Clean the build", "learned_from": "web", "instructions": "Remove-Item -Recurse build"}"#,
        ),
        reply(&["Not saved."]),
        reply(&["Cleaning"]),
    ])
    .await;
    checks_off(&mut state);
    let dir = workspace("skill-risky");
    let data = std::path::PathBuf::from(&dir).join("data");
    state.data_dir = Some(data.clone());
    let s = session::create(&state, Some(dir.clone())).await.unwrap();
    let no = Reply::Permission(PermissionChoice::No);
    let (_, frames) = run_with(&state, &s.id, "learn it", no, |_, _| {}).await;
    let summary = frames
        .iter()
        .find_map(|f| match &f.event {
            Event::PermissionRequest { summary, risk, .. } => {
                assert_eq!(*risk, thrall_protocol::Risk::High);
                Some(summary.clone())
            }
            _ => None,
        })
        .expect("a high-rated skill is put to the person");
    assert!(summary.contains("rated High") && summary.contains("deletes things"));
    assert!(
        !data.join("skills").join("clean-up").exists(),
        "turned down, so not kept"
    );
}
#[tokio::test]
async fn a_learned_skill_is_saved_without_asking_runs_in_a_helper_and_by_its_name() {
    let (mut state, provider) = setup(vec![
        tool_call(
            "c1",
            "skill_propose",
            r#"{"name": "Tidy Notes", "description": "Tidy a notes file", "learned_from": "user", "instructions": "1. Read it.\n2. Sort the lines."}"#,
        ),
        reply(&["Saved it."]),
        reply(&["Tidy notes"]),
        // Second turn: the skill runs in a helper.
        tool_call(
            "c2",
            "skill",
            r#"{"name": "tidy-notes", "task": "tidy notes.md"}"#,
        ),
        reply(&["Done."]),
        // Third turn: /tidy-notes typed.
        reply(&["On it."]),
    ])
    .await;
    checks_off(&mut state);
    let dir = workspace("skill-propose");
    let data = std::path::PathBuf::from(&dir).join("data");
    state.data_dir = Some(data.clone());
    let s = session::create(&state, Some(dir.clone())).await.unwrap();

    let (_, frames) = run_with(&state, &s.id, "tidy my notes", yes(), |_, _| {}).await;
    assert!(
        !kinds(&frames).contains(&"permission.request"),
        "a low-rated skill isn't put to the user"
    );
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepCompleted { summary, .. } if summary == "Learned skill tidy-notes (low)"
    )));
    let file = data.join("skills").join("tidy-notes").join("SKILL.md");
    let saved = std::fs::read_to_string(&file).unwrap();
    assert!(saved.contains("2. Sort the lines.") && saved.contains("rating: low"));

    // Used again: a helper does it; only its answer comes back.
    provider
        .helper
        .lock()
        .unwrap()
        .push_back(reply(&["Sorted 3 lines."]));
    let (_, frames) = run_with(&state, &s.id, "tidy notes.md again", yes(), |_, _| {}).await;
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepCompleted { summary, .. } if summary.contains("Skill finished in a helper")
    )));
    let helper_saw = provider
        .seen
        .lock()
        .unwrap()
        .iter()
        .find(|r| {
            r.messages
                .first()
                .is_some_and(|m| m.content.starts_with("You are a helper"))
        })
        .map(|r| r.messages[1].content.clone())
        .expect("a helper ran");
    assert!(helper_saw.contains("2. Sort the lines.") && helper_saw.contains("tidy notes.md"));

    // `/tidy-notes` tells Thrall to use it.
    run(&state, &s.id, "/tidy-notes the shopping list").await;
    let last_user = provider
        .seen
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|r| {
            r.messages
                .iter()
                .rev()
                .find(|m| m.role == ChatRole::User)
                .map(|m| m.content.clone())
        })
        .unwrap();
    assert!(last_user.contains("ran the skill below with /tidy-notes"));
}

#[tokio::test]
async fn home_starts_a_fresh_page_each_day_and_prunes_old_ones() {
    use thrall_core::config::FreshStart;
    use thrall_store::NewSession;
    let (mut state, _) = setup(vec![reply(&["x"])]).await;
    let old = |id: &str, at: i64| NewSession {
        id: id.into(),
        user_id: "u_test".into(),
        workspace: None,
        at,
    };
    // Yesterday's page (recent) and a page untouched for 40 days.
    let day_ms = 86_400_000;
    let now = thrall_core::ids::now_ms();
    state
        .store
        .create_session(old("s_y", now - day_ms))
        .await
        .unwrap();
    state
        .store
        .set_session_home("u_test", "s_y", "2000-01-02")
        .await
        .unwrap();
    // The person talked on yesterday's page, so it's kept as an earlier page.
    state
        .store
        .add_message(thrall_store::NewMessage {
            id: "m_y".into(),
            user_id: "u_test".into(),
            session_id: "s_y".into(),
            turn_id: None,
            role: "user".into(),
            content: r#"{"text":"hi"}"#.into(),
            tokens: 1,
            at: now - day_ms,
        })
        .await
        .unwrap();
    state
        .store
        .create_session(old("s_old", now - 40 * day_ms))
        .await
        .unwrap();
    state
        .store
        .set_session_home("u_test", "s_old", "2000-01-01")
        .await
        .unwrap();

    let page = thrall_agent::home::current(&state).await.unwrap();
    assert!(
        page.id != "s_y" && page.home_day.is_some(),
        "a new day, a new page"
    );
    let earlier = thrall_agent::home::earlier(&state, &page.id).await.unwrap();
    assert_eq!(
        earlier.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["s_y"],
        "the 40-day-old page is pruned (keep 30 days)"
    );
    assert_eq!(
        thrall_agent::home::current(&state).await.unwrap().id,
        page.id,
        "the same page for the rest of the day"
    );
    // Never: the newest page stays, whatever its day.
    state.config.home.fresh_start = FreshStart::Never;
    state
        .store
        .set_session_home("u_test", &page.id, "1999-12-31")
        .await
        .unwrap();
    assert_eq!(
        thrall_agent::home::current(&state).await.unwrap().id,
        page.id
    );
    // Daily again: the old-dated page was never used, so it becomes today's instead of a new one.
    state.config.home.fresh_start = FreshStart::Daily;
    let reused = thrall_agent::home::current(&state).await.unwrap();
    assert_eq!(reused.id, page.id);
    assert_ne!(reused.home_day.as_deref(), Some("1999-12-31"));
}

#[tokio::test]
async fn home_moves_out_of_the_data_folder_into_the_working_folder() {
    let (mut state, _) = setup(vec![]).await;
    let dir = std::path::PathBuf::from(workspace("home-moves"));
    let data = dir.join("data");
    state.data_dir = Some(data.clone());
    // Before 0.2.2: Home works inside the data folder.
    let page = thrall_agent::home::current(&state).await.unwrap();
    let old = data.join("home");
    assert_eq!(
        page.workspace.as_deref(),
        Some(old.display().to_string().as_str())
    );
    std::fs::write(old.join("meal-plan.md"), "oats").unwrap();

    // With a working folder, the folder moves there and the page follows it.
    let work = dir.join("work");
    state.work_dir = Some(work.clone());
    let moved = thrall_agent::home::current(&state).await.unwrap();
    let new = work.join("Home");
    assert_eq!(moved.id, page.id, "the same page");
    assert_eq!(
        moved.workspace.as_deref(),
        Some(new.display().to_string().as_str())
    );
    assert_eq!(
        std::fs::read_to_string(new.join("meal-plan.md")).unwrap(),
        "oats"
    );
    assert!(!old.exists(), "nothing left behind in the data folder");
}

#[test]
fn a_home_folder_name_keeps_only_plain_characters() {
    assert_eq!(thrall_agent::home::folder_name(" Tiff: <mum> "), "Tiff mum");
    assert_eq!(thrall_agent::home::folder_name("../.."), "");
}

#[tokio::test]
async fn thrall_reads_another_session_from_home() {
    let (mut state, provider) = setup(vec![
        reply(&["The thumbnail should be red."]),
        reply(&["Thumbnails"]),
        tool_call("c1", "sessions_list", "{}"),
        tool_call("c2", "session_read", r#"{"session": "thumbnail"}"#),
        reply(&["You decided on red."]),
    ])
    .await;
    checks_off(&mut state);
    let work = session::create(&state, None).await.unwrap();
    run(&state, &work.id, "what colour for the thumbnail?").await;
    let home = thrall_agent::home::current(&state).await.unwrap();
    run(&state, &home.id, "what did we decide about the thumbnail?").await;

    let seen = provider.seen.lock().unwrap().clone();
    let last = seen.last().unwrap();
    assert!(last.messages[0].content.contains("This is Home"));
    let result = |id: &str| {
        last.messages
            .iter()
            .find(|m| m.tool_call_id.as_deref() == Some(id))
            .map(|m| m.content.clone())
            .unwrap()
    };
    let listed = result("c1");
    assert!(
        listed.contains(&work.id) && listed.contains("[this session, Home]"),
        "{listed}"
    );
    let read = result("c2");
    assert!(
        read.contains("You: what colour for the thumbnail?")
            && read.contains("Thrall: The thumbnail should be red."),
        "{read}"
    );
}

#[tokio::test]
async fn check_ins_stay_quiet_when_all_is_well_and_only_look() {
    use thrall_agent::heartbeat::{self, Outcome};
    let (mut state, provider) = setup(vec![
        // First check-in: all clear.
        reply(&["HEARTBEAT_OK"]),
        // Second: tries to change a file (refused), then speaks up.
        tool_call("c1", "write_file", r#"{"path": "x.md", "content": "hi"}"#),
        reply(&["Your build failed overnight."]),
    ])
    .await;
    checks_off(&mut state);
    let data = std::path::PathBuf::from(workspace("heartbeat"));
    state.data_dir = Some(data.clone());
    state.config.heartbeat.model = "cheap/model".into();

    assert_eq!(
        heartbeat::beat(&state, false).await.unwrap(),
        Outcome::NoChecklist
    );
    heartbeat::save_checklist(&state, "- Did last night's build pass?").unwrap();
    assert_eq!(
        heartbeat::checklist(&state),
        "- Did last night's build pass?"
    );

    assert_eq!(
        heartbeat::beat(&state, false).await.unwrap(),
        Outcome::AllClear
    );
    let page = thrall_agent::home::current(&state).await.unwrap();
    assert!(
        state
            .store
            .context_messages("u_test", &page.id)
            .await
            .unwrap()
            .is_empty(),
        "an all-clear is taken back"
    );
    let seen = provider.seen.lock().unwrap().clone();
    assert_eq!(seen[0].model, "cheap/model");
    let asked = &seen[0].messages.last().unwrap().content;
    assert!(
        asked.starts_with("Check-in") && asked.contains("last night's build"),
        "{asked}"
    );

    let outcome = heartbeat::beat(&state, false).await.unwrap();
    assert_eq!(
        outcome,
        Outcome::Posted("Your build failed overnight.".into())
    );
    assert!(
        !data.join("home").join("x.md").exists(),
        "check-ins only look"
    );
    let last = provider.seen.lock().unwrap().last().unwrap().clone();
    let refused = last
        .messages
        .iter()
        .find(|m| m.tool_call_id.as_deref() == Some("c1"))
        .map(|m| m.content.clone())
        .unwrap();
    assert!(refused.contains("check-in"), "{refused}");
    assert!(
        !state
            .store
            .context_messages("u_test", &page.id)
            .await
            .unwrap()
            .is_empty()
    );
    // The check-in's own model is only for check-ins.
    assert_eq!(state.model_for(&page.id), state.model());
    heartbeat::save_checklist(&state, "  ").unwrap();
    assert!(heartbeat::checklist(&state).is_empty());
}

#[tokio::test]
async fn an_empty_reply_is_sent_back_once_for_a_summary() {
    let (state, provider) = setup(vec![
        reply(&[]),
        reply(&["Drew the board and saved it."]),
        reply(&["Board drawing"]),
    ])
    .await;
    let s = session::create(&state, None).await.unwrap();
    let (summary, _) = run(&state, &s.id, "draw the board").await;
    assert_eq!(summary.status, TurnStatus::Done);
    assert_eq!(summary.reply, "Drew the board and saved it.");
    let seen = provider.seen.lock().unwrap();
    assert!(
        seen[1]
            .messages
            .iter()
            .any(|m| m.content.contains("Your last reply was empty")),
        "the model was asked for a summary"
    );
}

#[tokio::test]
async fn a_reply_cut_off_at_its_length_limit_is_asked_for_smaller_steps() {
    let cut_off: Script = vec![Ok(StreamEvent::Finished(Some("length".into()))), usage()];
    let (state, provider) = setup(vec![
        cut_off,
        reply(&["Wrote the app in three files."]),
        reply(&["Recipe scaler"]),
    ])
    .await;
    let s = session::create(&state, None).await.unwrap();
    let (summary, _) = run(&state, &s.id, "build the recipe scaler").await;
    assert_eq!(summary.reply, "Wrote the app in three files.");
    let seen = provider.seen.lock().unwrap();
    let asked = &seen[1].messages;
    assert!(
        asked
            .iter()
            .any(|m| m.content.contains("cut off at the length limit")),
        "the model is told why"
    );
    assert!(
        !asked
            .iter()
            .any(|m| m.content.contains("Your last reply was empty")),
        "not the summary request"
    );
    assert_eq!(
        seen[0].thinking, None,
        "the model thinks as it likes at first"
    );
    assert_eq!(
        seen[1].thinking,
        Some("low"),
        "then less, so it has room to reply"
    );
}

#[tokio::test]
async fn an_empty_reply_on_nearly_no_credit_says_to_top_up_without_asking_again() {
    // OpenRouter cuts replies to what the credit can pay for: they come back empty.
    let (state, provider) = setup(vec![reply(&[]), reply(&["Never asked for"])]).await;
    *provider.credit.lock().unwrap() = Some(0.03);
    let s = session::create(&state, None).await.unwrap();
    let (summary, frames) = run(&state, &s.id, "finish the harvest table").await;
    assert_eq!(summary.status, TurnStatus::Done);
    assert!(
        frames.iter().any(|f| matches!(&f.event,
            Event::Notice { text, .. } if text.contains("credit is almost used up ($0.03 left)"))),
        "the person is told to top up"
    );
    let seen = provider.seen.lock().unwrap();
    assert!(
        !seen.iter().any(|r| r
            .messages
            .iter()
            .any(|m| m.content.contains("Your last reply was empty"))),
        "no second (paid) call to ask for a summary"
    );
}

#[tokio::test]
async fn a_turn_never_ends_with_a_silent_blank() {
    let (state, provider) = setup(vec![reply(&[]), reply(&[]), reply(&["A title"])]).await;
    let s = session::create(&state, None).await.unwrap();
    let (summary, frames) = run(&state, &s.id, "draw the board").await;
    assert_eq!(summary.status, TurnStatus::Done);
    assert!(
        frames.iter().any(|f| matches!(&f.event,
            Event::Notice { text, .. } if text.contains("finished without a reply"))),
        "the person is told"
    );
    // Asked once, not in a loop.
    assert!(provider.seen.lock().unwrap().len() <= 3);
}

// ---------- split mode (experimental): the Thinker plans and checks, the Doer works ----------

fn split_on(state: &mut AppState) {
    state.config.split.enabled = true;
    state.config.split.thinker_model = "big/thinker".into();
    state.config.split.doer_model = "cheap/doer".into();
}

fn is_doer(r: &ChatRequest) -> bool {
    r.messages[0].content.starts_with("You are a helper")
}

fn is_ask(r: &ChatRequest) -> bool {
    r.messages[0]
        .content
        .starts_with("You are the Thinker in split mode")
}

#[tokio::test]
async fn split_mode_hands_the_work_to_the_doer_and_tags_every_call() {
    let (mut state, provider) = setup(vec![
        tool_call(
            "c1",
            "hand_off",
            r#"{"brief": "Write done.txt containing ok, then read it back."}"#,
        ),
        reply(&["Yes, end it with a newline."]),
        reply(&["Done: done.txt says ok."]),
        reply(&["Split test"]),
    ])
    .await;
    checks_off(&mut state);
    split_on(&mut state);
    *provider.helper.lock().unwrap() = vec![
        tool_call(
            "d1",
            "write_file",
            r#"{"path": "done.txt", "content": "ok\n"}"#,
        ),
        tool_call(
            "d2",
            "ask_thinker",
            r#"{"question": "Should it end with a newline?"}"#,
        ),
        reply(&["Wrote done.txt (ok)."]),
    ]
    .into();
    let dir = workspace("split");
    let s = session::create(&state, Some(dir.clone())).await.unwrap();
    let (summary, frames) = run_with(&state, &s.id, "make done.txt", yes(), |_, _| {}).await;
    assert_eq!(summary.reply, "Done: done.txt says ok.");
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&dir).join("done.txt")).unwrap(),
        "ok\n"
    );
    for tool in ["hand_off", "write_file", "ask_thinker"] {
        assert!(
            frames
                .iter()
                .any(|f| matches!(&f.event, Event::StepStarted { tool: t, .. } if t == tool)),
            "{tool} showed as a step"
        );
    }

    let seen = provider.seen.lock().unwrap().clone();
    // The Thinker: its own model, told its part, offered hand_off but not the spending tools.
    let thinker = &seen[0];
    assert_eq!(thinker.model, "big/thinker");
    assert!(thinker.messages[0].content.contains("# Split mode"));
    let names: Vec<&str> = thinker.tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"hand_off"));
    assert!(!names.contains(&"agent") && !names.contains(&"make_image"));
    // The Doer: its own model, the brief as its task, and a way to ask.
    let doer = seen.iter().find(|r| is_doer(r)).unwrap();
    assert_eq!(doer.model, "cheap/doer");
    assert!(doer.messages[0].content.contains("You are the Doer"));
    assert!(doer.messages[1].content.contains("Write done.txt"));
    assert!(doer.tools.iter().any(|t| t.name == "ask_thinker"));
    // The question went to the Thinker's model with the brief, and the answer came back.
    let ask = seen.iter().find(|r| is_ask(r)).unwrap();
    assert_eq!(ask.model, "big/thinker");
    assert!(ask.tools.is_empty());
    assert!(ask.messages[1].content.contains("Write done.txt"));
    assert!(
        ask.messages[1]
            .content
            .contains("Should it end with a newline?")
    );
    let doer_last = seen.iter().rfind(|r| is_doer(r)).unwrap();
    assert_eq!(
        doer_last.messages.last().unwrap().content,
        "Yes, end it with a newline."
    );
    // The Thinker got the Doer's report, not its steps.
    let thinker_second = seen
        .iter()
        .filter(|r| !is_doer(r) && !is_ask(r))
        .nth(1)
        .unwrap();
    assert_eq!(
        thinker_second.messages.last().unwrap().content,
        "Wrote done.txt (ok)."
    );

    // Every call's usage row says who made it, on which model.
    let rows = state
        .store
        .turn_usage("u_test", &summary.turn_id)
        .await
        .unwrap();
    let who: Vec<(&str, &str)> = rows
        .iter()
        .map(|r| (r.role.as_str(), r.model.as_str()))
        .collect();
    assert_eq!(
        &who[..6],
        &[
            ("thinker", "big/thinker"),
            ("doer", "cheap/doer"),
            ("doer", "cheap/doer"),
            ("ask", "big/thinker"),
            ("doer", "cheap/doer"),
            ("thinker", "big/thinker"),
        ]
    );
    // The title is an ordinary call.
    assert_eq!(rows.last().unwrap().role, "chat");
    // The turn's total counts every call of the turn (the title comes after it ends).
    let cost: f64 = rows[..rows.len() - 1]
        .iter()
        .filter_map(|r| r.cost_usd)
        .sum();
    assert!((summary.usage.cost_usd.unwrap() - cost).abs() < 1e-9);
}

#[tokio::test]
async fn the_thinker_only_looks() {
    let (mut state, provider) = setup(vec![
        tool_call(
            "c1",
            "write_file",
            r#"{"path": "nope.txt", "content": "no"}"#,
        ),
        reply(&["I'll hand that off instead."]),
        reply(&["Look only"]),
    ])
    .await;
    checks_off(&mut state);
    split_on(&mut state);
    let dir = workspace("thinker-looks");
    let s = session::create(&state, Some(dir.clone())).await.unwrap();
    let (_, frames) = run_with(&state, &s.id, "write nope.txt", yes(), |_, _| {}).await;
    assert!(!std::path::Path::new(&dir).join("nope.txt").exists());
    assert!(
        !frames
            .iter()
            .any(|f| matches!(f.event, Event::PermissionRequest { .. })),
        "refused without asking the person"
    );
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::StepCompleted { status: StepStatus::Denied, summary, .. } if summary == "The Thinker only looks"
    )));
    let seen = provider.seen.lock().unwrap().clone();
    assert!(
        seen[1]
            .messages
            .last()
            .unwrap()
            .content
            .contains("you can only look")
    );
}

#[tokio::test]
async fn the_thinker_sends_work_back_once_at_most() {
    let brief = r#"{"brief": "Do the thing."}"#;
    let (mut state, provider) = setup(vec![
        tool_call("c1", "hand_off", brief),
        tool_call("c2", "hand_off", brief),
        tool_call("c3", "hand_off", brief),
        reply(&["Here's where it got to."]),
        reply(&["Send back"]),
    ])
    .await;
    checks_off(&mut state);
    split_on(&mut state);
    *provider.helper.lock().unwrap() = vec![reply(&["First go."]), reply(&["Second go."])].into();
    let s = session::create(&state, Some(workspace("send-back")))
        .await
        .unwrap();
    let (summary, _) = run_with(&state, &s.id, "do the thing", yes(), |_, _| {}).await;
    assert_eq!(summary.reply, "Here's where it got to.");
    let seen = provider.seen.lock().unwrap().clone();
    assert_eq!(seen.iter().filter(|r| is_doer(r)).count(), 2);
    let thinker_fourth = seen.iter().filter(|r| !is_doer(r)).nth(3).unwrap();
    assert!(
        thinker_fourth
            .messages
            .last()
            .unwrap()
            .content
            .contains("used both hand-offs")
    );
}

#[tokio::test]
async fn the_doer_can_only_ask_so_many_questions() {
    let ask = r#"{"question": "Which one?"}"#;
    let (mut state, provider) = setup(vec![
        tool_call("c1", "hand_off", r#"{"brief": "Pick one."}"#),
        reply(&["The first."]),
        reply(&["Still the first."]),
        reply(&["I said the first."]),
        reply(&["It picked one."]),
        reply(&["Questions"]),
    ])
    .await;
    checks_off(&mut state);
    split_on(&mut state);
    *provider.helper.lock().unwrap() = vec![
        tool_call("d1", "ask_thinker", ask),
        tool_call("d2", "ask_thinker", ask),
        tool_call("d3", "ask_thinker", ask),
        tool_call("d4", "ask_thinker", ask),
        reply(&["Picked the first."]),
    ]
    .into();
    let s = session::create(&state, Some(workspace("asks")))
        .await
        .unwrap();
    let (summary, _) = run_with(&state, &s.id, "pick one", yes(), |_, _| {}).await;
    assert_eq!(summary.reply, "It picked one.");
    let seen = provider.seen.lock().unwrap().clone();
    assert_eq!(seen.iter().filter(|r| is_ask(r)).count(), 3);
    let doer_last = seen.iter().rfind(|r| is_doer(r)).unwrap();
    assert!(
        doer_last
            .messages
            .last()
            .unwrap()
            .content
            .contains("asked the Thinker all you can")
    );
}

#[tokio::test]
async fn the_thinker_is_asked_to_check_what_the_doer_changed() {
    let (mut state, provider) = setup(vec![
        tool_call("c1", "hand_off", r#"{"brief": "Write done.txt."}"#),
        reply(&["All done."]),
        tool_call("c2", "read_file", r#"{"path": "done.txt"}"#),
        reply(&["Checked it: done.txt says ok."]),
        reply(&["Checking"]),
    ])
    .await;
    split_on(&mut state);
    *provider.helper.lock().unwrap() = vec![
        tool_call(
            "d1",
            "write_file",
            r#"{"path": "done.txt", "content": "ok\n"}"#,
        ),
        reply(&["Wrote done.txt."]),
    ]
    .into();
    let s = session::create(&state, Some(workspace("split-verify")))
        .await
        .unwrap();
    let (summary, frames) = run_with(&state, &s.id, "make done.txt", yes(), |_, _| {}).await;
    assert_eq!(summary.reply, "Checked it: done.txt says ok.");
    assert!(frames.iter().any(|f| matches!(
        &f.event,
        Event::Notice { text, .. } if text.contains("checking its work")
    )));
}

#[tokio::test]
async fn without_split_mode_calls_are_ordinary_chat_calls() {
    let (state, provider) = setup(vec![reply(&["Hi."]), reply(&["Plain"])]).await;
    let s = session::create(&state, Some(workspace("plain")))
        .await
        .unwrap();
    let (summary, _) = run(&state, &s.id, "hello").await;
    let seen = provider.seen.lock().unwrap().clone();
    assert!(!seen[0].tools.iter().any(|t| t.name == "hand_off"));
    assert!(!seen[0].messages[0].content.contains("# Split mode"));
    let rows = state
        .store
        .turn_usage("u_test", &summary.turn_id)
        .await
        .unwrap();
    assert!(!rows.is_empty());
    assert!(
        rows.iter()
            .all(|r| r.role == "chat" && r.model == state.model())
    );
}

/// A local model server that lists one model.
async fn fake_local_server() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let Ok((mut sock, _)) = listener.accept().await else {
            return;
        };
        let mut buf = vec![0; 4096];
        let _ = sock.read(&mut buf).await;
        let body = r#"{"data":[{"id":"llama3.1"}]}"#;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = sock.write_all(reply.as_bytes()).await;
    });
    base
}

#[tokio::test]
async fn testing_a_supplier_says_what_works_and_how_to_fix_what_doesnt() {
    use thrall_protocol::Reply;
    let (mut state, _) = setup(Vec::new()).await;
    state.keys = Arc::new(TestKeys(Some("fal-key".to_owned())));
    state.config.provider.local_base_url = fake_local_server().await;
    let tested = |r: Reply| match r {
        Reply::ProviderTested {
            ok,
            message,
            models,
            ..
        } => (ok, message, models),
        other => panic!("not provider.tested: {other:?}"),
    };
    let (ok, _, models) = tested(thrall_agent::settings::test_supplier(&state, "local").await);
    assert!(ok);
    assert_eq!(models, vec!["local/llama3.1"]);
    let (ok, message, _) = tested(thrall_agent::settings::test_supplier(&state, "fal").await);
    assert!(ok, "{message}");
    let (ok, message, _) = tested(thrall_agent::settings::test_supplier(&state, "nobody").await);
    assert!(!ok && message.contains("doesn't know"));
    // No key (and none in the environment): say how to add one.
    if std::env::var_os("ELEVENLABS_API_KEY").is_none() {
        let (ok, message, _) =
            tested(thrall_agent::settings::test_supplier(&state, "elevenlabs").await);
        assert!(
            !ok && message.contains("thrall key set elevenlabs"),
            "{message}"
        );
    }
}

#[tokio::test]
async fn commands_the_owner_turned_off_never_run_or_ask() {
    let marker = std::env::temp_dir().join(format!("thrall-rule-cmd-{}", std::process::id()));
    let _ = std::fs::remove_file(&marker);
    let command = format!(
        "{{\"command\": \"echo x > '{}'\"}}",
        marker.to_string_lossy().replace('\\', "/")
    );
    let (state, _) = setup(vec![
        tool_call("c1", "shell", &command),
        reply(&["I can't run commands here."]),
        reply(&["No commands"]),
    ])
    .await;
    state
        .store
        .set_person_rules("u_test", r#"{"no_commands":true}"#)
        .await
        .unwrap();
    let s = session::create(&state, Some(workspace("rule-cmd")))
        .await
        .unwrap();
    let (_, frames) = run_answering(&state, &s.id, "make a file", PermissionChoice::Yes).await;
    assert!(
        !frames
            .iter()
            .any(|f| matches!(f.event, Event::PermissionRequest { .. })),
        "nobody is asked about a command that isn't allowed"
    );
    assert!(frames.iter().any(|f| matches!(
        f.event,
        Event::StepCompleted {
            status: StepStatus::Denied,
            ..
        }
    )));
    assert!(!marker.exists(), "a command that's off must not run");
}

#[tokio::test]
async fn a_childs_answers_are_fit_for_their_age() {
    let (state, provider) = setup(vec![reply(&["Hi!"]), reply(&["Hello"])]).await;
    state
        .store
        .set_person_rules("u_test", r#"{"child":"under_13"}"#)
        .await
        .unwrap();
    let s = session::create(&state, Some(workspace("rule-child")))
        .await
        .unwrap();
    run(&state, &s.id, "hello").await;
    let seen = provider.seen.lock().unwrap();
    assert!(seen[0].messages[0].content.contains("child under 13"));
}

#[tokio::test]
async fn a_sessions_pc_work_never_runs_here_when_its_pc_is_away() {
    let (state, _) = setup(vec![
        tool_call(
            "c1",
            "write_file",
            r#"{"path": "made.txt", "content": "x"}"#,
        ),
        reply(&["Your PC isn't on."]),
        reply(&["PC away"]),
    ])
    .await;
    let ws = workspace("pc-away");
    let s = session::create(&state, Some(ws.clone())).await.unwrap();
    state
        .store
        .set_session_pc("u_test", &s.id, Some("p_away"))
        .await
        .unwrap();
    let (_, frames) = run_answering(&state, &s.id, "make a file", PermissionChoice::Yes).await;
    let failed = frames.iter().any(|f| {
        matches!(
            &f.event,
            Event::StepCompleted {
                status: StepStatus::Error,
                summary,
                ..
            } if summary.contains("isn't connected")
        )
    });
    assert!(failed, "the step says the PC isn't connected");
    assert!(
        !std::path::Path::new(&ws).join("made.txt").exists(),
        "nothing is written on this computer instead"
    );
}
