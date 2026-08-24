use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use utoipa::ToSchema;

pub type Result<T, E = LiveError> = std::result::Result<T, E>;

#[derive(Debug)]
pub enum LiveError {
    Io(String),
    Config(String),
    Protocol(String),
    Transport(String),
    Authentication(String),
    Authorization(String),
    Capability(String),
    NotFound(String),
    Conflict(String),
    InvalidHolo(String),
    UnknownCommitState(String),
}

impl LiveError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Io(_) => "LIVE_IO",
            Self::Config(_) => "LIVE_CONFIG_INVALID",
            Self::Protocol(_) => "LIVE_PROTOCOL_ERROR",
            Self::Transport(_) => "LIVE_TRANSPORT_UNAVAILABLE",
            Self::Authentication(_) => "LIVE_AUTHENTICATION_FAILED",
            Self::Authorization(_) => "LIVE_AUTHORIZATION_DENIED",
            Self::Capability(_) => "LIVE_CAPABILITY_MISSING",
            Self::NotFound(_) => "LIVE_NOT_FOUND",
            Self::Conflict(_) => "LIVE_CONFLICT",
            Self::InvalidHolo(_) => "LIVE_HOLO_INVALID",
            Self::UnknownCommitState(_) => "LIVE_UNKNOWN_COMMIT_STATE",
        }
    }

    pub fn io(path: &Path, error: std::io::Error) -> Self {
        Self::Io(format!("{}: {error}", path.display()))
    }
}

impl fmt::Display for LiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Io(message)
            | Self::Config(message)
            | Self::Protocol(message)
            | Self::Transport(message)
            | Self::Authentication(message)
            | Self::Authorization(message)
            | Self::Capability(message)
            | Self::NotFound(message)
            | Self::Conflict(message)
            | Self::InvalidHolo(message)
            | Self::UnknownCommitState(message) => message,
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for LiveError {}

impl From<std::io::Error> for LiveError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<toml::de::Error> for LiveError {
    fn from(error: toml::de::Error) -> Self {
        Self::Config(error.to_string())
    }
}

impl From<toml::ser::Error> for LiveError {
    fn from(error: toml::ser::Error) -> Self {
        Self::Config(error.to_string())
    }
}

impl From<serde_json::Error> for LiveError {
    fn from(error: serde_json::Error) -> Self {
        Self::Protocol(error.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl From<&LiveError> for ApiError {
    fn from(error: &LiveError) -> Self {
        Self {
            code: error.code().to_owned(),
            message: error.to_string(),
        }
    }
}

impl From<ApiError> for LiveError {
    fn from(error: ApiError) -> Self {
        match error.code.as_str() {
            "LIVE_AUTHENTICATION_FAILED" => Self::Authentication(error.message),
            "LIVE_AUTHORIZATION_DENIED" => Self::Authorization(error.message),
            "LIVE_CAPABILITY_MISSING" => Self::Capability(error.message),
            "LIVE_NOT_FOUND" => Self::NotFound(error.message),
            "LIVE_CONFLICT" => Self::Conflict(error.message),
            "LIVE_HOLO_INVALID" => Self::InvalidHolo(error.message),
            "LIVE_UNKNOWN_COMMIT_STATE" => Self::UnknownCommitState(error.message),
            _ => Self::Protocol(format!("{}: {}", error.code, error.message)),
        }
    }
}
