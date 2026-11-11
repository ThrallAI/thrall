//! Named constants for `thrall-tools`. Every limit, timeout and repeated string lives here.

/// Lines `read_file` returns unless asked for fewer.
pub const READ_DEFAULT_LINES: usize = 2000;
/// Files bigger than this aren't read whole.
pub const READ_MAX_BYTES: u64 = 10 * 1024 * 1024;
/// Most characters any one tool result sends to the model.
pub const RESULT_MAX_CHARS: usize = 60_000;
/// After a non-zero exit code, for the model: the person sees a warning, not a failure, so the
/// reply has to say whether the command did what was asked.
pub const NONZERO_EXIT_NOTE: &str = " (it ran to the end; the output below may still be what was \
     wanted: say in your reply whether it did the job)";
/// After a command that ended while something it started kept running (it held the output open),
/// when Thrall couldn't find what it was.
pub const LEFT_RUNNING_NOTE: &str = "The command ended, but something it started is still running \
     (it kept the output open). Thrall isn't tracking it and can't stop it: note its process ID \
     and port, and stop it when you're done, unless the person wants it left running.";
/// The same, when Thrall found the programs and keeps track of them. `{programs}` (names with
/// process IDs) and `{id}` are filled in.
pub const LEFT_TRACKED_NOTE: &str = "The command ended, but what it started is still running: \
     {programs}. Thrall keeps track of it as {id}: read its latest output with shell_output, and \
     stop it with shell_stop when you're done, unless the person wants it left running.";
/// Finds the programs a command started that are still running after it ended (Windows: the
/// shell's gone, so they're no longer under it). `{pid}` is the shell's process ID and `{ms}` when
/// it started (Unix milliseconds), so a later program reusing the ID isn't counted.
pub const LEFT_BEHIND_QUERY: &str = "Get-CimInstance Win32_Process -Filter 'ParentProcessId={pid}' \
     | Where-Object { $_.CreationDate -ge [DateTimeOffset]::FromUnixTimeMilliseconds({ms}).LocalDateTime } \
     | ForEach-Object { \"$($_.ProcessId) $($_.Name)\" }";
/// Allowance for clock granularity when matching programs started since the shell did.
pub const LEFT_BEHIND_MARGIN_MS: u128 = 1000;
/// After a command was stopped (timed out or interrupted): what was stopped with it.
pub const STOPPED_NOTE: &str = "it and the programs under it were stopped";
/// Added to [`STOPPED_NOTE`] only when the command starts a program on its own, which may have
/// escaped the stop.
pub const STOPPED_DETACHED_NOTE: &str = ". A program it started on its own (like with \
     Start-Process) may still be running: check before going on";
/// Ways of starting a program that keeps running after the shell command ends, which Thrall
/// doesn't track: matched in lower case where a command begins, so `Get-Help Start-Process`
/// doesn't count.
pub const UNTRACKED_STARTS: &[&str] = &[
    "start-process",
    "saps ",
    "start ",
    "start-job",
    "sajb ",
    "nohup ",
    "setsid ",
    "cmd /c start",
];
/// Programs that run without a window of output and outlive the command that starts them,
/// matched by name where a command begins (as `pythonw app.py`, not `Get-Process pythonw`).
pub const UNTRACKED_PROGRAMS: &[&str] = &["pythonw"];
/// What separates the commands on one shell line, for finding where each begins.
pub const COMMAND_SEPARATORS: &[char] = &[';', '|', '&', '\n', '(', '{'];
/// After an `mcp_search` by key words that found nothing, when some servers weren't searched
/// because they aren't started. `{servers}` is filled in.
pub const MCP_NOT_STARTED_NOTE: &str = " Only servers already started were searched. Not started: \
     {servers}. If one of these is clearly what's needed, search again with its name to start it; \
     don't start them to hunt for something.";
/// Characters of a single line shown before it's cut.
pub const LINE_MAX_CHARS: usize = 2000;

/// Most changes `edit_file` makes in one call.
pub const EDIT_MAX_CHANGES: usize = 50;
/// Most changed lines (with a little context) `edit_file` shows back to the model.
pub const EDIT_SHOWN_MAX_LINES: usize = 80;

/// Most entries `list_dir` shows.
pub const LIST_MAX_ENTRIES: usize = 500;
/// Most paths `glob` returns.
pub const GLOB_MAX_RESULTS: usize = 500;
/// Most matching lines `grep` returns.
pub const GREP_MAX_MATCHES: usize = 300;
/// Files bigger than this are skipped by `grep`.
pub const GREP_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// Most characters `calc` accepts in one call.
pub const CALC_MAX_INPUT_CHARS: usize = 8000;
/// Most lines (statements) `calc` works through in one call.
pub const CALC_MAX_STATEMENTS: usize = 200;
/// Deepest nesting of brackets and operators `calc` follows.
pub const CALC_MAX_DEPTH: usize = 64;
/// Significant digits `calc` shows.
pub const CALC_SIGNIFICANT_DIGITS: usize = 12;

