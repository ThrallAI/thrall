//! The versioned client protocol (see `docs/protocol.md`): every message a client and the engine
//! exchange.

pub mod constants;
pub mod events;
pub mod requests;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use events::{
    Diff, DiffKind, ErrorCode, Event, EventFrame, FileRef, NoticeLevel, PermissionChoice,
    QuestionItem, Risk, Role, StepStatus, TaskItem, TaskStatus, TurnStatus, Usage,
};
pub use requests::{
    AppRelease, ClientInfo, ClientKind, CommandItem, DeviceInfo, EngineInfo, FactInfo, FolderEntry,
    GroupInfo, HelperTypeInfo, KeyStatus, LessonInfo, Limits, LocalFound, ModeLimit, PersonInfo,
    PersonalityPreset, PhoneInfo, RemoteInfo, Reply, ReplyFrame, Request, RequestFrame,
    RewindAction, RewindFile, SessionInfo, SessionState, SettingValue, SkillDetail, SkillInfo,
    UserInfo,
};

/// Whether this build can talk to a peer speaking protocol major `version`.
pub fn supports(version: u32) -> bool {
    (constants::OLDEST_SUPPORTED_VERSION..=constants::PROTOCOL_VERSION).contains(&version)
}

/// Any frame the engine sends: a reply to a request (it has `re`) or a session event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ServerFrame {
    /// Answers a request.
    Reply(ReplyFrame),
    /// Something that happened in a session.
    Event(EventFrame),
}

/// Every frame in both directions, as one schema root (for generating client types).
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Protocol {
    /// Frames clients send.
    pub client: RequestFrame,
    /// Frames the engine sends.
    pub server: ServerFrame,
}

