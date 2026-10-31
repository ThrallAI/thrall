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
/// Characters of a single line shown before it's cut.
pub const LINE_MAX_CHARS: usize = 2000;

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
/// Pieces of a shell command that make it high-risk (lower-case match). High-risk commands always
/// ask and are never covered by "always".
pub const SHELL_RISKY: &[&str] = &[
    "rm -r",
    "rm -f",
    "rmdir",
    "remove-item",
    "del /s",
    "rd /s",
    "format ",
    "mkfs",
    "shutdown",
    "restart-computer",
    "stop-computer",
    "reg delete",
    "set-executionpolicy",
    "git push --force",
    "git reset --hard",
    "git clean",
    "> /dev/",
    "dd if=",
];

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