/// Shell commands time out after this unless the model asks for longer.
pub const SHELL_DEFAULT_TIMEOUT_SECS: u64 = 120;
/// The longest a shell command may run.
pub const SHELL_MAX_TIMEOUT_SECS: u64 = 600;
/// Output kept for a program left running in the background (the newest part).
pub const BACKGROUND_OUTPUT_MAX_CHARS: usize = 20_000;
/// Most of a background program's output `shell_output` returns at once.
pub const BACKGROUND_READ_MAX_CHARS: usize = 6_000;
/// Programs one person may leave running in the background at once.
pub const BACKGROUND_MAX_PER_PERSON: usize = 5;
/// With keep_running and nothing to wait for, a program still going after this long is kept.
pub const KEEP_RUNNING_SETTLE_SECS: u64 = 3;
/// After a command ends, how long its output may take to finish arriving. Longer means something
/// it started (like Start-Process) is still running and holding the output open.
pub const OUTPUT_DRAIN_MS: u64 = 2000;
/// Set to "1" for programs Thrall runs, so Python prints as it goes instead of holding output back.
pub const PYTHON_UNBUFFERED: &str = "PYTHONUNBUFFERED";
/// Git commands time out after this.
pub const GIT_TIMEOUT_SECS: u64 = 60;
/// Most characters of live output sent to clients per step (the model still gets the result).
pub const LIVE_OUTPUT_MAX_CHARS: usize = 20_000;

/// Web requests time out after this.
pub const WEB_TIMEOUT_SECS: u64 = 30;
/// Most bytes downloaded by `web_fetch`.
pub const WEB_MAX_BYTES: usize = 3 * 1024 * 1024;
/// Text width used when turning web pages into text.
pub const WEB_TEXT_WIDTH: usize = 100;
/// Search results returned by `web_search`.
pub const SEARCH_MAX_RESULTS: usize = 8;
/// DuckDuckGo's plain-HTML search endpoint (no key needed).
pub const SEARCH_URL: &str = "https://html.duckduckgo.com/html/";
/// Sent with web requests.
pub const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) Thrall/0.1";

/// A project's own Thrall folder (skills, settings) inside its workspace.
pub const PROJECT_DIR: &str = ".thrall";
/// Skills folder name (in the data folder and in a project's `.thrall`).
pub const SKILLS_DIR: &str = "skills";
/// The file that makes a folder a skill.
pub const SKILL_FILE: &str = "SKILL.md";
/// Most characters of a skill's instructions loaded at once.
pub const SKILL_MAX_CHARS: usize = 40_000;
/// Front matter `run:` value that makes a skill run in a helper.
pub const SKILL_RUN_HELPER: &str = "helper";
/// Longest skill folder name.
pub const SKILL_NAME_MAX_CHARS: usize = 48;
/// Longest description a proposed skill may have.
pub const SKILL_DESCRIPTION_MAX_CHARS: usize = 300;
/// `learned_from` for a skill the person taught (or wrote themselves).
pub const SKILL_SOURCE_USER: &str = "user";
/// `learned_from` for a skill Thrall worked out in its own work.
pub const SKILL_SOURCE_MY_WORK: &str = "my_work";
/// `learned_from` for a skill made from the person's files.
pub const SKILL_SOURCE_FILES: &str = "files";

/// Helper types folder name (in the data folder and in a project's `.thrall`).
pub const AGENTS_DIR: &str = "agents";
/// A helper type's file extension.
pub const AGENT_FILE_EXT: &str = "md";

/// Thrall's own helper types: (name, what it's for, read only, instructions).
pub const AGENT_TYPES: &[(&str, &str, bool, &str)] = &[
    (
        "researcher",
        "Finds things out: searches the web, the codebase or documents and reports back with sources",
        true,
        "You are a researcher. Find out what the task asks and report it plainly. Search broadly \
         first, then read the best sources in full. Say where each fact came from (a link, or a \
         file and line). Separate what you found from what you think. If sources disagree, say so. \
         Don't change anything.",
    ),
    (
        "coder",
        "Writes and changes code or files for one well-defined piece of work",
        false,
        "You are a coder. Make the change the task describes, and only that. Read the code around \
         it first and match its style. Keep changes small. After changing something, check it: \
         build, run the tests or read the result back. Finish with what you changed (files) and \
         how you checked it.",
    ),
    (
        "reviewer",
        "Reviews work for mistakes: bugs, security problems, missing cases, things that don't match the request",
        true,
        "You are a reviewer. Look for real problems in the work the task names: bugs, security \
         holes, missing cases, and places it doesn't do what was asked. For each, say where it is, \
         what goes wrong and how you know. Put the most serious first. Don't report style \
         preferences as problems, and don't change anything. If you find nothing wrong, say so.",
    ),
    (
        "planner",
        "Works out how to do something bigger: the steps, their order, and the risks",
        true,
        "You are a planner. Work out how to do what the task asks: look at what's there now, then \
         give the steps in order, what each changes, what could go wrong, and how to check each \
         one. Pick one approach and say why; mention another only when it's a real choice. Don't \
         change anything.",
    ),
];

/// The shortest repeat a scheduled job may have.
pub const MIN_INTERVAL_SECS: i64 = 5 * 60;
/// Title prefix of the session a scheduled job runs in.
pub const SCHEDULED_TITLE_PREFIX: &str = "Scheduled: ";

/// Facts returned by `memory_search`.
pub const MEMORY_SEARCH_LIMIT: u32 = 20;