/// The JSON Schema of the whole protocol, pretty-printed. TypeScript and Kotlin types are
/// generated from it; `docs/protocol.schema.json` holds the committed copy.
pub fn schema_json() -> String {
    let schema = schemars::schema_for!(Protocol);
    let mut text = serde_json::to_string_pretty(&schema).unwrap_or_default();
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supports_current_version() {
        assert!(supports(constants::PROTOCOL_VERSION));
    }

    #[test]
    fn rejects_unknown_versions() {
        assert!(!supports(0));
        assert!(!supports(constants::PROTOCOL_VERSION + 1));
    }

    fn round_trips<T: Serialize + for<'de> Deserialize<'de>>(text: &str) {
        let frame: T = serde_json::from_str(text).unwrap();
        let again = serde_json::to_string(&frame).unwrap();
        let original: serde_json::Value = serde_json::from_str(text).unwrap();
        let reparsed: serde_json::Value = serde_json::from_str(&again).unwrap();
        assert_eq!(original, reparsed, "changed after a round trip: {text}");
    }

    /// Frames copied from `docs/protocol.md` §15 must parse and re-serialise unchanged.
    #[test]
    fn spec_examples_round_trip() {
        let events = [
            r#"{"v":1,"session":"s_1","seq":41,"at":0,"type":"turn.started","turn":"t_9"}"#,
            r#"{"v":1,"session":"s_1","seq":42,"at":0,"type":"message.started","turn":"t_9","message":"m_20","role":"assistant"}"#,
            r#"{"v":1,"session":"s_1","at":0,"type":"message.delta","message":"m_20","text":"I'll remove build/."}"#,
            r#"{"v":1,"session":"s_1","seq":44,"at":0,"type":"message.completed","message":"m_20","text":"I'll remove build/."}"#,
            r#"{"v":1,"session":"s_1","seq":46,"at":0,"type":"permission.request","request":"r_2","turn":"t_9","step":"p_3","tool":"fs_delete","summary":"Delete build/ (214 files)","risk":"medium","choices":["yes","no","always"]}"#,
            r#"{"v":1,"session":"s_1","seq":47,"at":0,"type":"permission.resolved","request":"r_2","choice":"yes","answered_by":"tui"}"#,
            r#"{"v":1,"session":"s_1","seq":49,"at":0,"type":"turn.completed","turn":"t_9","status":"done","usage":{"input_tokens":1840,"output_tokens":96,"cost_usd":0.0031}}"#,
            r#"{"v":1,"session":"s_1","seq":50,"at":0,"type":"error","code":"provider_unavailable","message":"OpenRouter isn't responding.","retry":true}"#,
        ];
        for text in events {
            round_trips::<EventFrame>(text);
        }
        let requests = [
            r#"{"v":1,"id":"c-1","type":"hello","protocols":[1],"client":{"kind":"tui","name":"Thrall Terminal","version":"0.1.0"},"token":"t"}"#,
            r#"{"v":1,"id":"c-4","type":"turn.send","session":"s_1","text":"delete the build folder"}"#,
            r#"{"v":1,"id":"c-5","type":"permission.answer","request":"r_2","choice":"yes"}"#,
            r#"{"v":1,"id":"c-6","type":"session.open","session":"s_1","after_seq":40}"#,
            r#"{"v":1,"id":"c-7","type":"plan.decide","plan":"p_1","approve":true,"auto_edits":true}"#,
            r#"{"v":1,"id":"c-8","type":"command.run","session":"s_1","name":"model","args":"x/y"}"#,
        ];
        for text in requests {
            round_trips::<RequestFrame>(text);
        }
        let replies = [
            r#"{"v":1,"re":"c-4","type":"ack"}"#,
            r#"{"v":1,"re":"c-9","type":"error","code":"already_answered","message":"Someone else answered first.","retry":false}"#,
            r#"{"v":1,"re":"c-1","type":"welcome","protocol":1,"engine":{"version":"0.1.0","model":"x/y","edition":"home"},"user":{"id":"u","name":"Owner","role":"owner"},"capabilities":["streaming"],"limits":{"frame_bytes":1048576}}"#,
            r#"{"v":1,"re":"c-6","type":"session.opened","session":"s_1","title":"t","mode":"ask","state":"idle","last_seq":49,"updated":0,"cost_usd":0.0031}"#,
        ];
        for text in replies {
            round_trips::<ReplyFrame>(text);
        }
    }

    #[test]
    fn server_frames_tell_replies_from_events() {
        let reply =
            r#"{"v":1,"re":"c-1","type":"error","code":"busy","message":"m","retry":false}"#;
        let event = r#"{"v":1,"session":"s","at":0,"type":"error","code":"busy","message":"m","retry":false}"#;
        assert!(matches!(
            serde_json::from_str(reply).unwrap(),
            ServerFrame::Reply(_)
        ));
        assert!(matches!(
            serde_json::from_str(event).unwrap(),
            ServerFrame::Event(_)
        ));
    }

    #[test]
    fn deltas_and_status_are_live_only() {
        let delta = Event::MessageDelta {
            message: "m".into(),
            text: "x".into(),
        };
        assert!(delta.is_live_only());
        let status = Event::Status {
            state: SessionState::Working,
            model: "m".into(),
        };
        assert!(status.is_live_only());
        assert!(
            !Event::TurnStarted {
                turn: "t".into(),
                scheduled: false
            }
            .is_live_only()
        );
    }

    #[test]
    fn type_names_match_serde_tags() {
        let event = Event::Notice {
            level: NoticeLevel::Info,
            text: "hi".into(),
        };
        assert_eq!(
            serde_json::to_value(&event).unwrap()["type"],
            event.type_name()
        );
        let request = Request::TurnInterrupt {
            session: "s".into(),
        };
        assert_eq!(
            serde_json::to_value(&request).unwrap()["type"],
            request.type_name()
        );
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let text = r#"{"v":1,"session":"s","seq":1,"at":0,"type":"turn.started","turn":"t","future_field":true}"#;
        assert!(serde_json::from_str::<EventFrame>(text).is_ok());
    }

    /// The committed schema must match the types. To refresh it after changing them, run the
    /// tests with `THRALL_UPDATE_SCHEMA=1`.
    #[test]
    fn committed_schema_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(constants::SCHEMA_FILE);
        let fresh = schema_json();
        if std::env::var_os(constants::UPDATE_SCHEMA_ENV).is_some() {
            std::fs::write(&path, &fresh).unwrap();
            return;
        }
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            committed == fresh,
            "docs/protocol.schema.json is out of date: run the tests with {}=1",
            constants::UPDATE_SCHEMA_ENV
        );
    }
}
