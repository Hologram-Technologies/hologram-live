//! Portable View provider and transactional host-surface lifecycle.

use crate::application_plan::{ProviderContext, ResolvedLayer};
use crate::error::{LiveError, Result};
use crate::holo_provider::{
    LayerInvocation, LayerPrepareContext, LayerProvider, LayerRuntimeStatus, PreparedLayer,
    ProviderTarget,
};
use crate::holo_view;
use hologram::space::LayerKind;
use hologram_view_surface::{
    PortableViewAsset, PortableViewAttachment, PortableViewSurface, ViewAttachmentId,
    ViewSurfaceRegistry,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

static ATTACHMENT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub struct ViewProvider {
    surfaces: Arc<ViewSurfaceRegistry>,
}

impl ViewProvider {
    pub fn new(surfaces: Arc<ViewSurfaceRegistry>) -> Self {
        Self { surfaces }
    }

    pub fn headless() -> Self {
        Self::new(Arc::new(ViewSurfaceRegistry::new()))
    }

    fn resolve_surface(
        &self,
        target: ProviderTarget,
    ) -> std::result::Result<Arc<dyn PortableViewSurface>, String> {
        self.surfaces.portable()?.ok_or_else(|| match target {
            ProviderTarget::Direct => concat!(
                "portable View surface is unavailable for direct/headless execution; ",
                "run the application through a host that publishes a portable surface"
            )
            .to_owned(),
            ProviderTarget::Resident => {
                "portable View surface is unavailable for resident execution".to_owned()
            }
        })
    }
}

#[tonic::async_trait]
impl LayerProvider for ViewProvider {
    fn kind(&self) -> LayerKind {
        LayerKind::View
    }

    fn contract(&self) -> Option<&'static str> {
        None
    }

    fn name(&self) -> &'static str {
        "portable-view"
    }

    fn availability(
        &self,
        context: &ProviderContext<'_>,
        target: ProviderTarget,
    ) -> std::result::Result<(), String> {
        holo_view::validate_surface(context.aux).map_err(|error| error.to_string())?;
        holo_view::decode(context.content).map_err(|error| error.to_string())?;
        self.resolve_surface(target).map(|_| ())
    }

    async fn prepare(&self, context: LayerPrepareContext) -> Result<Arc<dyn PreparedLayer>> {
        holo_view::validate_surface(&context.layer.aux)?;
        let bundle = holo_view::decode(&context.layer.content)?;
        let surface = self
            .resolve_surface(context.target)
            .map_err(LiveError::Capability)?;
        let id = attachment_id(&context.layer, &context.identity.application_kappa);
        let resident_bytes = context.layer.content.len();
        let attachment = PortableViewAttachment {
            id: id.clone(),
            entry: bundle.entry,
            assets: bundle
                .files
                .into_iter()
                .map(|file| PortableViewAsset {
                    path: file.path,
                    bytes: file.bytes.into(),
                })
                .collect(),
            intents: context.view_intents,
        };
        Ok(Arc::new(PreparedViewLayer {
            position: context.layer.position,
            surface,
            attachment,
            id,
            resident_bytes,
            state: Mutex::new(AttachmentState::Prepared),
        }))
    }
}