/// Git subcommands that only read, so they run without asking.
pub const GIT_READ_ONLY: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "blame",
    "rev-parse",
    "ls-files",
    "shortlog",
    "describe",
];
/// Pieces of a shell command that make it high-risk (lower-case match). High-risk commands ask in
/// every mode, Full auto too, and are never covered by "always". Deleting whole folders outside
/// the session's, stopping programs by name and changing the registry or system files are
/// high-risk too, worked out from the command's words.
pub const SHELL_RISKY: &[&str] = &[
    "format ",
    "format-volume",
    "clear-disk",
    "initialize-disk",
    "diskpart",
    "mkfs",
    "bcdedit",
    "shutdown",
    "restart-computer",
    "stop-computer",
    "set-executionpolicy",
    "git push --force",
    "git push -f",
    "git reset --hard",
    "git clean",
    "> /dev/",
    "dd if=",
];

/// Pieces of a shell command that need a look (lower-case match): installing and sending code
/// out. Like deleting and stopping a program, they ask unless the mode is Full auto.
pub const SHELL_CAREFUL: &[&str] = &[
    " install",
    "install-",
    "uninstall",
    "npm i ",
    "msiexec",
    "add-appxpackage",
    "git push",
];

/// Programs that delete files (lower-case, matched as whole words). A command using one says so at
/// the start of its permission card, so a delete chained onto something harmless isn't missed.
pub const SHELL_DELETERS: &[&str] = &["rm", "rmdir", "remove-item", "ri", "del", "erase", "rd"];

/// Words that make a delete take whole folders (with one of `SHELL_DELETERS`).
pub const SHELL_RECURSIVE: &[&str] = &["-r", "-rf", "-fr", "-recurse", "/s", "rmdir", "rd"];

/// Words that move a command to another folder, so what it deletes may be outside the session's.
pub const SHELL_MOVERS: &[&str] = &[
    "cd",
    "set-location",
    "sl",
    "pushd",
    "push-location",
    "chdir",
];

/// Programs that stop other programs (whole words), and the words that make them stop every
/// program of a name rather than one process.
pub const SHELL_STOPPERS: &[&str] = &[
    "stop-process",
    "spps",
    "kill",
    "taskkill",
    "pkill",
    "killall",
];
pub const SHELL_BY_NAME: &[&str] = &["-name", "-processname", "/im", "pkill", "killall"];

/// Words that change something on disk (with `>`), for spotting changes to the system's folders.
pub const SHELL_WRITERS: &[&str] = &[
    "rm",
    "rmdir",
    "remove-item",
    "ri",
    "del",
    "erase",
    "rd",
    "copy-item",
    "cp",
    "copy",
    "move-item",
    "mv",
    "move",
    "rename-item",
    "ren",
    "new-item",
    "set-content",
    "add-content",
    "out-file",
    "icacls",
    "takeown",
    "attrib",
];

/// How the registry is named in a command, and the commands that change it (lower case).
pub const REGISTRY_NAMES: &[&str] = &[
    "hklm:",
    "hkcu:",
    "hkcr:",
    "hku:",
    "hkcc:",
    "registry::",
    "hkey_",
];
pub const REGISTRY_WRITERS: &[&str] = &[
    "set-itemproperty",
    "new-itemproperty",
    "remove-itemproperty",
    "rename-itemproperty",
    "copy-itemproperty",
    "move-itemproperty",
    "set-item",
    "new-item",
    "remove-item",
];
pub const REG_EXE_WRITES: &[&str] = &[
    "reg add",
    "reg delete",
    "reg import",
    "reg copy",
    "reg restore",
    "reg load",
    "reg unload",
];

/// What starts the permission card of a command that deletes files, of another risky one, and of
/// one that changes the registry or system files.
pub const SHELL_DELETES_NOTE: &str = "Deletes files: ";
pub const SHELL_RISKY_NOTE: &str = "Risky: ";
pub const SHELL_SYSTEM_NOTE: &str = "Changes the system: ";

/// Why a change to the registry or system files can't go ahead yet: it needs its outcome, how to
/// undo it, and a backup that the undo names.
pub const SYSTEM_CHANGE_NEEDS: &str = "This changes the registry or system files, so it must be \
     reversible. First make a backup (reg export <key> <file>.reg, or copy the file somewhere), \
     then run it again with `outcome` (what will change and what that does) and `undo` (how to \
     put it back, naming the backup file).";

/// Characters that chain or pipe commands; such commands can't be covered by "always".
pub const SHELL_CHAINING: &[char] = &[';', '&', '|', '`', '$', '>', '<', '\n'];

/// Git arguments that make a command high-risk.
pub const GIT_RISKY: &[&str] = &[
    "--force",
    "push -f",
    "reset --hard",
    "clean",
    "branch -D",
    "push --delete",
];

/// Flags that make an otherwise read-only git subcommand write something.
pub const GIT_WRITE_FLAGS: &[&str] = &["--output", "-o"];

/// Most rows of one spreadsheet sheet turned into text.
pub const SHEET_MAX_ROWS: usize = 2000;

/// Most slides read from one deck.
pub const SLIDES_MAX: usize = 500;

/// Biggest document `read_document` opens, in bytes.
pub const DOCUMENT_MAX_BYTES: u64 = 50 * 1024 * 1024;

