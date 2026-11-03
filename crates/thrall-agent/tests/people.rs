//! A person's terms in whole turns: whose key their turns use, their own model, and their monthly
//! spending limit.
// Integration tests are their own crate, so the in-tests unwrap exemption doesn't reach helpers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::{Arc, Mutex};

use futures::{StreamExt, stream};
use thrall_agent::people::OwnKeys;
use thrall_agent::{AgentError, AppState, session, turn};
use thrall_core::Config;
use thrall_core::ids::now_ms;
use thrall_llm::{ChatMessage, ChatRequest, ChatRole, LlmError, LlmProvider, ReplyStream};
use thrall_llm::{StreamEvent, TokenUsage};
use thrall_protocol::{Event, EventFrame, NoticeLevel, TurnStatus};
use thrall_store::{PersonTerms, SqliteStore, Store, UsageRecord};
use tokio::sync::mpsc;

const OWNER: &str = "u_owner";
const SIS: &str = "pe_sis";

/// Answers with its own name, and remembers the models it was asked for.
struct Named {
    name: &'static str,
    models: Mutex<Vec<String>>,
}

impl Named {
    fn new(name: &'static str) -> Arc<Self> {
        Arc::new(Self {
            name,
            models: Mutex::new(Vec::new()),
        })
    }
}

impl LlmProvider for Named {
    fn name(&self) -> &str {
        self.name
    }

    fn stream(&self, request: ChatRequest) -> ReplyStream {
        self.models.lock().unwrap().push(request.model);
        stream::iter(vec![
            Ok(StreamEvent::Text(self.name.to_owned())),
            Ok(StreamEvent::Finished(Some("stop".into()))),
            Ok(StreamEvent::Usage(TokenUsage {
                input_tokens: 1,
                cached_tokens: 0,
                output_tokens: 1,
                cost_usd: Some(0.000_001),
            })),
        ])
        .boxed()
    }
}

/// One person's own key, in memory.
struct TestKeys {
    key: Mutex<Option<String>>,
    provider: Arc<Named>,
}

impl OwnKeys for TestKeys {
    fn has(&self, person: &str) -> bool {
        person == SIS && self.key.lock().unwrap().is_some()
    }
    fn provider(&self, person: &str) -> Option<Arc<dyn LlmProvider>> {
        self.has(person)
            .then(|| self.provider.clone() as Arc<dyn LlmProvider>)
    }
    fn set(&self, _person: &str, key: &str) -> Result<(), String> {
        *self.key.lock().unwrap() = Some(key.to_owned());
        Ok(())
    }
    fn clear(&self, _person: &str) -> Result<(), String> {
        *self.key.lock().unwrap() = None;
        Ok(())
    }
}

async fn turn_for(state: &AppState) -> Result<(turn::TurnSummary, Vec<EventFrame>), AgentError> {
    let s = session::create(state, None).await?;
    let (tx, mut rx) = mpsc::channel::<EventFrame>(1024);
    let collect = async {
        let mut frames = Vec::new();
        while let Some(frame) = rx.recv().await {
            frames.push(frame);
        }
        frames
    };
    let go = async move {
        let summary = turn::run(state, &s.id, "hi", &tx).await;
        drop(tx);
        summary
    };
    let (summary, frames) = tokio::join!(go, collect);
    Ok((summary?, frames))
}

