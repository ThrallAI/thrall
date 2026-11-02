//! Named constants for `thrall-llm`. Every limit, timeout and repeated string lives here.

/// Seconds allowed to open a connection to the provider.
pub const CONNECT_TIMEOUT_SECS: u64 = 15;

/// Seconds of silence allowed between streamed chunks before the call counts as stalled.
pub const READ_TIMEOUT_SECS: u64 = 120;

/// Extra attempts after a failure that may be temporary (network, 429, 5xx).
pub const MAX_RETRIES: u32 = 3;

/// First retry delay in milliseconds; doubles each attempt.
pub const RETRY_BASE_DELAY_MS: u64 = 1000;

/// Longest a provider may ask us to wait (Retry-After) before we give up instead.
pub const MAX_RETRY_AFTER_SECS: u64 = 60;

/// Path of the chat endpoint under the provider's base URL.
pub const CHAT_COMPLETIONS_PATH: &str = "/chat/completions";

/// Path of the model list under the provider's base URL (what each model accepts).
pub const MODELS_PATH: &str = "/models";
/// Path of the account's credit (bought and used) under the provider's base URL.
pub const CREDITS_PATH: &str = "/credits";

/// Path of the embeddings endpoint under the provider's base URL.
pub const EMBEDDINGS_PATH: &str = "/embeddings";

/// Marker OpenAI-compatible streams send as the last event.
pub const STREAM_DONE_MARKER: &str = "[DONE]";

/// App name OpenRouter shows in its activity log.
pub const OPENROUTER_APP_TITLE: &str = "Thrall";

/// Most characters of a provider error body kept in a message.
pub const MAX_ERROR_BODY_CHARS: usize = 300;

/// Samples per second of the speech audio models stream (raw 16-bit mono).
pub const SPEECH_SAMPLE_RATE: u32 = 24_000;

/// Instructions for reading text aloud.
pub const SPEAK_PROMPT: &str = "You are a text-to-speech voice. Read the user's message aloud exactly as written, word for word, in a natural voice. Don't answer it, add to it, or comment on it.";

/// fal.ai's job queue (video models).
pub const FAL_QUEUE_BASE: &str = "https://queue.fal.run";

/// Longest a finished video may take to download, in seconds.
pub const FAL_DOWNLOAD_TIMEOUT_SECS: u64 = 300;

/// Model ids starting with this go to ChatGPT (your ChatGPT plan) instead of OpenRouter:
/// `chatgpt/gpt-5.3-codex`.
pub const CHATGPT_MODEL_PREFIX: &str = "chatgpt/";

/// `local/llama3.1`: a model on a server on this computer or network (Ollama, LM Studio), free.
pub const LOCAL_MODEL_PREFIX: &str = "local/";

/// `direct/openai/gpt-5`: a model reached straight at the company that makes it, with the
/// person's own key for that company, instead of through OpenRouter.
pub const DIRECT_MODEL_PREFIX: &str = "direct/";

/// Companies Thrall reaches directly with the person's own key (`thrall key set <id>`).
pub const DIRECT_SUPPLIERS: &[crate::direct::DirectSupplier] = &[
    crate::direct::DirectSupplier {
        id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        key_env_var: "OPENAI_API_KEY",
        max_tokens_field: Some("max_completion_tokens"),
    },
    crate::direct::DirectSupplier {
        id: "anthropic",
        name: "Anthropic",
        base_url: "https://api.anthropic.com/v1",
        key_env_var: "ANTHROPIC_API_KEY",
        max_tokens_field: Some("max_tokens"),
    },
    crate::direct::DirectSupplier {
        id: "google",
        name: "Google",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        key_env_var: "GEMINI_API_KEY",
        max_tokens_field: Some("max_tokens"),
    },
    crate::direct::DirectSupplier {
        id: ELEVENLABS_ID,
        name: "ElevenLabs",
        base_url: "https://api.elevenlabs.io/v1",
        key_env_var: "ELEVENLABS_API_KEY",
        max_tokens_field: None,
    },
];

/// ElevenLabs' supplier id (voices only).
pub const ELEVENLABS_ID: &str = "elevenlabs";

/// ElevenLabs: speech for one voice is made at this path, then the voice id.
pub const ELEVENLABS_SPEECH_PATH: &str = "/text-to-speech/";

/// The audio ElevenLabs sends back: raw 16-bit samples at `SPEECH_SAMPLE_RATE`.
pub const ELEVENLABS_OUTPUT_FORMAT: &str = "pcm_24000";

/// The header ElevenLabs reads the key from.
pub const ELEVENLABS_KEY_HEADER: &str = "xi-api-key";

/// The header Anthropic reads the key from on its own endpoints (the model list).
pub const ANTHROPIC_KEY_HEADER: &str = "x-api-key";

/// Anthropic's API version header, and the version asked for.
pub const ANTHROPIC_VERSION_HEADER: &str = "anthropic-version";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Google lists its models as `models/<id>`; the id alone is what chats take.
pub const GOOGLE_MODEL_LIST_PREFIX: &str = "models/";

/// Most model ids a supplier's Test reports.
pub const TEST_MODELS_MAX: usize = 200;

/// ChatGPT's Codex backend (the endpoint the official Codex app uses with a ChatGPT sign-in).
pub const CHATGPT_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";

/// OpenAI's sign-in pages.
pub const OPENAI_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";

/// Where sign-in codes and refresh tokens are swapped for access tokens.
pub const OPENAI_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// The Codex app's public sign-in client id (no secret: sign-in uses PKCE).
pub const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// What the sign-in asks for.
pub const OPENAI_SCOPES: &str =
    "openid profile email offline_access api.connectors.read api.connectors.invoke";

/// The port the browser comes back to after signing in (the one OpenAI expects for this client).
pub const OPENAI_CALLBACK_PORT: u16 = 1455;

/// The address the browser comes back to.
pub const OPENAI_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";

/// Where the account id sits in OpenAI's sign-in token.
pub const OPENAI_AUTH_CLAIM: &str = "https://api.openai.com/auth";

/// Seconds before an access token expires that it's renewed.
pub const TOKEN_RENEW_MARGIN_SECS: u64 = 120;

/// Seconds a ChatGPT reply may stay silent (long tool-heavy turns think for a while).
pub const CHATGPT_READ_TIMEOUT_SECS: u64 = 300;

/// The models a ChatGPT sign-in can use (the Codex backend's list).
pub const CHATGPT_MODELS_URL: &str = "https://chatgpt.com/backend-api/codex/models";

/// The Codex app version sent when listing models. The list leaves out models that need a newer
/// app, so keep this at a current Codex release (`npm view @openai/codex version`).
pub const CHATGPT_CLIENT_VERSION: &str = "0.162.1";