/// A PDF averaging fewer characters of text per page than this is treated as scanned (pictures of
/// pages), so the picture reader transcribes it.
pub const SCANNED_PDF_CHARS_PER_PAGE: usize = 200;
/// Longest run of glyphs in one PDF font-width range that's rewritten as a plain list before the
/// text is read (a font with a longer one is read as it is).
pub const PDF_WIDTH_RANGE_MAX: i64 = 4096;

/// Where a PDF stamp's text sits: points below the top edge (baseline), in the outer margin.
pub const STAMP_TOP_MARGIN: f32 = 15.0;

/// Where a PDF stamp's text sits: points above the bottom edge (baseline).
pub const STAMP_BOTTOM_MARGIN: f32 = 8.0;

/// Most characters of clipboard text returned to the model.
pub const CLIPBOARD_MAX_CHARS: usize = 20_000;

/// Saved clipboard snippets, in the data folder.
pub const SNIPPETS_FILE: &str = "snippets.json";

/// The browser's own profile (cookies, logins), in the data folder; never the user's everyday one.
pub const BROWSER_PROFILE_DIR: &str = "browser-profile";

/// Browser window size (also the page size for screenshots).
pub const BROWSER_WINDOW_WIDTH: u32 = 1280;
/// Browser window height.
pub const BROWSER_WINDOW_HEIGHT: u32 = 900;

/// Longest wait for a page to load or answer, in seconds.
pub const BROWSER_TIMEOUT_SECS: u64 = 30;

/// Longest Chrome may take to start, in seconds. The library's own 20 isn't enough on a slow or
/// busy PC (or several starting at once), the first start after an update especially.
pub const BROWSER_LAUNCH_SECS: u64 = 60;

/// Longest one browser action (click, fill, read, back, screenshot) may take in all, in seconds:
/// a page that stops answering fails the step instead of holding the turn.
pub const BROWSER_ACTION_SECS: u64 = 60;

/// Longest opening a page may take in all (load, settle, first look), in seconds.
pub const BROWSER_OPEN_SECS: u64 = 90;

/// Longest printing or previewing a PDF may take in all, in seconds.
pub const BROWSER_PRINT_SECS: u64 = 180;

/// Longest a page pop-up's message is kept, in characters.
pub const BROWSER_DIALOG_MAX_CHARS: usize = 300;

/// Pause after something that may load a page, so it settles before reading it.
pub const BROWSER_SETTLE_MS: u64 = 1200;

/// Linux and macOS: folders programs are installed in that an app's PATH may lack (Homebrew on
/// Apple silicon and Intel, then the usual local folder).
pub const EXTRA_PROGRAM_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin"];

/// macOS: the browsers Thrall can drive, in the order it tries them (inside an Applications folder).
pub const MAC_BROWSERS: &[&str] = &[
    "Google Chrome.app/Contents/MacOS/Google Chrome",
    "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "Chromium.app/Contents/MacOS/Chromium",
    "Brave Browser.app/Contents/MacOS/Brave Browser",
];

/// Linux: the browsers Thrall can drive, by the names distributions install them on the PATH, in
/// the order it tries them (Omarchy ships Chromium).
pub const LINUX_BROWSERS: &[&str] = &[
    "google-chrome",
    "google-chrome-stable",
    "microsoft-edge",
    "microsoft-edge-stable",
    "chromium",
    "chromium-browser",
    "brave",
    "brave-browser",
];

/// How often to check whether a page has finished loading, in milliseconds.
pub const BROWSER_POLL_MS: u64 = 100;

/// Longest a video download may take, in seconds.
pub const DOWNLOAD_TIMEOUT_SECS: u64 = 900;

/// Longest fetching a video's captions may take, in seconds.
pub const CAPTIONS_TIMEOUT_SECS: u64 = 120;

/// Captions are fetched into a temporary folder with this name inside the download folder.
pub const CAPTIONS_TEMP_PREFIX: &str = ".captions-";

/// A caption transcript starts a new paragraph, with its time, after this many seconds.
pub const CAPTION_PARAGRAPH_SECS: u64 = 30;

/// How many lines just written a caption line is checked against: automatic captions repeat
/// each line as the next one starts.
pub const CAPTION_REPEAT_WINDOW: usize = 2;

/// Long recordings are made small for listening: one channel at this sample rate (Hz)...
pub const LISTEN_SAMPLE_RATE: &str = "16000";

/// ...and this bitrate. Speech stays clear; an hour is about 14 MB.
pub const LISTEN_BITRATE: &str = "32k";

/// Longest making a recording small and cutting it into parts may take, in seconds.
pub const SPLIT_TIMEOUT_SECS: u64 = 900;

/// Retries when the search service throttles (answers with a bot check).
pub const SEARCH_RETRIES: u32 = 2;

/// First pause before a throttled search is retried, in milliseconds (grows each try).
pub const SEARCH_RETRY_MS: u64 = 2500;

