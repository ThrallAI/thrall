//! Named constants for `thrall-gates`. Every limit, timeout and repeated string lives here.

/// Longest message the input gate accepts, in characters.
pub const INPUT_MAX_CHARS: usize = 200_000;

/// What a secret is replaced with wherever a gate removes it.
pub const REDACTED: &str = "[redacted secret]";

/// Shapes of common API keys and tokens. Each must be specific enough not to catch normal text.
pub const SECRET_PATTERNS: &[&str] = &[
    // OpenRouter, OpenAI, Anthropic and similar `sk-` keys.
    r"\bsk-(?:or-|ant-|proj-)?[A-Za-z0-9_\-]{20,}",
    // GitHub tokens.
    r"\bgh[pousr]_[A-Za-z0-9]{30,}",
    r"\bgithub_pat_[A-Za-z0-9_]{40,}",
    // AWS access key ids.
    r"\bAKIA[0-9A-Z]{16}\b",
    // Google API keys.
    r"\bAIza[0-9A-Za-z_\-]{35}\b",
    // Slack tokens.
    r"\bxox[abpr]-[A-Za-z0-9\-]{10,}",
    // fal.ai keys (uuid:hex).
    r"\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}:[0-9a-f]{32}\b",
    // Private key blocks.
    r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
];

/// Gate names in the audit log.
pub mod gate {
    /// Checks what the user sends.
    pub const INPUT: &str = "input";
    /// Controls what enters the model's context.
    pub const CONTEXT: &str = "context";
    /// Allows, asks about or denies tool calls.
    pub const TOOL: &str = "tool";
    /// Checks what Thrall says.
    pub const OUTPUT: &str = "output";
    /// Controls what reaches long-term memory.
    pub const MEMORY: &str = "memory";
}

/// Decisions in the audit log.
pub mod decision {
    /// Let through.
    pub const ALLOW: &str = "allow";
    /// Let through with changes (secrets removed, text cleaned).
    pub const CHANGED: &str = "changed";
    /// The user said yes.
    pub const APPROVED: &str = "approved";
    /// The user said yes, and yes from now on.
    pub const ALWAYS: &str = "always";
    /// The user said no.
    pub const DECLINED: &str = "declined";
    /// Refused without asking.
    pub const DENY: &str = "deny";
}
