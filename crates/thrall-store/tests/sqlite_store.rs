//! The SQLite store, end to end, against an in-memory database.
// Integration tests are their own crate, so the in-tests unwrap exemption doesn't reach helpers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use thrall_store::{
    Checkpoint, Device, NewEvent, NewMessage, NewSession, PersonTerms, SqliteStore, Store,
    StoreError, TurnEnd, UsageRecord,
};

const ME: &str = "u_me";
const OTHER: &str = "u_other";

async fn store_with_session() -> (SqliteStore, String) {
    let store = SqliteStore::open_in_memory().await.unwrap();
    store.ensure_user(ME, "Me", 1).await.unwrap();
    store.ensure_user(OTHER, "Other", 1).await.unwrap();
    let session = store
        .create_session(NewSession {
            id: "s_1".into(),
            user_id: ME.into(),
            workspace: None,
            at: 10,
        })
        .await
        .unwrap();
    (store, session.id)
}

#[tokio::test]
async fn people_have_a_role_and_a_plan() {
    let (store, _) = store_with_session().await;
    // New people are members on the free plan.
    let me = store.get_person(ME).await.unwrap().unwrap();
    assert_eq!(
        (me.name.as_str(), me.role.as_str(), me.plan.as_str()),
        ("Me", "member", "free")
    );
    store.set_role(ME, "owner").await.unwrap();
    assert_eq!(store.get_person(ME).await.unwrap().unwrap().role, "owner");
    assert_eq!(
        store.get_person(OTHER).await.unwrap().unwrap().role,
        "member"
    );
    assert!(store.get_person("u_nobody").await.unwrap().is_none());
    // Adding someone again keeps who they are.
    store.ensure_user(ME, "Renamed", 2).await.unwrap();
    let me = store.get_person(ME).await.unwrap().unwrap();
    assert_eq!((me.name.as_str(), me.role.as_str()), ("Me", "owner"));
    let ids: Vec<String> = store
        .list_people()
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, vec![ME, OTHER]);
}

#[tokio::test]
async fn removing_a_person_keeps_their_things() {
    let (store, session) = store_with_session().await;
    assert!(store.rename_person(ME, "Me Too").await.unwrap());
    assert!(!store.rename_person("u_nobody", "X").await.unwrap());
    assert!(store.remove_person(ME, 5).await.unwrap());
    // Once only; a removed person can't be renamed and drops out of the list.
    assert!(!store.remove_person(ME, 6).await.unwrap());
    assert!(!store.rename_person(ME, "Back").await.unwrap());
    let me = store.get_person(ME).await.unwrap().unwrap();
    assert_eq!((me.name.as_str(), me.removed_at), ("Me Too", Some(5)));
    let ids: Vec<String> = store
        .list_people()
        .await
        .unwrap()
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, vec![OTHER]);
    // Their session is still there.
    assert!(store.get_session(ME, &session).await.unwrap().is_some());
}

#[tokio::test]
async fn a_person_has_terms() {
    let (store, _) = store_with_session().await;
    let me = store.get_person(ME).await.unwrap().unwrap();
    assert_eq!(
        (me.own_key, me.model, me.monthly_limit_micros),
        (false, None, None),
        "nothing set: the owner's key and model, no limit"
    );
    let terms = PersonTerms {
        own_key: true,
        model: Some("openai/gpt-5-mini".into()),
        monthly_limit_micros: Some(5_000_000),
    };
    assert!(store.set_person_terms(ME, &terms).await.unwrap());
    let me = store.get_person(ME).await.unwrap().unwrap();
    assert_eq!(
        PersonTerms {
            own_key: me.own_key,
            model: me.model,
            monthly_limit_micros: me.monthly_limit_micros,
        },
        terms
    );
    assert!(!store.set_person_terms("u_nobody", &terms).await.unwrap());
    store.remove_person(ME, 5).await.unwrap();
    assert!(!store.set_person_terms(ME, &terms).await.unwrap());
}