/// Distance of the running header and footer from the page edge (1 cm).
pub const WORD_HEADER_GAP: i32 = 567;
/// Body text font (installed with Office on Windows and Mac).
pub const WORD_BODY_FONT: &str = "Calibri";
/// Code font.
pub const WORD_CODE_FONT: &str = "Consolas";
/// Text colour.
pub const WORD_INK: &str = "1C1B23";
/// Secondary text (header, footer, quotes, minor headings).
pub const WORD_MUTED: &str = "5F5B6E";
/// Accent: the title rule, sub-headings, links, the table header rule.
pub const WORD_ACCENT: &str = "5B3FD1";
/// Hairlines between table rows and under headings.
pub const WORD_LINE: &str = "E4E1EC";
/// Table header fill.
pub const WORD_SOFT: &str = "EFEAFD";
/// Code and alternate-row fill.
pub const WORD_TINT: &str = "F7F6FB";

/// Documents (PDF and Word), sized like the web app's chat: body text 14px, in points.
pub const LAYOUT_BODY_PT: f64 = 10.5;
/// `###` and smaller headings (15px).
pub const LAYOUT_SUBHEADING_PT: f64 = 11.25;
/// `##` headings (17px).
pub const LAYOUT_HEADING_PT: f64 = 12.75;
/// The title, the first `#` (20px).
pub const LAYOUT_TITLE_PT: f64 = 15.0;
/// Default margin on every side, in millimetres.
pub const LAYOUT_MARGIN_MM: f64 = 18.0;
/// Smallest size a document may ask for, in points.
pub const LAYOUT_MIN_PT: f64 = 5.0;
/// Largest size a document may ask for, in points.
pub const LAYOUT_MAX_PT: f64 = 72.0;
/// make_pdf max_pages: the smallest the whole document may be shrunk to.
pub const PDF_MIN_SCALE: f64 = 0.5;
/// make_pdf max_pages: how much each try shrinks by.
pub const PDF_SCALE_STEP: f64 = 0.05;
/// A title not sized on purpose is at least this much bigger than the headings.
pub const LAYOUT_TITLE_OVER_HEADING: f64 = 1.2;
/// make_pdf layouts: markdown laid out as a document, one picture per page, or a page Thrall
/// designs itself.
pub const PDF_LAYOUTS: &[&str] = &["document", "pictures", "custom"];
/// The pictures layout's margin when none is given, in millimetres.
pub const PICTURES_MARGIN_MM: f64 = 10.0;
/// The custom layout's margin when none is given (its own CSS sets the page), in millimetres.
pub const CUSTOM_MARGIN_MM: f64 = 0.0;
/// Room kept free inside each picture page, so rounding never pushes it onto a second sheet (mm).
pub const PICTURE_PAGE_SLACK_MM: f64 = 1.0;
/// make_pdf's preview picture (for checking a PDF by eye): at most this many pages of it,
pub const PDF_PREVIEW_MAX_PAGES: f64 = 8.0;
/// at this scale,
pub const PDF_PREVIEW_SCALE: f64 = 0.5;
/// as a JPEG of this quality.
pub const PDF_PREVIEW_QUALITY: i64 = 70;
/// Where previews are kept, in Thrall's data folder.
pub const PDF_PREVIEW_DIR: &str = "previews";
/// CSS pixels per inch.
pub const CSS_PX_PER_IN: f64 = 96.0;
/// A picture named in a page: `src="…"` or `src='…'`.
pub const PICTURE_SRC_PATTERN: &str = r#"(?i)(\bsrc\s*=\s*)(?:"([^"]*)"|'([^']*)')"#;
/// A picture named in CSS: `url(…)`, quoted or not.
pub const PICTURE_URL_PATTERN: &str = r#"(?i)\burl\(\s*(?:"([^"]*)"|'([^']*)'|([^)"'\s]+))\s*\)"#;
/// Most pictures one PDF may hold.
pub const PDF_MAX_PICTURES: usize = 200;
/// Largest picture make_pdf puts into a PDF.
pub const PDF_PICTURE_MAX_BYTES: u64 = 40 * 1024 * 1024;
/// Most picture bytes one PDF may hold.
pub const PDF_PICTURES_MAX_BYTES: u64 = 250 * 1024 * 1024;
/// Picture types make_pdf can put into a PDF: extension and media type.
pub const PDF_PICTURE_TYPES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("bmp", "image/bmp"),
    ("svg", "image/svg+xml"),
];

// ---------- style check (the humaniser's linter) ----------

