//! Storage errors.

/// A storage call failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The database refused or failed.
    #[error("Thrall's database had a problem: {0}")]
    Database(#[from] sqlx::Error),

    /// Bringing the database up to date failed.
    #[error("Thrall couldn't update its database: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),

    /// The thing asked for doesn't exist (or belongs to someone else).
    #[error("That {0} doesn't exist.")]
    NotFound(&'static str),

    /// Stored data didn't make sense (a negative counter, …).
    #[error("Thrall's database holds unexpected data: {0}")]
    Corrupt(String),
}
