pub mod control_plane;
pub mod files;
pub mod history;
pub mod holo;
pub mod registry;
pub mod system;

use crate::error::{ApiError, LiveError};
use crate::module::LiveModule;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::sync::Arc;

pub fn builtins() -> Vec<Arc<dyn LiveModule>> {
    vec![
        Arc::new(system::SystemModule),
        Arc::new(registry::KappaRegistryModule),
        Arc::new(files::FilesModule),
        Arc::new(holo::HoloModule),
        Arc::new(history::HistoryModule),
        Arc::new(control_plane::ControlPlaneModule),
    ]
}

pub struct HttpError(pub LiveError);

impl From<LiveError> for HttpError {
    fn from(error: LiveError) -> Self {
        Self(error)
    }
}

impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        let status = match self.0 {
            LiveError::Authentication(_) => StatusCode::UNAUTHORIZED,
            LiveError::Authorization(_) => StatusCode::FORBIDDEN,
            LiveError::NotFound(_) => StatusCode::NOT_FOUND,
            LiveError::Conflict(_) | LiveError::UnknownCommitState(_) => StatusCode::CONFLICT,
            LiveError::Capability(_) => StatusCode::NOT_IMPLEMENTED,
            LiveError::Config(_) | LiveError::Protocol(_) | LiveError::InvalidHolo(_) => {
                StatusCode::BAD_REQUEST
            }
            LiveError::Io(_) | LiveError::Transport(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(ApiError::from(&self.0))).into_response()
    }
}