/// The habits `style_check` looks for, from Wikipedia's "Signs of AI writing" guide. Each regex is
/// matched without regard to case; a hit that overlaps an earlier one is dropped.
pub const STYLE_PATTERNS: &[crate::style::Pattern] = &[
    crate::style::Pattern {
        label: "inflated significance",
        why: "puffs up importance instead of saying what happened",
        fix: "say plainly what it did or why it matters, with a fact",
        min_hits: 1,
        regexes: &[
            r"\b(?:is|was|are|were|stands as|stood as|serves as|served as|remains) an? (?:testament|reminder)\b",
            r"\ba (?:true |powerful |lasting )?testament to\b",
            r"\bpivotal (?:moment|role|point|step|turning point)\b",
            r"\bplay(?:s|ed|ing)? an? (?:crucial|pivotal|vital|key|significant|central|instrumental) role\b",
            r"\bunderscor(?:es|ed|ing) (?:its|the|their|his|her|this) (?:importance|significance)\b",
            r"\bsetting the stage for\b",
            r"\b(?:evolving|ever-evolving|ever-changing|shifting) landscape\b",
            r"\b(?:left|leaves|leaving) an? (?:lasting|indelible|enduring) (?:mark|impact|legacy)\b",
            r"\bindelible mark\b",
            r"\bmarks? an? (?:significant|major|pivotal|important) (?:milestone|shift|step|moment)\b",
            r"\bin today[’']?s (?:fast-paced|digital|modern|ever-changing) world\b",
        ],
    },
    crate::style::Pattern {
        label: "advert tone",
        why: "reads like a brochure",
        fix: "describe it with specifics, not praise",
        min_hits: 1,
        regexes: &[
            r"\bnestled\b",
            r"\bin the heart of\b",
            r"\bboasts?\b",
            r"\bvibrant\b",
            r"\bgroundbreaking\b",
            r"\brenowned\b",
            r"\bbreathtaking\b",
            r"\bmust-(?:see|visit|have)\b",
            r"\bdiverse (?:array|range)\b",
            r"\brich (?:history|heritage|tapestry|culture)\b",
            r"\bseamless(?:ly)?\b",
            r"\bcutting-edge\b",
            r"\bgame-?changer\b",
            r"\bunparalleled\b",
            r"\bworld-class\b",
            r"\bstate-of-the-art\b",
        ],
    },
    crate::style::Pattern {
        label: "-ing tail",
        why: "a tacked-on clause that claims meaning without adding any",
        fix: "end the sentence there; add a new one only if there's a real point",
        min_hits: 1,
        regexes: &[
            r",\s+(?:highlighting|underscoring|emphasi[sz]ing|showcasing|reflecting|symboli[sz]ing|demonstrating|illustrating|fostering|cementing|solidifying|contributing to|paving the way)\b",
        ],
    },
    crate::style::Pattern {
        label: "vague attribution",
        why: "claims backing it doesn't name",
        fix: "name the source, or drop the claim",
        min_hits: 1,
        regexes: &[
            r"\bexperts (?:say|argue|believe|agree|suggest|note)\b",
            r"\b(?:some|many) (?:critics|observers|experts|analysts|commentators) (?:argue|say|believe|note|have noted|suggest)\b",
            r"\bindustry reports\b",
            r"\bobservers have\b",
            r"\bstudies (?:show|have shown|suggest)\b",
            r"\bresearch (?:shows|suggests|has shown)\b",
            r"\bit is (?:widely|generally|often) (?:believed|recogni[sz]ed|acknowledged|accepted|considered)\b",
        ],
    },
    crate::style::Pattern {
        label: "stock ending",
        why: "a formula that hedges or repeats instead of finishing",
        fix: "stop once the point is made, or end on a concrete fact",
        min_hits: 1,
        regexes: &[
            r"\bdespite (?:these|its|their|the|such|some) challenges\b",
            r"\bin conclusion\b",
            r"\bin summary\b",
            r"\bto sum up\b",
            r"\bfuture outlook\b",
            r"\bas we look (?:to|toward|towards) the future\b",
            r"\btime will tell\b",
            r"\bthe future (?:looks|is) bright\b",
            r"\bremains to be seen\b",
        ],
    },
    crate::style::Pattern {
        label: "not-X-but-Y",
        why: "a stock contrast against something nobody claimed",
        fix: "say the real point directly",
        min_hits: 1,
        regexes: &[
            r"\bnot only\b[^.!?\n]{1,80}?\bbut also\b",
            r"\b(?:not|isn[’']?t|aren[’']?t) (?:just|only|merely|simply) (?:about )?[^.!?\n]{1,60}?[,;:—–]\s*(?:it[’']?s|it is|they[’']?re|but|rather)\b",
        ],
    },
    crate::style::Pattern {
        label: "stand-in verb",
        why: "a roundabout verb where \"is\" would do",
        fix: "use is, are or has",
        min_hits: 1,
        regexes: &[r"\b(?:serves|served|stands|stood|functions|acts) as (?:a|an|the)\b"],
    },
    crate::style::Pattern {
        label: "chatbot talk",
        why: "talk to a user that doesn't belong in finished writing",
        fix: "delete it",
        min_hits: 1,
        regexes: &[
            r"\bgreat question\b",
            r"\bi hope this helps\b",
            r"\blet me know if\b",
            r"\bfeel free to\b",
            r"\bhappy to help\b",
            r"\bi[’']?d be (?:happy|glad) to\b",
            r"\bas an ai\b",
            r"\bcertainly!",
            r"\babsolutely!",
            r"\bhere[’']?s an? (?:breakdown|rundown|quick overview|summary)\b",
            r"\bhope this (?:email|message) finds you well\b",
            r"\bwould you like me to\b",
        ],
    },
    crate::style::Pattern {
        label: "filler",
        why: "words that delay the point",
        fix: "cut them and start with the point",
        min_hits: 1,
        regexes: &[
            r"\bit(?:[’']s| is) worth noting\b",
            r"\bit(?:[’']s| is) important to (?:note|remember|understand)\b",
            r"\bit should be noted\b",
            r"\bneedless to say\b",
            r"\bat the end of the day\b",
            r"\bwhen it comes to\b",
        ],
    },
    crate::style::Pattern {
        label: "chatbot leftover",
        why: "markup a chatbot left behind",
        fix: "remove it",
        min_hits: 1,
        regexes: &[
            r"contentReference",
            r"oaicite",
            r"\bturn\d+search\d+\b",
            r"utm_source=(?:chatgpt|openai|copilot|perplexity)",
            r"\[cite:\s*\d+\]",
            r"\b(?:start|end)_span\b",
            r"\bgrok_card\b",
        ],
    },
    // Last, so a word inside a longer phrase above counts there instead.
    crate::style::Pattern {
        label: "overused word",
        why: "a word language models lean on; several together give the text away",
        fix: "use the plain word (look into, detailed, careful, show, use, strong, important)",
        min_hits: STYLE_VOCABULARY_MIN_HITS,
        regexes: &[
            r"\bdelv(?:e|es|ed|ing)\b",
            r"\btapestry\b",
            r"\bintricate(?:ly)?\b",
            r"\bmeticulous(?:ly)?\b",
            r"\bshowcas(?:e|es|ed|ing)\b",
            r"\bgarner(?:s|ed|ing)?\b",
            r"\binterplay\b",
            r"\bmultifaceted\b",
            r"\bleverag(?:e|es|ed|ing)\b",
            r"\brobust\b",
            r"\bcrucial\b",
            r"\bfoster(?:s|ed|ing)?\b",
            r"\bunderscor(?:e|es|ed|ing)\b",
            r"\brealm\b",
            r"\bembark(?:s|ed|ing)?\b",
            r"\bharness(?:es|ed|ing)?\b",
            r"\bparamount\b",
            r"\bbustling\b",
            r"\benduring\b",
            r"\bnavigat(?:e|es|ing) the\b",
            r"\badditionally\b",
        ],
    },
];
/// "Overused word" hits count only when there are at least this many: one is normal writing.
pub const STYLE_VOCABULARY_MIN_HITS: usize = 2;
/// Most hits a style report lists (the rest are counted).
pub const STYLE_MAX_HITS: usize = 25;
/// Sentence rhythm is judged only with at least this many sentences.
pub const STYLE_MIN_SENTENCES: usize = 6;
/// Sentence lengths that vary less than this (standard deviation ÷ mean) read as flat.
pub const STYLE_LOW_VARIATION: f64 = 0.35;
/// One first word starting at least this share of sentences is a repeated opener...
pub const STYLE_OPENER_SHARE: f64 = 0.3;
/// ...when it starts at least this many.
pub const STYLE_OPENER_MIN: usize = 3;
/// This many sentences or more opening with a linking word read as padding.
pub const STYLE_TRANSITIONS_MIN: usize = 3;
/// Linking words that open sentences.
pub const STYLE_TRANSITIONS: &[&str] = &[
    "additionally",
    "furthermore",
    "moreover",
    "overall",
    "ultimately",
    "consequently",
    "notably",
    "importantly",
];
/// Em dashes per 100 words above this (with at least `STYLE_MARKS_MIN`) are noted.
pub const STYLE_EM_DASHES_PER_100: f64 = 1.0;
/// Bold spans per 100 words above this (with at least `STYLE_MARKS_MIN`) are noted.
pub const STYLE_BOLD_PER_100: f64 = 2.0;
/// Fewest em dashes, bold spans, emoji-led lines or Title Case headings worth a note.
pub const STYLE_MARKS_MIN: usize = 3;
/// A heading with at least this many long words, all capitalised, is in Title Case.
pub const STYLE_TITLE_CASE_WORDS: usize = 3;
/// Words this short or shorter are left out of the Title Case test ("of", "and", "the").
pub const STYLE_SMALL_WORD_CHARS: usize = 3;
/// Biggest plain-text file `style_check` reads.
pub const STYLE_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// Detector-aware rewrites aim for at least this much sentence-length variation (human prose
/// usually lands around 0.5 to 0.8; model prose often under 0.4).
pub const STYLE_DETECTOR_VARIATION: f64 = 0.5;
/// Detector-aware: sentences this short or shorter count as short.
pub const STYLE_SHORT_SENTENCE_WORDS: usize = 8;
/// Detector-aware: sentences this long or longer count as long.
pub const STYLE_LONG_SENTENCE_WORDS: usize = 25;
/// Detector-aware: the fewest sentences rhythm is reported for (under `STYLE_MIN_SENTENCES` it's
/// marked as a rough guide).
pub const STYLE_DETECTOR_MIN_SENTENCES: usize = 3;

