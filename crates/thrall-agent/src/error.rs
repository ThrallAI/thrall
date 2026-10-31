//! Agent errors: things that stop a turn from being recorded at all. Model failures are not here;
//! they end the turn as `failed` with an `error` event the user sees.

use thrall_store::StoreError;

/// The agent couldn't run or record a turn.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// Storage failed.
    #[error(transparent)]
    Store(#[from] StoreError),

    /// The session doesn't exist (or isn't the user's).
    #[error("There's no session {0}. Run `thrall sessions` to see yours.")]
    SessionNotFound(String),

    /// The input gate refused the message.
    #[error("{0}")]
    InputRefused(String),

    /// A turn is already running in this session.
    #[error("Thrall is already working in this session. Wait for it, or press Esc to stop it.")]
    Busy,

    /// An attached file id doesn't match an upload.
    #[error("The attached file {0} isn't there any more. Attach it again.")]
    FileNotFound(String),

    /// A recording couldn't be turned into text.
    #[error("Couldn't transcribe the recording: {0}")]
    Transcription(String),

    /// There's no earlier message to edit or retry.
    #[error("There's nothing to retry yet: send a message first.")]
    NothingToRetry,

    /// The turn to rewind to isn't in this session.
    #[error("That message isn't in this session any more, so there's nothing to rewind to.")]
    TurnNotFound,

    /// No such slash command.
    #[error("There's no /{0} command. Type /help to see them.")]
    UnknownCommand(String),

    /// Home is built in: it can't be renamed, archived, pinned, moved or deleted.
    #[error(
        "Home is built in, so it can't be renamed, archived, pinned, moved or deleted. Earlier \
         pages are cleared in Settings → General → Home."
    )]
    HomeIsBuiltIn,

    /// The person's turns use their own AI key, and it isn't set.
    #[error("{0}")]
    OwnKeyMissing(String),

    /// The person has spent this month's limit.
    #[error("{0}")]
    OverLimit(String),

    /// An event couldn't be encoded (a bug).
    #[error("Thrall hit an internal error encoding an event: {0}")]
    Encode(#[from] serde_json::Error),
}

impl AgentError {
    /// The protocol error code clients switch on.
    pub fn code(&self) -> thrall_protocol::ErrorCode {
        use thrall_protocol::ErrorCode;
        match self {
            Self::SessionNotFound(_) => ErrorCode::NotFound,
            Self::InputRefused(_) | Self::HomeIsBuiltIn => ErrorCode::Invalid,
            Self::Busy => ErrorCode::Busy,
            Self::NothingToRetry | Self::TurnNotFound | Self::FileNotFound(_) => {
                ErrorCode::NotFound
            }
            Self::UnknownCommand(_) => ErrorCode::UnknownCommand,
            Self::Transcription(_) => ErrorCode::ProviderUnavailable,
            Self::OwnKeyMissing(_) => ErrorCode::ProviderKeyMissing,
            Self::OverLimit(_) => ErrorCode::ProviderRejected,
            Self::Store(thrall_store::StoreError::NotFound(_)) => ErrorCode::NotFound,
            Self::Store(_) | Self::Encode(_) => ErrorCode::Internal,
        }
    }
}
