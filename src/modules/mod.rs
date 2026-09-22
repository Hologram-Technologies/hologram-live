use crate::error::{ApiError, LiveError};
use crate::module::LiveModule;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::sync::Arc;

mod model_inventory;

/// Declares every trusted, statically linked module in one place.
///
/// A module still owns its typed routes, lifecycle, and descriptor. Adding it
/// to `default` makes it available to the registry and enables it in the
/// default configuration without duplicating its ID in `config.rs`. A module
/// under `opt_in` is available but runs only when the configuration names it.
macro_rules! builtin_modules {
    (
        default: [ $( $module:ident :: $module_type:ident ),+ $(,)? ],
        opt_in: [ $( $(#[$gate:meta])* $opt:ident :: $opt_type:ident ),* $(,)? ] $(,)?
    ) => {
        $(pub mod $module;)+
        $($(#[$gate])* pub mod $opt;)*

        fn default_builtins() -> Vec<Arc<dyn LiveModule>> {
            vec![$(Arc::new($module::$module_type)),+]
        }

        pub fn builtins() -> Vec<Arc<dyn LiveModule>> {
            #[allow(unused_mut, reason = "every opt-in module may be compiled out")]
            let mut modules = default_builtins();
            $($(#[$gate])* modules.push(Arc::new($opt::$opt_type));)*
            modules
        }

        pub fn builtin_ids() -> Vec<String> {
            ids(builtins())
        }

        pub fn default_builtin_ids() -> Vec<String> {
            ids(default_builtins())
        }
    };
}

fn ids(modules: Vec<Arc<dyn LiveModule>>) -> Vec<String> {
    modules
        .into_iter()
        .map(|module| module.descriptor().id.to_owned())
        .collect()
}

builtin_modules! {
    default: [
        system::SystemModule,
        registry::KappaRegistryModule,
        files::FilesModule,
        holo::HoloModule,
        history::HistoryModule,
        chat::ChatModule,
        inference::InferenceModule,
        openai_compat::OpenAiCompatModule,
        ollama_compat::OllamaCompatModule,
        control_plane::ControlPlaneModule,
    ],
    opt_in: [
        #[cfg(feature = "oci")]
        oci::OciRegistryModule,
    ],
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