// ---------- other sessions (sessions_list, session_read) ----------

/// Sessions `sessions_list` shows unless asked for more.
pub const SESSIONS_LIST_DEFAULT: u32 = 30;
/// Most sessions `sessions_list` shows.
pub const SESSIONS_LIST_MAX: u32 = 200;
/// Recent events read to tell what a session is doing (working, waiting for the person, idle).
pub const SESSION_STATE_TAIL_EVENTS: u64 = 200;
/// Entries (messages and steps) `session_read` returns from the end unless asked for more.
pub const SESSION_READ_DEFAULT_LAST: usize = 40;
/// Most characters of one message `session_read` shows.
pub const SESSION_READ_ENTRY_MAX_CHARS: usize = 2_000;
/// Most characters `session_read` returns in all.
pub const SESSION_READ_MAX_CHARS: usize = 20_000;
/// Most matches a `session_read` search returns.
pub const SESSION_SEARCH_MAX_HITS: usize = 20;

// ---------- zip files (archive) ----------

/// Most bytes one unpack may write in all (a "zip bomb" is a small file that unpacks to far more).
pub const ARCHIVE_MAX_UNPACKED_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Most files one unpack may write.
pub const ARCHIVE_MAX_FILES: usize = 50_000;
/// Most files and folders a new zip may hold.
pub const ARCHIVE_MAX_ADDED: usize = 50_000;
/// Most entries listed (in a listing, a preview or a result) before "… and N more".
pub const ARCHIVE_LIST_ENTRIES: usize = 300;