fn attachment_id(layer: &ResolvedLayer, application_kappa: &str) -> ViewAttachmentId {
    let sequence = ATTACHMENT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut token = blake3::Hasher::new();
    token.update(application_kappa.as_bytes());
    token.update(&layer.position.to_be_bytes());
    token.update(&sequence.to_be_bytes());
    ViewAttachmentId {
        token: token.finalize().to_hex().to_string(),
        application_kappa: application_kappa.to_owned(),
        layer_position: layer.position,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttachmentState {
    Prepared,
    Attached,
    Stopped,
}

struct PreparedViewLayer {
    position: u32,
    surface: Arc<dyn PortableViewSurface>,
    attachment: PortableViewAttachment,
    id: ViewAttachmentId,
    resident_bytes: usize,
    state: Mutex<AttachmentState>,
}

#[tonic::async_trait]
impl PreparedLayer for PreparedViewLayer {
    fn position(&self) -> u32 {
        self.position
    }

    async fn start(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        match *state {
            AttachmentState::Prepared => {
                self.surface
                    .attach(self.attachment.clone())
                    .await
                    .map_err(|error| {
                        LiveError::Transport(format!(
                            "attach portable View layer {}: {error}",
                            self.position
                        ))
                    })?;
                *state = AttachmentState::Attached;
                Ok(())
            }
            AttachmentState::Attached => Ok(()),
            AttachmentState::Stopped => Err(LiveError::Conflict(format!(
                "portable View layer {} cannot start after it stopped",
                self.position
            ))),
        }
    }

    async fn invoke(&self, _inputs: Vec<Vec<u8>>) -> Result<LayerInvocation> {
        Err(LiveError::Capability(format!(
            "portable View layer {} is non-exit-bearing and cannot be invoked",
            self.position
        )))
    }

    async fn stop(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        match *state {
            AttachmentState::Prepared => {
                *state = AttachmentState::Stopped;
                Ok(())
            }
            AttachmentState::Attached => {
                self.surface.detach(&self.id).await.map_err(|error| {
                    LiveError::Transport(format!(
                        "detach portable View layer {}: {error}",
                        self.position
                    ))
                })?;
                *state = AttachmentState::Stopped;
                Ok(())
            }
            AttachmentState::Stopped => Ok(()),
        }
    }

    fn status(&self) -> LayerRuntimeStatus {
        LayerRuntimeStatus {
            resident_bytes: self.resident_bytes,
            ..LayerRuntimeStatus::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application_plan::{HoloIdentity, ResolutionSource};
    use crate::holo_capability::{self, EffectiveGrant, RequestedCapabilities};
    use crate::holo_provider::LayerCompletionRole;
    use hologram::space::address_bytes;
    use hologram_view_surface::{PortableViewIntentHandler, SurfaceFuture, ViewIntentRequest};
    use std::sync::Mutex as StdMutex;

    #[derive(Default)]
    struct RecordingSurface {
        events: StdMutex<Vec<String>>,
        attachment: StdMutex<Option<PortableViewAttachment>>,
    }

    struct UnboundIntents;

    impl PortableViewIntentHandler for UnboundIntents {
        fn handle<'a>(
            &'a self,
            _id: &'a ViewAttachmentId,
            _request: ViewIntentRequest,
        ) -> hologram_view_surface::IntentFuture<'a> {
            Box::pin(async { Err("test intent handler is unbound".to_owned()) })
        }
    }

    impl PortableViewSurface for RecordingSurface {
        fn attach(&self, view: PortableViewAttachment) -> SurfaceFuture<'_> {
            Box::pin(async move {
                self.events
                    .lock()
                    .expect("events")
                    .push(format!("attach:{}", view.id.layer_position));
                *self.attachment.lock().expect("attachment") = Some(view);
                Ok(())
            })
        }

        fn detach<'a>(&'a self, id: &'a ViewAttachmentId) -> SurfaceFuture<'a> {
            Box::pin(async move {
                self.events
                    .lock()
                    .expect("events")
                    .push(format!("detach:{}", id.layer_position));
                Ok(())
            })
        }
    }

    fn bundle() -> Arc<[u8]> {
        holo_view::encode(&holo_view::ViewBundle {
            version: holo_view::VIEW_BUNDLE_VERSION,
            entry: holo_view::PORTABLE_ENTRY.to_owned(),
            files: vec![holo_view::ViewFile {
                path: holo_view::PORTABLE_ENTRY.to_owned(),
                bytes: b"<h1>view</h1>".to_vec(),
            }],
        })
        .expect("bundle")
        .into()
    }

    fn layer(content: Arc<[u8]>) -> ResolvedLayer {
        ResolvedLayer {
            position: 1,
            kind: LayerKind::View,
            content_kappa: address_bytes(&content).to_string(),
            entry: String::new(),
            aux: holo_view::PORTABLE_SURFACE.to_owned(),
            primary: false,
            content,
            resolution_source: ResolutionSource::Embedded,
            provider: "portable-view".to_owned(),
        }
    }

    fn requested_capabilities() -> RequestedCapabilities {
        let bytes: Arc<[u8]> = holo_capability::empty_canonical().into();
        let kappa = address_bytes(&bytes);
        RequestedCapabilities::decode(kappa.as_ref(), bytes).expect("capabilities")
    }

    #[test]
    fn headless_availability_is_explicit() {
        let content = bundle();
        let provider = ViewProvider::headless();
        let error = provider
            .availability(
                &ProviderContext {
                    application_kappa: "blake3:application",
                    position: 1,
                    kind: LayerKind::View,
                    entry: "",
                    aux: holo_view::PORTABLE_SURFACE,
                    primary: false,
                    layer_count: 2,
                    content: &content,
                },
                ProviderTarget::Direct,
            )
            .expect_err("headless surface");
        assert!(error.contains("direct/headless"), "{error}");
        assert!(error.contains("portable View surface"), "{error}");
    }

    #[tokio::test]
    async fn prepare_start_and_stop_follow_the_surface_lifecycle() {
        let surface = Arc::new(RecordingSurface::default());
        let surfaces = Arc::new(ViewSurfaceRegistry::new());
        surfaces
            .register_portable(surface.clone())
            .expect("register");
        let provider = ViewProvider::new(surfaces);
        let content = bundle();
        let prepared = provider
            .prepare(LayerPrepareContext {
                identity: HoloIdentity {
                    archive_kappa: "blake3:archive".to_owned(),
                    archive_fingerprint: "fingerprint".to_owned(),
                    application_kappa: "blake3:application".to_owned(),
                },
                effective_grant: EffectiveGrant::local_baseline(),
                requested_capabilities: requested_capabilities(),
                layer: layer(content.clone()),
                target: ProviderTarget::Direct,
                view_intents: Arc::new(UnboundIntents),
            })
            .await
            .expect("prepare");
        assert!(surface.events.lock().expect("events").is_empty());
        assert_eq!(prepared.status().resident_bytes, content.len());

        prepared.start().await.expect("start");
        let attachment = surface
            .attachment
            .lock()
            .expect("attachment")
            .clone()
            .expect("attached bundle");
        assert_eq!(attachment.entry, holo_view::PORTABLE_ENTRY);
        assert_eq!(attachment.assets.len(), 1);
        assert_eq!(attachment.id.application_kappa, "blake3:application");
        prepared.stop().await.expect("stop");
        prepared.stop().await.expect("idempotent stop");
        assert_eq!(
            *surface.events.lock().expect("events"),
            ["attach:1", "detach:1"]
        );
        assert_eq!(
            crate::holo_provider::layer_completion_role(LayerKind::View),
            LayerCompletionRole::NonExitBearing
        );
    }
}