fn warnings(frames: &[EventFrame]) -> Vec<String> {
    frames
        .iter()
        .filter_map(|f| match &f.event {
            Event::Notice {
                level: NoticeLevel::Warning,
                text,
            } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

async fn spend(store: &dyn Store, person: &str, usd: f64, n: u32) {
    store
        .record_usage(UsageRecord {
            id: format!("u_spend_{n}"),
            user_id: person.into(),
            session_id: None,
            turn_id: None,
            provider: "openrouter".into(),
            model: "m".into(),
            role: "chat".into(),
            input_tokens: 0,
            cached_tokens: 0,
            output_tokens: 0,
            cost_usd: Some(usd),
            at: now_ms(),
        })
        .await
        .unwrap();
}

fn set(state: &AppState, terms: PersonTerms) {
    state.set_terms(SIS, terms);
}

#[tokio::test]
async fn a_person_uses_their_own_key_model_and_limit() {
    let store = SqliteStore::open_in_memory().await.unwrap();
    store.ensure_user(OWNER, "Owner", 0).await.unwrap();
    store.set_role(OWNER, "owner").await.unwrap();
    store.ensure_user(SIS, "Sis", 1).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(store);
    let owners = Named::new("owner's key");
    let hers = Named::new("her key");
    let keys = Arc::new(TestKeys {
        key: Mutex::new(None),
        provider: hers.clone(),
    });
    let engine = AppState::new(
        Config::default(),
        store.clone(),
        owners.clone(),
        OWNER.into(),
    )
    .unwrap()
    .with_own_keys(keys.clone());
    engine.load_terms().await.unwrap();
    let sis = engine.for_user(SIS);

    // Nothing set: the owner's key.
    let (done, _) = turn_for(&sis).await.unwrap();
    assert_eq!(done.reply, "owner's key");

    // Her own key, before it's set: the turn is refused, and nothing goes on the owner's key.
    set(
        &engine,
        PersonTerms {
            own_key: true,
            ..PersonTerms::default()
        },
    );
    let before = owners.models.lock().unwrap().len();
    let refused = turn_for(&sis).await.unwrap_err();
    assert!(
        matches!(refused, AgentError::OwnKeyMissing(_)),
        "{refused:?}"
    );
    assert_eq!(owners.models.lock().unwrap().len(), before);
    let mut direct = sis.llm.stream(ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage::new(ChatRole::User, "hi")],
        max_tokens: 10,
        tools: Vec::new(),
        thinking: None,
    });
    assert!(matches!(
        direct.next().await,
        Some(Err(LlmError::KeyMissing { .. }))
    ));

    // Set: her turns go on her key, the owner's still on his.
    keys.set(SIS, "sk-hers").unwrap();
    let (done, _) = turn_for(&sis).await.unwrap();
    assert_eq!(done.reply, "her key");
    let (done, _) = turn_for(&engine).await.unwrap();
    assert_eq!(done.reply, "owner's key");

    // Her own model, for her only.
    set(
        &engine,
        PersonTerms {
            own_key: true,
            model: Some("small/model".into()),
            ..PersonTerms::default()
        },
    );
    hers.models.lock().unwrap().clear();
    turn_for(&sis).await.unwrap();
    assert!(
        hers.models
            .lock()
            .unwrap()
            .contains(&"small/model".to_owned())
    );
    assert_eq!(sis.model(), "small/model");
    assert_ne!(engine.model(), "small/model");

    // A $1 limit: a warning from 80%, refused at it.
    set(
        &engine,
        PersonTerms {
            own_key: false,
            model: None,
            monthly_limit_micros: Some(1_000_000),
        },
    );
    spend(store.as_ref(), SIS, 0.85, 1).await;
    let (done, frames) = turn_for(&sis).await.unwrap();
    assert_eq!(done.status, TurnStatus::Done);
    let warned = warnings(&frames);
    assert!(
        warned
            .iter()
            .any(|w| w.contains("of this month's $1.00 limit")),
        "{warned:?}"
    );
    spend(store.as_ref(), SIS, 0.2, 2).await;
    let refused = turn_for(&sis).await.unwrap_err();
    assert!(
        matches!(&refused, AgentError::OverLimit(why) if why.contains("$1.00")),
        "{refused:?}"
    );
    // Anything else that costs money stops too, mid-turn included.
    let mut direct = sis.llm.stream(ChatRequest {
        model: "m".into(),
        messages: vec![ChatMessage::new(ChatRole::User, "hi")],
        max_tokens: 10,
        tools: Vec::new(),
        thinking: None,
    });
    assert!(matches!(
        direct.next().await,
        Some(Err(LlmError::OverLimit { .. }))
    ));
    // The owner isn't held to her limit.
    assert!(turn_for(&engine).await.is_ok());
}

#[tokio::test]
async fn members_get_only_the_services_shared_with_everyone() {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::open_in_memory().await.unwrap());
    let server = |shared| thrall_core::config::McpServerConfig {
        command: "thrall-no-such-program".into(),
        shared,
        ..Default::default()
    };
    let has_mcp_tools = |state: &AppState| state.tools.iter().any(|t| t.name() == "mcp_search");
    let mut config = Config::default();
    config.mcp.servers.insert("github".into(), server(false));
    config.mcp.servers.insert("youtube".into(), server(true));
    let engine =
        AppState::new(config.clone(), store.clone(), Named::new("o"), OWNER.into()).unwrap();
    let owner = engine.for_user(OWNER);
    assert_eq!(owner.mcp.names(), vec!["github", "youtube"]);
    let sis = engine.for_user(SIS);
    assert_eq!(sis.mcp.names(), vec!["youtube"]);
    assert!(has_mcp_tools(&sis));
    let err = sis
        .mcp
        .call("github", "x", serde_json::Map::new())
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("no MCP server called github"),
        "{err}"
    );
    // Her settings screen lists only what's shared with her.
    let thrall_protocol::Reply::SettingsShown { config: shown, .. } =
        thrall_agent::settings::shown(&sis, false).await
    else {
        panic!("not settings");
    };
    let listed: Vec<&String> = shown["mcp"]["servers"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    assert_eq!(listed, vec!["youtube"]);

    // Nothing shared: she isn't offered the MCP tools at all; the owner still is.
    config.mcp.servers.insert("youtube".into(), server(false));
    let engine = AppState::new(config, store, Named::new("o"), OWNER.into()).unwrap();
    assert!(engine.for_user(SIS).mcp.is_empty() && !has_mcp_tools(&engine.for_user(SIS)));
    assert!(has_mcp_tools(&engine.for_user(OWNER)));
}