/// Words in a learned skill that mean it deletes things (rated High).
pub const SKILL_RISK_DELETES: &[&str] = &[
    "rm -rf",
    "rm -r ",
    "remove-item",
    "del /",
    "rmdir",
    "rd /s",
    "--force",
    "push -f",
    "drop table",
    "format ",
    "delete ",
];

/// … that it sends or publishes things (rated High).
pub const SKILL_RISK_SENDS: &[&str] = &[
    "send email",
    "send an email",
    "smtp",
    "-method post",
    "curl -x post",
    "publish",
    "upload",
    "post to",
    "tweet",
    "send a message",
];

/// … that it handles keys or passwords (rated High).
pub const SKILL_RISK_SECRETS: &[&str] = &[
    "api key",
    "api_key",
    "apikey",
    "password",
    "token",
    "secret",
    "credential",
    "private key",
];

/// … that it runs code it downloads (rated High).
pub const SKILL_RISK_RUNS_DOWNLOADS: &[&str] = &[
    "| sh",
    "| bash",
    "|sh",
    "|bash",
    "invoke-expression",
    "iex ",
    "| iex",
    "curl -s",
    "wget ",
];

/// … that it runs commands (at least Medium).
pub const SKILL_RISK_COMMANDS: &[&str] = &[
    "```sh",
    "```bash",
    "```powershell",
    "```ps1",
    "```shell",
    "```cmd",
    "run `",
    "shell",
    "terminal",
];

/// Programs and models on this computer (`program/…` models) live in this folder inside the
/// data folder: whisper.cpp models (`ggml-*.bin`) in `whisper/`, Piper voices (`*.onnx`) in
/// `piper/`. Thrall never downloads them itself.
pub const LOCAL_MODELS_DIR: &str = "models";
pub const WHISPER_MODELS_DIR: &str = "whisper";
pub const PIPER_VOICES_DIR: &str = "piper";

/// whisper.cpp's command-line program, by the names its builds and packages use.
pub const WHISPER_PROGRAMS: &[&str] = &["whisper-cli", "whisper-cpp"];

/// Piper's command-line program.
pub const PIPER_PROGRAM: &str = "piper";

/// The file endings of whisper.cpp models and Piper voices.
pub const WHISPER_MODEL_EXT: &str = "bin";
pub const PIPER_VOICE_EXT: &str = "onnx";

/// The `program/…` names: whisper.cpp, Piper, a Stable Diffusion web UI.
pub const PROGRAM_WHISPER: &str = "whisper";
pub const PROGRAM_PIPER: &str = "piper";
pub const PROGRAM_SD_WEBUI: &str = "sd-webui";

/// whisper.cpp reads 16 kHz mono sound.
pub const WHISPER_SAMPLE_RATE: &str = "16000";

/// Longest one local program may run, in seconds (long recordings are cut into parts first).
pub const PROGRAM_MAX_SECS: u64 = 15 * 60;

/// A Stable Diffusion web UI's (AUTOMATIC1111, Forge) picture and model-list endpoints.
pub const SD_TXT2IMG_PATH: &str = "/sdapi/v1/txt2img";
pub const SD_IMG2IMG_PATH: &str = "/sdapi/v1/img2img";
pub const SD_MODELS_PATH: &str = "/sdapi/v1/sd-models";

/// Steps a local picture takes.
pub const SD_STEPS: u32 = 25;

/// Picture sizes per shape (sizes SDXL-class models are trained on); anything else is square.
pub const SD_SIZES: &[(&str, u32, u32)] = &[
    ("1:1", 1024, 1024),
    ("16:9", 1344, 768),
    ("9:16", 768, 1344),
    ("4:3", 1152, 896),
    ("3:4", 896, 1152),
    ("3:2", 1216, 832),
    ("2:3", 832, 1216),
];

/// Seconds a quick look for a program's web server may take.
pub const PROGRAM_PROBE_SECS: u64 = 3;

/// LM Studio's usual address (Ollama's is the default local server).
pub const LM_STUDIO_URL: &str = "http://localhost:1234/v1";

/// Tools that run on a companion PC for a session that works there: everything that touches the
/// PC's files or runs its programs. The rest (web, AI, memory, schedules) run here.
pub const COMPANION_TOOLS: &[&str] = &[
    "read_file",
    "write_file",
    "edit_file",
    "list_dir",
    "glob",
    "grep",
    "read_document",
    "edit_spreadsheet",
    "edit_word",
    "edit_pdf",
    "make_pdf",
    "archive",
    "style_check",
    "shell",
    "shell_output",
    "shell_stop",
    "git",
    "system_info",
    "clipboard",
];

/// Biggest file carried across a companion PC's link in one go (a picture a tool made, a file
/// kept for rewinding or copied over).
pub const COMPANION_FILE_MAX_BYTES: u64 = 16 * 1024 * 1024;
