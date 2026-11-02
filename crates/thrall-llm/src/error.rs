//! Provider errors. `Display` is written for the user; [`LlmError::is_retryable`] drives retries.

/// A model call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LlmError {
    /// The provider rejected the key.
    #[error(
        "{provider} didn't accept your API key. Check it, then run `thrall key set {provider_id}`."
    )]
    Unauthorized {
        /// Provider display name.
        provider: String,
        /// Provider id for the command hint.
        provider_id: String,
    },

    /// Too many requests.
    #[error("{provider} is limiting requests right now. Wait a moment and try again.")]
    RateLimited {
        /// Provider display name.
        provider: String,
        /// Seconds the provider asked us to wait, if it said.
        retry_after_secs: Option<u64>,
    },

    /// The account has no credit left to pay for the request.
    #[error(
        "Your {provider} credit has run out. Top up your credit, then say \"continue\" and Thrall \
         will pick up where it stopped."
    )]
    OutOfCredit {
        /// Provider display name.
        provider: String,
    },

    /// The provider refused the request itself (bad model id, too long, …).
    #[error("{provider} refused the request: {message}")]
    Rejected {
        /// Provider display name.
        provider: String,
        /// HTTP status.
        status: u16,
        /// The provider's explanation.
        message: String,
    },

    /// The provider had a problem on its side.
    #[error("{provider} is having problems ({status}). Try again in a minute.")]
    ServerError {
        /// Provider display name.
        provider: String,
        /// HTTP status.
        status: u16,
    },

    /// Couldn't reach the provider, or the connection dropped.
    #[error("Couldn't reach {provider}: {detail}. Check your internet connection.")]
    Network {
        /// Provider display name.
        provider: String,
        /// What went wrong.
        detail: String,
    },

    /// The person's turns use their own key, and it isn't set (Thrall's own refusal).
    #[error("{message}")]
    KeyMissing {
        /// What to tell them.
        message: String,
    },

    /// The person has spent this month's limit (Thrall's own refusal).
    #[error("{message}")]
    OverLimit {
        /// What to tell them.
        message: String,
    },

    /// The provider sent something we couldn't understand.
    #[error("{provider} sent a reply Thrall couldn't read: {detail}")]
    BadResponse {
        /// Provider display name.
        provider: String,
        /// What was wrong.
        detail: String,
    },
}

impl LlmError {
    /// The same error, naming `name` (and `id` in the key hint) as the provider: a supplier
    /// reached through a client built for another (an OpenAI key through the compatible client).
    pub fn relabel(self, name: &str, id: &str) -> Self {
        let name = name.to_owned();
        match self {
            Self::Unauthorized { .. } => Self::Unauthorized {
                provider: name,
                provider_id: id.to_owned(),
            },
            Self::RateLimited {
                retry_after_secs, ..
            } => Self::RateLimited {
                provider: name,
                retry_after_secs,
            },
            Self::OutOfCredit { .. } => Self::OutOfCredit { provider: name },
            Self::Rejected {
                status, message, ..
            } => Self::Rejected {
                provider: name,
                status,
                message,
            },
            Self::ServerError { status, .. } => Self::ServerError {
                provider: name,
                status,
            },
            Self::Network { detail, .. } => Self::Network {
                provider: name,
                detail,
            },
            Self::BadResponse { detail, .. } => Self::BadResponse {
                provider: name,
                detail,
            },
            other @ (Self::KeyMissing { .. } | Self::OverLimit { .. }) => other,
        }
    }

    /// Whether trying again may work.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimited { .. } | Self::ServerError { .. } | Self::Network { .. }
        )
    }
}
