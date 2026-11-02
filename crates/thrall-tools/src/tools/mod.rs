//! The core tools.

mod archive;
mod audio;
mod browse;
mod calc;
mod clipboard;
mod control;
mod documents;
pub(crate) mod files;
mod images;
mod mcp;
mod office;
pub mod schedule;
mod search;
mod sessions;
mod shell;
mod system;
mod web;
mod work;
mod writing;

use std::sync::Arc;

pub use archive::Archive;
pub use audio::{
    Captions, DownloadVideo, SPEAK, Speak, TRANSCRIBE, Transcribe, VIDEO_TRANSCRIPT,
    VideoTranscript, captions_to_text, clock, download, fetch_captions, ffmpeg, split_recording,
};
pub use browse::BrowserTool;
pub use calc::Calc;
pub use clipboard::Clipboard;
pub use control::{
    ASK_USER, AgentResult, AskUser, DELEGATE, Delegate, EXIT_PLAN_MODE, EXPERIENCE_NOTE,
    ExitPlanMode, ExperienceNote, PROPOSE_SKILL, ProposeSkill, Restart, USE_SKILL, UseSkill,
    find_skill, skill_text,
};
pub use documents::ReadDocument;
pub use files::{EditFile, ListDir, ReadFile, WriteFile};
pub use images::{MAKE_IMAGE, MAKE_VIDEO, MakeImage, MakeVideo, VIEW_IMAGE, ViewImage};
pub use mcp::{MCP_CALL, MCP_SEARCH, McpCall, McpSearch};
pub use office::{EditPdf, EditSpreadsheet, EditWord, MakePdf};
pub use schedule::{Schedule, ScheduleList, ScheduleRemove};
pub use search::{Glob, Grep};
pub use sessions::{SessionRead, SessionsList};
pub use shell::{Git, Shell, ShellOutput, ShellStop, shell_command};
pub use system::SystemInfo;
pub use web::{WebFetch, WebSearch};
pub use work::{MEMORY_SAVE, MemorySave, MemorySearch, TodoWrite};
pub use writing::{STYLE_CHECK, StyleCheck};

use crate::tool::Tool;

/// Every core tool, in the order the model sees them.
pub fn core() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(ReadFile),
        Arc::new(ReadDocument),
        Arc::new(EditSpreadsheet),
        Arc::new(EditWord),
        Arc::new(EditPdf),
        Arc::new(MakePdf),
        Arc::new(Archive),
        Arc::new(StyleCheck),
        Arc::new(WriteFile),
        Arc::new(EditFile),
        Arc::new(ListDir),
        Arc::new(Glob),
        Arc::new(Grep),
        Arc::new(Calc),
        Arc::new(Shell),
        Arc::new(ShellOutput),
        Arc::new(ShellStop),
        Arc::new(Git),
        Arc::new(WebFetch),
        Arc::new(WebSearch),
        Arc::new(BrowserTool),
        Arc::new(MakeImage),
        Arc::new(MakeVideo),
        Arc::new(ViewImage),
        Arc::new(Speak),
        Arc::new(Transcribe),
        Arc::new(DownloadVideo),
        Arc::new(VideoTranscript),
        Arc::new(Clipboard),
        Arc::new(SystemInfo),
        Arc::new(TodoWrite),
        Arc::new(MemorySave),
        Arc::new(MemorySearch),
        Arc::new(SessionsList),
        Arc::new(SessionRead),
        Arc::new(UseSkill),
        Arc::new(ProposeSkill),
        Arc::new(Delegate),
        Arc::new(AgentResult),
        Arc::new(Schedule),
        Arc::new(ScheduleList),
        Arc::new(ScheduleRemove),
        Arc::new(ExitPlanMode),
        Arc::new(AskUser),
        Arc::new(ExperienceNote),
        Arc::new(Restart),
    ]
}

/// Tools a helper may not use: no helpers of its own, no plans, restarts or schedules.
pub const NOT_FOR_HELPERS: &[&str] = &[
    DELEGATE,
    "agent_result",
    EXIT_PLAN_MODE,
    // Helpers decide for themselves; only Thrall asks the person.
    ASK_USER,
    // Lessons are Thrall's own, noted in the person's conversation.
    EXPERIENCE_NOTE,
    "thrall_restart",
    "schedule",
    "schedule_remove",
    "todo_write",
    // Skills are offered to the person by Thrall itself.
    PROPOSE_SKILL,
    // Pictures cost money; only Thrall itself asks for them.
    MAKE_IMAGE,
    MAKE_VIDEO,
];