#[tokio::test]
async fn devices_sign_in_by_token_hash_and_sign_out() {
    let (store, _) = store_with_session().await;
    let device = |id: &str, at: i64| Device {
        id: id.into(),
        user_id: ME.into(),
        name: "Edge on Windows".into(),
        created_at: at,
        last_seen_at: None,
    };
    store.add_device(&device("dv_1", 1), "hash1").await.unwrap();
    store.add_device(&device("dv_2", 2), "hash2").await.unwrap();
    // Using a token marks the device seen.
    let used = store.use_device("hash1", 50).await.unwrap().unwrap();
    assert_eq!((used.id.as_str(), used.last_seen_at), ("dv_1", Some(50)));
    assert!(store.use_device("nope", 50).await.unwrap().is_none());
    let ids = |list: Vec<Device>| list.into_iter().map(|d| d.id).collect::<Vec<_>>();
    assert_eq!(
        ids(store.list_devices(ME).await.unwrap()),
        vec!["dv_2", "dv_1"]
    );
    // Only its own person can sign a device out, once.
    assert!(!store.remove_device(OTHER, "dv_1", 60).await.unwrap());
    assert!(store.remove_device(ME, "dv_1", 60).await.unwrap());
    assert!(!store.remove_device(ME, "dv_1", 61).await.unwrap());
    assert!(!store.device_signed_in("dv_1").await.unwrap());
    assert!(store.use_device("hash1", 70).await.unwrap().is_none());
    // Removing a person signs out the rest.
    store.remove_devices_of(ME, 80).await.unwrap();
    assert!(store.list_devices(ME).await.unwrap().is_empty());
    assert!(store.use_device("hash2", 90).await.unwrap().is_none());
}

fn event(session: &str, kind: &str, at: i64) -> NewEvent {
    NewEvent {
        user_id: ME.into(),
        session_id: session.into(),
        turn_id: None,
        kind: kind.into(),
        payload: format!("{{\"type\":\"{kind}\"}}"),
        at,
    }
}

