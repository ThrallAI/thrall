//! Named constants for `thrall-tui`. Every limit, timeout and repeated string lives here.

use std::time::Duration;

/// How often the screen refreshes while Thrall works (spinner, elapsed time).
pub const TICK: Duration = Duration::from_millis(100);

/// How long the input thread waits for a key before checking whether to stop.
pub const INPUT_POLL: Duration = Duration::from_millis(50);

/// An Enter this soon after another key is part of a paste: it becomes a newline, not a send.
pub const PASTE_ENTER_WINDOW: Duration = Duration::from_millis(8);

/// Tallest the input box grows, in text lines.
pub const INPUT_MAX_LINES: u16 = 8;

/// Most tasks shown in the task panel.
pub const TASKS_MAX_SHOWN: usize = 8;

/// Most slash-command suggestions shown at once.
pub const SUGGESTIONS_MAX: usize = 8;

/// Sessions listed by the resume picker.
pub const PICKER_SESSIONS: u32 = 30;

/// How long a request may wait for the engine's reply.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How this client introduces itself to the engine.
pub const CLIENT_DISPLAY_NAME: &str = "Thrall Terminal";

/// Output lines of a running step shown while it's collapsed.
pub const STEP_TAIL_LINES: usize = 3;

/// Output and diff lines shown for an expanded step.
pub const STEP_EXPANDED_LINES: usize = 200;

/// Lines moved by one PageUp / PageDown.
pub const PAGE_SCROLL_LINES: usize = 10;

/// Spinner frames.
pub const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Spinner frames for consoles that can't show symbols.
pub const SPINNER_ASCII: &[&str] = &["|", "/", "-", "\\"];

/// Set to 1 or 0 to force plain ASCII drawing on or off.
pub const ASCII_ENV_VAR: &str = "THRALL_ASCII";

/// Keys this soon after the previous one are typing, not an answer to a card.
pub const CARD_TYPING_GAP: Duration = Duration::from_millis(250);

/// Keys this soon after a card appears were meant for something else.
pub const CARD_SETTLE: Duration = Duration::from_millis(400);

/// Most diff lines shown in an edit's permission card.
pub const PERMISSION_DIFF_MAX_LINES: usize = 24;

/// Most lines of a permission request's summary shown before it's cut.
pub const PERMISSION_SUMMARY_MAX_LINES: u16 = 6;

/// Shown in the empty input box.
pub const INPUT_PLACEHOLDER: &str = "Message Thrall · / for commands";

/// Shown when the person chooses to type their answers to a question card.
pub const QUESTION_TYPE_HINT: &str = "Type your answers in the message box, one per question in order, separated by |, then press Enter.";

/// Put before a step a helper took, so its steps read as the helper's own.
pub const HELPER_STEP_MARK: &str = "  > ";

/// `answered_by` on a question card nobody answered in time.
pub const QUESTION_TIMEOUT: &str = "timeout";

/// Key hints under the input box.
pub const HINTS: &str =
    "Enter send · Shift+Enter or Ctrl+J new line · Ctrl+O steps · PgUp/PgDn scroll · Ctrl+C quit";

/// Commands the terminal handles itself (the engine's come from `command.list`).
pub const CLIENT_COMMANDS: &[(&str, &str)] = &[
    ("home", "Go to Home, your own conversation with Thrall"),
    ("new", "Start a new session"),
    ("resume", "Pick an earlier session to continue"),
    ("retry", "Run your last message again"),
    ("edit", "Replace your last message: /edit <new message>"),
    ("rewind", "Go back to before a message: /rewind lists them"),
    ("forget", "Forget a fact Thrall remembered: /forget <id>"),
    ("expand", "Show or hide step details (Ctrl+O)"),
    ("quit", "Leave Thrall"),
];

/// How many recent messages `/rewind` lists.
pub const REWIND_LIST_MAX: usize = 10;

/// `/rewind N files`: put back only files.
pub const REWIND_FILES: &str = "files";

/// `/rewind N chat`: take back only the conversation.
pub const REWIND_CHAT: &str = "chat";

/// `/rewind N both` (the default): files and conversation.
pub const REWIND_BOTH: &str = "both";

/// Characters of each message `/rewind` shows.
pub const REWIND_LINE_CHARS: usize = 70;
