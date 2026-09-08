//! Unified error types.

/// Unified error type for the whole crate.
#[derive(Debug, thiserror::Error)]
pub enum SkillsError {
    /// A user-facing plain error message.
    #[error("{0}")]
    Message(String),
    /// One or more invalid agent names (comma-separated).
    #[error("invalid agents: {0}")]
    InvalidAgents(String),
    /// I/O error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON (de)serialization error.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    /// YAML (de)serialization error.
    #[error(transparent)]
    Yaml(#[from] noyalib::Error),
    /// git (libgit2) error.
    #[error(transparent)]
    Git(#[from] git2::Error),
    /// HTTP request error.
    #[error(transparent)]
    Http(Box<ureq::Error>),
    /// Zip archive error.
    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
}

/// Project-wide unified `Result` alias.
pub type Result<T> = std::result::Result<T, SkillsError>;

/// Semantic alias for library consumers.
pub type Error = SkillsError;

impl SkillsError {
    /// Construct a plain-message error.
    pub fn msg(msg: impl Into<String>) -> Self {
        SkillsError::Message(msg.into())
    }
}

impl From<ureq::Error> for SkillsError {
    fn from(e: ureq::Error) -> Self {
        SkillsError::Http(Box::new(e))
    }
}
