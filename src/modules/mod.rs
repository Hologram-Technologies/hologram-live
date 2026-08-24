use crate::error::{ApiError, LiveError};
use crate::module::LiveModule;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::sync::Arc;

/// Declares every trusted, statically linked module in one place.
///
/// A module still owns its typed routes, lifecycle, and descriptor. Adding it
/// to this catalogue makes it available to the registry and enables it in the
/// default configuration without duplicating its ID in `config.rs`.
macro_rules! builtin_modules {
    ($( $module:ident :: $module_type:ident ),+ $(,)?) => {
        $(pub mod $module;)+

        pub fn builtins() -> Vec<Arc<dyn LiveModule>> {
            vec![$(Arc::new($module::$module_type)),+]
        }

        pub fn default_builtin_ids() -> Vec<String> {
            builtins()
                .into_iter()
                .map(|module| module.descriptor().id.to_owned())
                .collect()
        }
    };
}

builtin_modules! {
    system::SystemModule,
    registry::KappaRegistryModule,
    files::FilesModule,
    holo::HoloModule,
    history::HistoryModule,
    chat::ChatModule,
    control_plane::ControlPlaneModule,
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