#[tokio::test]
async fn archived_sessions_leave_the_list_and_can_come_back() {
    let (store, id) = store_with_session().await;
    store.set_session_archived(ME, &id, Some(50)).await.unwrap();
    assert!(store.list_sessions(ME, 10).await.unwrap().is_empty());
    let archived = store.list_archived_sessions(ME, 10).await.unwrap();
    assert_eq!(archived.len(), 1);
    assert!(
        store
            .list_archived_sessions(OTHER, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        store.set_session_archived(OTHER, &id, None).await,
        Err(StoreError::NotFound(_))
    ));
    store.set_session_archived(ME, &id, None).await.unwrap();
    assert_eq!(store.list_sessions(ME, 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn home_pages_stay_out_of_the_session_lists() {
    let (store, normal) = store_with_session().await;
    for (id, at) in [("s_h1", 20), ("s_h2", 30)] {
        store
            .create_session(NewSession {
                id: id.into(),
                user_id: ME.into(),
                workspace: None,
                at,
            })
            .await
            .unwrap();
        store.set_session_home(ME, id, "2026-10-16").await.unwrap();
    }
    let listed = store.list_sessions(ME, 10).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, normal);
    assert!(listed[0].home_day.is_none());
    let home = store.home_sessions(ME).await.unwrap();
    assert_eq!(
        home.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        vec!["s_h2", "s_h1"],
        "newest first"
    );
    assert_eq!(home[0].home_day.as_deref(), Some("2026-10-16"));
    assert!(store.home_sessions(OTHER).await.unwrap().is_empty());
    assert!(matches!(
        store.set_session_home(OTHER, "s_h1", "2026-10-17").await,
        Err(StoreError::NotFound(_))
    ));
    store
        .set_session_archived(ME, "s_h1", Some(40))
        .await
        .unwrap();
    assert!(
        store
            .list_archived_sessions(ME, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn pinned_sessions_list_first() {
    let (store, first) = store_with_session().await;
    store
        .create_session(NewSession {
            id: "s_2".into(),
            user_id: ME.into(),
            workspace: None,
            at: 20,
        })
        .await
        .unwrap();
    // The newer session lists first until the older one is pinned.
    assert_eq!(store.list_sessions(ME, 10).await.unwrap()[0].id, "s_2");
    store
        .set_session_pinned(ME, &first, Some(30))
        .await
        .unwrap();
    let list = store.list_sessions(ME, 10).await.unwrap();
    assert_eq!(list[0].id, first);
    assert_eq!(list[0].pinned_at, Some(30));
    store.set_session_pinned(ME, &first, None).await.unwrap();
    assert_eq!(store.list_sessions(ME, 10).await.unwrap()[0].id, "s_2");
}

#[tokio::test]
async fn groups_hold_sessions_and_deleting_one_keeps_them() {
    let (store, id) = store_with_session().await;
    let a = store.create_group(ME, "g_a", "Clients", 1).await.unwrap();
    let b = store.create_group(ME, "g_b", "Ideas", 2).await.unwrap();
    assert_eq!((a.position, b.position), (0, 1));

    store.set_session_group(ME, &id, Some("g_b")).await.unwrap();
    assert_eq!(
        store
            .get_session(ME, &id)
            .await
            .unwrap()
            .unwrap()
            .group_id
            .as_deref(),
        Some("g_b")
    );
    // Someone else's group can't be used.
    store.create_group(OTHER, "g_x", "Theirs", 3).await.unwrap();
    assert!(matches!(
        store.set_session_group(ME, &id, Some("g_x")).await,
        Err(StoreError::NotFound(_))
    ));

    store.rename_group(ME, "g_a", "Work").await.unwrap();
    store
        .reorder_groups(ME, &["g_b".to_owned(), "g_a".to_owned()])
        .await
        .unwrap();
    let groups = store.list_groups(ME).await.unwrap();
    let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, vec!["Ideas", "Work"]);

    store.delete_group(ME, "g_b").await.unwrap();
    assert_eq!(store.list_groups(ME).await.unwrap().len(), 1);
    let session = store.get_session(ME, &id).await.unwrap().unwrap();
    assert_eq!(session.group_id, None, "the session stays, ungrouped");
}

#[tokio::test]
async fn a_retracted_turn_leaves_the_context_but_stays_stored() {
    let (store, id) = store_with_session().await;
    for (msg, turn, at) in [("m1", "t1", 1), ("m2", "t2", 2)] {
        store
            .add_message(NewMessage {
                id: msg.into(),
                user_id: ME.into(),
                session_id: id.clone(),
                turn_id: Some(turn.into()),
                role: "user".into(),
                content: "{}".into(),
                tokens: 0,
                at,
            })
            .await
            .unwrap();
    }
    store
        .append_event(NewEvent {
            user_id: ME.into(),
            session_id: id.clone(),
            turn_id: Some("t2".into()),
            kind: "turn.started".into(),
            payload: "{}".into(),
            at: 3,
        })
        .await
        .unwrap();
    // The user's message, Thrall's reply, then a message typed while it worked.
    let turn_events = [
        (
            "message.started",
            r#"{"type":"message.started","turn":"t2","message":"m2","role":"user"}"#,
        ),
        (
            "message.completed",
            r#"{"type":"message.completed","message":"m2","text":"what I typed"}"#,
        ),
        (
            "message.started",
            r#"{"type":"message.started","turn":"t2","message":"m3","role":"assistant"}"#,
        ),
        (
            "message.completed",
            r#"{"type":"message.completed","message":"m3","text":"working on it"}"#,
        ),
        (
            "message.started",
            r#"{"type":"message.started","turn":"t2","message":"m4","role":"user"}"#,
        ),
        (
            "message.completed",
            r#"{"type":"message.completed","message":"m4","text":"also dark mode"}"#,
        ),
    ];
    for (kind, payload) in turn_events {
        store
            .append_event(NewEvent {
                user_id: ME.into(),
                session_id: id.clone(),
                turn_id: Some("t2".into()),
                kind: kind.into(),
                payload: payload.into(),
                at: 4,
            })
            .await
            .unwrap();
    }
    assert_eq!(
        store.last_turn(ME, &id).await.unwrap(),
        Some((
            "t2".to_owned(),
            vec!["what I typed".to_owned(), "also dark mode".to_owned()]
        ))
    );
    store.retract_turn(ME, &id, "t2", 5).await.unwrap();
    let context = store.context_messages(ME, &id).await.unwrap();
    assert_eq!(context.len(), 1);
    assert_eq!(context[0].id, "m1");
}

#[tokio::test]
async fn new_session_starts_empty() {
    let (store, id) = store_with_session().await;
    let session = store.get_session(ME, &id).await.unwrap().unwrap();
    assert_eq!(session.last_seq, 0);
    assert_eq!(session.mode, "ask");
    assert_eq!(session.title, "");
}

#[tokio::test]
async fn events_get_increasing_seq_and_replay_after() {
    let (store, id) = store_with_session().await;
    for (i, kind) in ["turn.started", "message.completed", "turn.completed"]
        .iter()
        .enumerate()
    {
        let seq = store
            .append_event(event(&id, kind, 20 + i as i64))
            .await
            .unwrap();
        assert_eq!(seq, i as u64 + 1);
    }
    let all = store.events_after(ME, &id, 0, 100).await.unwrap();
    assert_eq!(all.len(), 3);
    let tail = store.events_after(ME, &id, 1, 100).await.unwrap();
    assert_eq!(tail.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![2, 3]);
    assert_eq!(tail[0].kind, "message.completed");
    let session = store.get_session(ME, &id).await.unwrap().unwrap();
    assert_eq!(session.last_seq, 3);
    assert_eq!(session.updated_at, 22);
}

#[tokio::test]
async fn other_users_see_nothing() {
    let (store, id) = store_with_session().await;
    store
        .append_event(event(&id, "turn.started", 20))
        .await
        .unwrap();
    assert!(store.get_session(OTHER, &id).await.unwrap().is_none());
    assert!(
        store
            .events_after(OTHER, &id, 0, 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(store.list_sessions(OTHER, 10).await.unwrap().is_empty());
    let mut theirs = event(&id, "turn.started", 21);
    theirs.user_id = OTHER.into();
    assert!(matches!(
        store.append_event(theirs).await,
        Err(StoreError::NotFound("session"))
    ));
}

#[tokio::test]
async fn sessions_list_newest_activity_first() {
    let (store, first) = store_with_session().await;
    store
        .create_session(NewSession {
            id: "s_2".into(),
            user_id: ME.into(),
            workspace: Some("C:/work".into()),
            at: 15,
        })
        .await
        .unwrap();
    store
        .append_event(event(&first, "turn.started", 50))
        .await
        .unwrap();
    let ids: Vec<String> = store
        .list_sessions(ME, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|s| s.id)
        .collect();
    assert_eq!(ids, vec!["s_1".to_string(), "s_2".to_string()]);
}

#[tokio::test]
async fn messages_come_back_in_order() {
    let (store, id) = store_with_session().await;
    for (n, (role, at)) in [("user", 30), ("assistant", 31)].iter().enumerate() {
        store
            .add_message(NewMessage {
                id: format!("m_{n}"),
                user_id: ME.into(),
                session_id: id.clone(),
                turn_id: None,
                role: (*role).into(),
                content: "{\"text\":\"hi\"}".into(),
                tokens: 2,
                at: *at,
            })
            .await
            .unwrap();
    }
    let roles: Vec<String> = store
        .context_messages(ME, &id)
        .await
        .unwrap()
        .into_iter()
        .map(|m| m.role)
        .collect();
    assert_eq!(roles, vec!["user", "assistant"]);
}

#[tokio::test]
async fn turns_and_usage_add_up() {
    let (store, id) = store_with_session().await;
    store.start_turn(ME, &id, "t_1", 40).await.unwrap();
    store
        .finish_turn(TurnEnd {
            user_id: ME.into(),
            turn_id: "t_1".into(),
            status: "done".into(),
            at: 41,
            input_tokens: 100,
            output_tokens: 20,
            cost_usd: Some(0.0031),
        })
        .await
        .unwrap();
    for n in 0..2 {
        store
            .record_usage(UsageRecord {
                id: format!("u_{n}"),
                user_id: ME.into(),
                session_id: Some(id.clone()),
                turn_id: Some("t_1".into()),
                provider: "openrouter".into(),
                model: "test/model".into(),
                role: "chat".into(),
                input_tokens: 100,
                cached_tokens: 0,
                output_tokens: 20,
                cost_usd: Some(0.0031),
                at: 41,
            })
            .await
            .unwrap();
    }
    let totals = store.usage_totals(ME, Some(&id)).await.unwrap();
    assert_eq!(totals.input_tokens, 200);
    assert_eq!(totals.output_tokens, 40);
    assert!((totals.cost_usd - 0.0062).abs() < 1e-9);
    assert_eq!(
        store.usage_totals(OTHER, None).await.unwrap().input_tokens,
        0
    );
}

#[tokio::test]
async fn spending_counts_from_a_moment() {
    let (store, id) = store_with_session().await;
    let spends = [
        (ME, 10, Some(0.5)),
        (ME, 20, Some(0.25)),
        (ME, 30, None),
        (OTHER, 20, Some(9.0)),
    ];
    for (n, (who, at, cost)) in spends.into_iter().enumerate() {
        store
            .record_usage(UsageRecord {
                id: format!("u_{n}"),
                user_id: who.into(),
                session_id: Some(id.clone()),
                turn_id: None,
                provider: "openrouter".into(),
                model: "m".into(),
                role: "chat".into(),
                input_tokens: 1,
                cached_tokens: 0,
                output_tokens: 1,
                cost_usd: cost,
                at,
            })
            .await
            .unwrap();
    }
    assert!((store.spent_since(ME, 0).await.unwrap() - 0.75).abs() < 1e-9);
    assert!((store.spent_since(ME, 20).await.unwrap() - 0.25).abs() < 1e-9);
    assert_eq!(store.spent_since(ME, 31).await.unwrap(), 0.0);
}

#[tokio::test]
async fn a_turns_usage_rows_keep_who_made_each_call_and_what_was_cached() {
    let (store, id) = store_with_session().await;
    for (n, (model, role, cached)) in [("big/thinker", "thinker", 0), ("cheap/doer", "doer", 900)]
        .into_iter()
        .enumerate()
    {
        store
            .record_usage(UsageRecord {
                id: format!("u_{n}"),
                user_id: ME.into(),
                session_id: Some(id.clone()),
                turn_id: Some("t_split".into()),
                provider: "openrouter".into(),
                model: model.into(),
                role: role.into(),
                input_tokens: 1000,
                cached_tokens: cached,
                output_tokens: 50,
                cost_usd: Some(0.01),
                at: 30 + n as i64,
            })
            .await
            .unwrap();
    }
    let rows = store.turn_usage(ME, "t_split").await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        (rows[0].model.as_str(), rows[0].role.as_str()),
        ("big/thinker", "thinker")
    );
    assert_eq!(rows[1].role, "doer");
    assert_eq!(rows[1].cached_tokens, 900);
    assert_eq!(rows[1].cost_usd, Some(0.01));
    assert!(
        store
            .turn_usage("u_other", "t_split")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn deleting_a_session_removes_its_events_but_keeps_usage() {
    let (store, id) = store_with_session().await;
    store
        .append_event(event(&id, "turn.started", 20))
        .await
        .unwrap();
    store
        .record_usage(UsageRecord {
            id: "u_1".into(),
            user_id: ME.into(),
            session_id: Some(id.clone()),
            turn_id: None,
            provider: "openrouter".into(),
            model: "m".into(),
            role: "chat".into(),
            input_tokens: 5,
            cached_tokens: 0,
            output_tokens: 1,
            cost_usd: None,
            at: 20,
        })
        .await
        .unwrap();
    store.delete_session(ME, &id).await.unwrap();
    assert!(store.get_session(ME, &id).await.unwrap().is_none());
    assert!(
        store
            .events_after(ME, &id, 0, 100)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.usage_totals(ME, None).await.unwrap().input_tokens, 5);
}

#[tokio::test]
async fn steps_are_recorded_and_finished() {
    let (store, session) = store_with_session().await;
    store.start_turn(ME, &session, "t_1", 20).await.unwrap();
    store
        .start_step(thrall_store::NewStep {
            id: "p_1".into(),
            user_id: ME.into(),
            turn_id: "t_1".into(),
            tool: "read_file".into(),
            title: "Reading a.txt".into(),
            args: "{}".into(),
            at: 21,
        })
        .await
        .unwrap();
    store
        .finish_step(ME, "p_1", "ok", "12 lines", 22)
        .await
        .unwrap();
    assert!(matches!(
        store.finish_step(OTHER, "p_1", "ok", "", 23).await,
        Err(StoreError::NotFound(_))
    ));
}

#[tokio::test]
async fn task_list_is_replaced_whole() {
    let (store, session) = store_with_session().await;
    let task = |id: &str, status: &str| thrall_store::TaskRow {
        id: id.into(),
        text: format!("task {id}"),
        status: status.into(),
    };
    store
        .set_tasks(
            ME,
            &session,
            &[task("a", "pending"), task("b", "pending")],
            30,
        )
        .await
        .unwrap();
    store
        .set_tasks(
            ME,
            &session,
            &[task("b", "done"), task("c", "in_progress")],
            31,
        )
        .await
        .unwrap();
    let tasks = store.tasks(ME, &session).await.unwrap();
    let ids: Vec<_> = tasks.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["b", "c"]);
    assert_eq!(tasks[0].status, "done");
    assert!(store.tasks(OTHER, &session).await.unwrap().is_empty());
}

#[tokio::test]
async fn knowledge_search_matches_all_words_for_owner_only() {
    let (store, _) = store_with_session().await;
    store
        .save_knowledge(ME, "k1", "editor", "Prefers VS Code with a dark theme", 40)
        .await
        .unwrap();
    store
        .save_knowledge(ME, "k2", "pets", "Has a dog called Rex", 41)
        .await
        .unwrap();
    let hits = store.search_knowledge(ME, "dark code", 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, "k1");
    assert!(
        store
            .search_knowledge(ME, "100%", 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .search_knowledge(OTHER, "dog", 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn grants_cover_only_their_session_and_scope() {
    let (store, session) = store_with_session().await;
    assert!(!store.has_grant(ME, &session, "shell:cargo").await.unwrap());
    store
        .add_grant(ME, &session, "g1", "shell", "shell:cargo", 50)
        .await
        .unwrap();
    assert!(store.has_grant(ME, &session, "shell:cargo").await.unwrap());
    assert!(!store.has_grant(ME, &session, "shell:rm").await.unwrap());
    assert!(
        !store
            .has_grant(OTHER, &session, "shell:cargo")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn sessions_remember_their_mode() {
    let (store, session) = store_with_session().await;
    store.set_session_mode(ME, &session, "plan").await.unwrap();
    assert_eq!(
        store.get_session(ME, &session).await.unwrap().unwrap().mode,
        "plan"
    );
    assert!(
        store
            .set_session_mode(OTHER, &session, "ask")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn audit_entries_are_written() {
    let (store, _) = store_with_session().await;
    store
        .append_audit(thrall_store::AuditEntry {
            id: "a1".into(),
            user_id: ME.into(),
            at: 60,
            gate: "tool".into(),
            action: "shell".into(),
            decision: "approved".into(),
            detail: "{}".into(),
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn memory_text_search_finds_facts_and_episodes_but_not_superseded_facts() {
    use thrall_store::{Episode, MemoryKind};
    let (store, session) = store_with_session().await;
    store
        .save_knowledge(ME, "k_old", "editor", "Uses Vim for everything", 10)
        .await
        .unwrap();
    store
        .save_knowledge(ME, "k_new", "editor", "Switched to VS Code", 20)
        .await
        .unwrap();
    store
        .supersede_knowledge(ME, "k_old", "k_new", 21)
        .await
        .unwrap();
    store
        .add_episode(
            ME,
            Episode {
                id: "e1".into(),
                session_id: Some(session.clone()),
                summary: "Fixing the Rust build".into(),
                content: "We fixed a borrow checker error in main.rs".into(),
                created_at: 30,
            },
        )
        .await
        .unwrap();

    let hits = store.search_memory_text(ME, "vim code", 10).await.unwrap();
    let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
    assert!(ids.contains(&"k_new") && !ids.contains(&"k_old"), "{ids:?}");
    let hits = store
        .search_memory_text(ME, "borrow checker", 10)
        .await
        .unwrap();
    assert_eq!(hits[0].kind, MemoryKind::Episode);
    // Odd characters can't break the query, and other users see nothing.
    assert!(
        store
            .search_memory_text(ME, "\"* OR (", 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .search_memory_text(OTHER, "borrow", 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        store.knowledge_by_topic(ME, "EDITOR").await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn vectors_round_trip_and_compaction_hides_messages() {
    use thrall_store::{MemoryKind, NewMessage};
    let (store, session) = store_with_session().await;
    store.save_knowledge(ME, "k1", "t", "c", 1).await.unwrap();
    store
        .put_embedding(ME, MemoryKind::Knowledge, "k1", "m", &[0.5, -1.25, 3.0])
        .await
        .unwrap();
    let v = store.embeddings(ME, "m").await.unwrap();
    assert_eq!(v[0].vector, vec![0.5, -1.25, 3.0]);
    assert!(
        store
            .embeddings(ME, "other-model")
            .await
            .unwrap()
            .is_empty()
    );

    for (i, id) in ["m1", "m2", "m3"].iter().enumerate() {
        store
            .add_message(NewMessage {
                id: (*id).into(),
                user_id: ME.into(),
                session_id: session.clone(),
                turn_id: None,
                role: "user".into(),
                content: "{}".into(),
                tokens: 0,
                at: 100 + i as i64,
            })
            .await
            .unwrap();
    }
    store
        .compact_messages(ME, &session, &["m1".into(), "m2".into()])
        .await
        .unwrap();
    let left = store.context_messages(ME, &session).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, "m3");
}

fn checkpoint(session: &str, turn: &str, path: &str, content: &[u8], at: i64) -> Checkpoint {
    Checkpoint {
        session_id: session.into(),
        turn_id: turn.into(),
        user_id: ME.into(),
        path: path.into(),
        existed: true,
        content: Some(content.to_vec()),
        too_big: false,
        after_hash: None,
        at,
    }
}

#[tokio::test]
async fn checkpoints_keep_the_first_copy_and_follow_turn_order() {
    let (store, id) = store_with_session().await;
    for (turn, at) in [("t_a", 1), ("t_b", 2), ("t_c", 3)] {
        store.start_turn(ME, &id, turn, at).await.unwrap();
    }
    assert_eq!(
        store.turns_from(ME, &id, "t_b").await.unwrap(),
        vec!["t_b".to_owned(), "t_c".to_owned()]
    );
    assert!(
        store
            .turns_from(ME, &id, "t_missing")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .turns_from(OTHER, &id, "t_b")
            .await
            .unwrap()
            .is_empty()
    );

    // Only the first copy per turn and file is kept: that's the content before the turn.
    assert!(
        store
            .save_checkpoint(checkpoint(&id, "t_b", "/f", b"before b", 10))
            .await
            .unwrap()
    );
    assert!(
        !store
            .save_checkpoint(checkpoint(&id, "t_b", "/f", b"mid b", 11))
            .await
            .unwrap()
    );
    store
        .save_checkpoint(checkpoint(&id, "t_a", "/f", b"before a", 5))
        .await
        .unwrap();
    store
        .save_checkpoint(checkpoint(&id, "t_c", "/g", b"before c", 20))
        .await
        .unwrap();
    store
        .set_checkpoint_after(ME, &id, "t_b", "/f", "hash-b")
        .await
        .unwrap();

    let from_b = store.checkpoints_from(ME, &id, "t_b").await.unwrap();
    let seen: Vec<(&str, &str)> = from_b
        .iter()
        .map(|c| (c.turn_id.as_str(), c.path.as_str()))
        .collect();
    assert_eq!(seen, vec![("t_b", "/f"), ("t_c", "/g")]);
    assert_eq!(from_b[0].content.as_deref(), Some(&b"before b"[..]));
    assert_eq!(from_b[0].after_hash.as_deref(), Some("hash-b"));
    assert_eq!(
        store.checkpoints_from(ME, &id, "t_a").await.unwrap().len(),
        3
    );
    assert!(
        store
            .checkpoints_from(OTHER, &id, "t_a")
            .await
            .unwrap()
            .is_empty()
    );

    // They go with their session.
    store.delete_session(ME, &id).await.unwrap();
    assert!(
        store
            .checkpoints_from(ME, &id, "t_a")
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_forgotten_fact_is_no_longer_listed_or_found() {
    let (store, _) = store_with_session().await;
    store
        .save_knowledge(ME, "k_1", "editor", "uses VS Code", 1)
        .await
        .unwrap();
    store
        .save_knowledge(ME, "k_2", "shell", "PowerShell", 2)
        .await
        .unwrap();
    assert_eq!(store.search_knowledge(ME, "", 10).await.unwrap().len(), 2);
    assert!(!store.forget_knowledge(OTHER, "k_1", 3).await.unwrap());
    assert!(store.forget_knowledge(ME, "k_1", 3).await.unwrap());
    assert!(
        !store.forget_knowledge(ME, "k_1", 4).await.unwrap(),
        "already forgotten"
    );
    let left = store.search_knowledge(ME, "", 10).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].id, "k_2");
    assert!(
        store
            .search_memory_text(ME, "VS Code", 10)
            .await
            .unwrap()
            .is_empty()
    );
}
