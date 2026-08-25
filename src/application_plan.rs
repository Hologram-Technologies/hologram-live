use crate::error::{LiveError, Result};
use crate::util::hex;
use hologram::archive::HoloLoader;
use hologram::space::{address_bytes, AppManifest, LayerKind, Realization};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use utoipa::ToSchema;

/// The three deliberately distinct identities of a compiled application.
///
/// `archive_kappa` addresses the complete physical file, while
/// `application_kappa` addresses the canonical application manifest and is
/// therefore stable across fat and thin packaging. The archive fingerprint is
/// the integrity value recorded in the `.holo` footer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct HoloIdentity {
    pub archive_kappa: String,
    pub archive_fingerprint: String,
    pub application_kappa: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanLimits {
    pub max_layers: usize,
    pub max_objects: usize,
    pub max_resolved_bytes: u64,
}

impl Default for PlanLimits {
    fn default() -> Self {
        Self {
            max_layers: 256,
            max_objects: 512,
            max_resolved_bytes: 4 * 1024 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionSource {
    Embedded,
    LocalStore,
    /// Reserved for a future explicitly configured registry or peer resolver.
    ConfiguredResolver(String),
}

#[derive(Debug, Clone)]
pub struct ResolvedObject {
    pub kappa: String,
    pub bytes: Arc<[u8]>,
    pub source: ResolutionSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestEdge {
    RequiredCapabilities,
    Layer { position: u32 },
}

impl std::fmt::Display for ManifestEdge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RequiredCapabilities => formatter.write_str("required capabilities"),
            Self::Layer { position } => write!(formatter, "layer {position}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAvailability {
    Unchecked,
    Available { provider: String },
    Unavailable { reason: String },
}

#[derive(Debug, Clone)]
pub struct PlannedLayer {
    pub position: u32,
    pub kind: LayerKind,
    pub content_kappa: String,
    pub entry: String,
    pub aux: String,
    pub primary: bool,
    pub resolution_source: Option<ResolutionSource>,
    pub provider: ProviderAvailability,
}

#[derive(Debug, Clone)]
pub struct ResolvedLayer {
    pub position: u32,
    pub kind: LayerKind,
    pub content_kappa: String,
    pub entry: String,
    pub aux: String,
    pub primary: bool,
    pub content: Arc<[u8]>,
    pub resolution_source: ResolutionSource,
    pub provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildReference {
    pub position: u32,
    pub application_kappa: String,
    pub capabilities_kappa: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanBlocker {
    MissingObject {
        kappa: String,
        edge: ManifestEdge,
    },
    ContentMismatch {
        kappa: String,
        edge: ManifestEdge,
        source: ResolutionSource,
    },
    ProviderUnavailable {
        position: u32,
        kind: String,
        entry: String,
        reason: String,
    },
    ChildClosureUnsupported {
        position: u32,
        application_kappa: String,
        capabilities_kappa: String,
    },
    LimitExceeded {
        limit: &'static str,
        maximum: u64,
        actual: u64,
        edge: Option<ManifestEdge>,
    },
    ExecutionShapeUnsupported {
        reason: String,
    },
}

impl PlanBlocker {
    fn into_error(self, application_kappa: &str) -> LiveError {
        match self {
            Self::MissingObject { kappa, edge } => LiveError::NotFound(format!(
                "application {application_kappa} cannot resolve {kappa} referenced by {edge} from embedded content or the local store"
            )),
            Self::ContentMismatch {
                kappa,
                edge,
                source,
            } => LiveError::InvalidHolo(format!(
                "application {application_kappa} resolved {kappa} for {edge} from {} but the bytes do not match the declared kappa",
                resolution_source_name(&source)
            )),
            Self::ProviderUnavailable {
                position,
                kind,
                entry,
                reason,
            } => LiveError::Capability(format!(
                "application {application_kappa} layer {position} ({kind}, entry {entry}) has no available provider: {reason}"
            )),
            Self::ChildClosureUnsupported {
                position,
                application_kappa: child,
                capabilities_kappa,
            } => LiveError::Capability(format!(
                "application {application_kappa} child {position} references application {child} with delegated capabilities {capabilities_kappa}; child closure execution is deferred until capability attenuation is implemented"
            )),
            Self::LimitExceeded {
                limit,
                maximum,
                actual,
                edge,
            } => LiveError::InvalidHolo(format!(
                "application {application_kappa} exceeds planning limit {limit}: maximum {maximum}, actual {actual}{}",
                edge.map_or_else(String::new, |edge| format!(" while resolving {edge}"))
            )),
            Self::ExecutionShapeUnsupported { reason } => LiveError::Capability(format!(
                "application {application_kappa} cannot execute with the current lifecycle: {reason}"
            )),
        }
    }
}

pub struct ProviderContext<'a> {
    pub position: u32,
    pub kind: LayerKind,
    pub entry: &'a str,
    pub aux: &'a str,
    pub primary: bool,
    pub layer_count: usize,
    pub content: &'a [u8],
}

pub struct ApplicationPlanReport {
    pub identity: HoloIdentity,
    pub primary_layer: Option<u32>,
    pub requires_kappa: String,
    pub layers: Vec<PlannedLayer>,
    pub children: Vec<ChildReference>,
    pub objects: BTreeMap<String, ResolvedObject>,
    pub blockers: Vec<PlanBlocker>,
    pub resolved_bytes: u64,
    manifest: AppManifest,
}

impl ApplicationPlanReport {
    pub fn evaluate_providers<F>(&mut self, mut evaluate: F)
    where
        F: FnMut(ProviderContext<'_>) -> ProviderAvailability,
    {
        let layer_count = self.layers.len();
        for layer in &mut self.layers {
            let Some(object) = self.objects.get(&layer.content_kappa) else {
                continue;
            };
            let availability = evaluate(ProviderContext {
                position: layer.position,
                kind: layer.kind,
                entry: &layer.entry,
                aux: &layer.aux,
                primary: layer.primary,
                layer_count,
                content: &object.bytes,
            });
            if let ProviderAvailability::Unavailable { reason } = &availability {
                self.blockers.push(PlanBlocker::ProviderUnavailable {
                    position: layer.position,
                    kind: layer_kind_name(layer.kind).to_owned(),
                    entry: layer.entry.clone(),
                    reason: reason.clone(),
                });
            }
            layer.provider = availability;
        }
    }

    pub fn require_single_primary(&mut self) {
        if self.primary_layer.is_none() {
            self.blockers.push(PlanBlocker::ExecutionShapeUnsupported {
                reason: "the manifest has no primary exit-bearing layer".to_owned(),
            });
        } else if self.layers.len() != 1 {
            self.blockers.push(PlanBlocker::ExecutionShapeUnsupported {
                reason: format!(
                    "multi-layer lifecycle is not connected yet; the manifest declares {} layers",
                    self.layers.len()
                ),
            });
        }
    }

    pub fn runnable(&self) -> bool {
        self.blockers.is_empty()
            && self
                .layers
                .iter()
                .all(|layer| matches!(layer.provider, ProviderAvailability::Available { .. }))
    }

    pub fn into_application_plan(self) -> Result<ApplicationPlan> {
        if let Some(blocker) = self.blockers.into_iter().next() {
            return Err(blocker.into_error(&self.identity.application_kappa));
        }
        let capabilities = self.objects.get(&self.requires_kappa).ok_or_else(|| {
            LiveError::Conflict("planner lost the resolved capability object".to_owned())
        })?;
        let mut layers = Vec::with_capacity(self.layers.len());
        for layer in self.layers {
            let object = self.objects.get(&layer.content_kappa).ok_or_else(|| {
                LiveError::Conflict(format!(
                    "planner lost resolved content {} for layer {}",
                    layer.content_kappa, layer.position
                ))
            })?;
            let provider = match layer.provider {
                ProviderAvailability::Available { provider } => provider,
                ProviderAvailability::Unchecked => {
                    return Err(LiveError::Capability(format!(
                        "provider availability was not evaluated for application {} layer {}",
                        self.identity.application_kappa, layer.position
                    )));
                }
                ProviderAvailability::Unavailable { reason } => {
                    return Err(LiveError::Capability(reason));
                }
            };
            layers.push(ResolvedLayer {
                position: layer.position,
                kind: layer.kind,
                content_kappa: layer.content_kappa,
                entry: layer.entry,
                aux: layer.aux,
                primary: layer.primary,
                content: object.bytes.clone(),
                resolution_source: object.source.clone(),
                provider,
            });
        }
        Ok(ApplicationPlan {
            identity: self.identity,
            primary_layer: self.primary_layer,
            requires_kappa: self.requires_kappa,
            required_capabilities: capabilities.bytes.clone(),
            layers,
            objects: self.objects,
            manifest: self.manifest,
        })
    }
}

pub struct ApplicationPlan {
    pub identity: HoloIdentity,
    pub primary_layer: Option<u32>,
    pub requires_kappa: String,
    pub required_capabilities: Arc<[u8]>,
    pub layers: Vec<ResolvedLayer>,
    pub objects: BTreeMap<String, ResolvedObject>,
    manifest: AppManifest,
}

impl std::fmt::Debug for ApplicationPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApplicationPlan")
            .field("identity", &self.identity)
            .field("primary_layer", &self.primary_layer)
            .field("requires_kappa", &self.requires_kappa)
            .field("layers", &self.layers)
            .field("objects", &self.objects)
            .finish_non_exhaustive()
    }
}

impl ApplicationPlan {
    pub fn primary(&self) -> Option<&ResolvedLayer> {
        self.primary_layer
            .and_then(|position| self.layers.get(position as usize))
    }

    pub fn verified_manifest(&self) -> &AppManifest {
        &self.manifest
    }
}

struct ObjectRequest {
    kappa: String,
    edges: Vec<ManifestEdge>,
}

pub fn explain_application<F>(
    archive_bytes: &[u8],
    limits: PlanLimits,
    mut resolve_local: F,
) -> Result<ApplicationPlanReport>
where
    F: FnMut(&str) -> Result<Option<Vec<u8>>>,
{
    let loader = HoloLoader::from_bytes(archive_bytes)
        .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    let archive_fingerprint = hex(&loader.fingerprint());
    let archive = loader
        .into_plan()
        .map_err(|error| LiveError::InvalidHolo(error.to_string()))?;
    let manifest_bytes = archive.app_manifest().ok_or_else(|| {
        LiveError::Capability("application planning requires an AppManifest section".to_owned())
    })?;
    let manifest = AppManifest::decode(manifest_bytes).map_err(|error| {
        LiveError::InvalidHolo(format!("decode application manifest: {error:?}"))
    })?;
    manifest.validate().map_err(|error| {
        LiveError::InvalidHolo(format!("validate application manifest: {error:?}"))
    })?;
    let canonical_manifest = manifest.canonicalize();
    let identity = HoloIdentity {
        archive_kappa: address_bytes(archive_bytes).to_string(),
        archive_fingerprint,
        application_kappa: address_bytes(&canonical_manifest).to_string(),
    };

    let mut blockers = Vec::new();
    if manifest.layers.len() > limits.max_layers {
        blockers.push(PlanBlocker::LimitExceeded {
            limit: "layers",
            maximum: limits.max_layers.try_into().unwrap_or(u64::MAX),
            actual: manifest.layers.len().try_into().unwrap_or(u64::MAX),
            edge: None,
        });
    }

    let layers = manifest
        .layers
        .iter()
        .enumerate()
        .map(|(position, layer)| PlannedLayer {
            position: position.try_into().unwrap_or(u32::MAX),
            kind: layer.kind,
            content_kappa: layer.content.to_string(),
            entry: layer.entry.clone(),
            aux: layer.aux.clone(),
            primary: manifest.primary == u32::try_from(position).ok(),
            resolution_source: None,
            provider: ProviderAvailability::Unchecked,
        })
        .collect::<Vec<_>>();
    let children = manifest
        .children
        .iter()
        .enumerate()
        .map(|(position, (application, capabilities))| ChildReference {
            position: position.try_into().unwrap_or(u32::MAX),
            application_kappa: application.to_string(),
            capabilities_kappa: capabilities.to_string(),
        })
        .collect::<Vec<_>>();
    blockers.extend(
        children
            .iter()
            .map(|child| PlanBlocker::ChildClosureUnsupported {
                position: child.position,
                application_kappa: child.application_kappa.clone(),
                capabilities_kappa: child.capabilities_kappa.clone(),
            }),
    );

    let mut requests = Vec::new();
    let mut request_indices = HashMap::new();
    add_request(
        &mut requests,
        &mut request_indices,
        manifest.requires.to_string(),
        ManifestEdge::RequiredCapabilities,
    );
    for layer in &layers {
        add_request(
            &mut requests,
            &mut request_indices,
            layer.content_kappa.clone(),
            ManifestEdge::Layer {
                position: layer.position,
            },
        );
    }
    if requests.len() > limits.max_objects {
        blockers.push(PlanBlocker::LimitExceeded {
            limit: "resolved_objects",
            maximum: limits.max_objects.try_into().unwrap_or(u64::MAX),
            actual: requests.len().try_into().unwrap_or(u64::MAX),
            edge: None,
        });
    }

    let embedded = archive
        .content_blobs()
        .map_err(|error| LiveError::InvalidHolo(error.to_string()))?
        .into_iter()
        .map(|(label, bytes)| {
            let kappa = std::str::from_utf8(label).map_err(|_| {
                LiveError::InvalidHolo("embedded content kappa is not UTF-8".to_owned())
            })?;
            Ok((kappa.to_owned(), bytes))
        })
        .collect::<Result<HashMap<_, _>>>()?;

    let mut objects = BTreeMap::new();
    let mut resolved_bytes = 0u64;
    if manifest.layers.len() <= limits.max_layers && requests.len() <= limits.max_objects {
        for request in &requests {
            let (bytes, source) = if let Some(bytes) = embedded.get(&request.kappa) {
                (Some(bytes.to_vec()), ResolutionSource::Embedded)
            } else {
                (resolve_local(&request.kappa)?, ResolutionSource::LocalStore)
            };
            let Some(bytes) = bytes else {
                blockers.extend(request.edges.iter().cloned().map(|edge| {
                    PlanBlocker::MissingObject {
                        kappa: request.kappa.clone(),
                        edge,
                    }
                }));
                continue;
            };
            if address_bytes(&bytes).to_string() != request.kappa {
                blockers.extend(request.edges.iter().cloned().map(|edge| {
                    PlanBlocker::ContentMismatch {
                        kappa: request.kappa.clone(),
                        edge,
                        source: source.clone(),
                    }
                }));
                continue;
            }
            let byte_length = bytes.len().try_into().unwrap_or(u64::MAX);
            let next_total = resolved_bytes.saturating_add(byte_length);
            if next_total > limits.max_resolved_bytes {
                blockers.push(PlanBlocker::LimitExceeded {
                    limit: "resolved_bytes",
                    maximum: limits.max_resolved_bytes,
                    actual: next_total,
                    edge: request.edges.first().cloned(),
                });
                break;
            }
            resolved_bytes = next_total;
            objects.insert(
                request.kappa.clone(),
                ResolvedObject {
                    kappa: request.kappa.clone(),
                    bytes: Arc::from(bytes),
                    source,
                },
            );
        }
    }

    let mut layers = layers;
    for layer in &mut layers {
        layer.resolution_source = objects
            .get(&layer.content_kappa)
            .map(|object| object.source.clone());
    }
    Ok(ApplicationPlanReport {
        identity,
        primary_layer: manifest.primary,
        requires_kappa: manifest.requires.to_string(),
        layers,
        children,
        objects,
        blockers,
        resolved_bytes,
        manifest,
    })
}

fn add_request(
    requests: &mut Vec<ObjectRequest>,
    indices: &mut HashMap<String, usize>,
    kappa: String,
    edge: ManifestEdge,
) {
    if let Some(index) = indices.get(&kappa).copied() {
        requests[index].edges.push(edge);
    } else {
        indices.insert(kappa.clone(), requests.len());
        requests.push(ObjectRequest {
            kappa,
            edges: vec![edge],
        });
    }
}

pub const fn layer_kind_name(kind: LayerKind) -> &'static str {
    match kind {
        LayerKind::WasmCodemodule => "wasm",
        LayerKind::TensorPlan => "tensor",
        LayerKind::RootfsImage => "rootfs",
        LayerKind::View => "view",
        LayerKind::InferenceModel => "inference-model",
    }
}

const fn resolution_source_name(source: &ResolutionSource) -> &str {
    match source {
        ResolutionSource::Embedded => "embedded content",
        ResolutionSource::LocalStore => "local store",
        ResolutionSource::ConfiguredResolver(_) => "configured resolver",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hologram::archive::HoloWriter;
    use hologram::space::{KappaLabel71, Layer};

    fn write_archive(manifest: &AppManifest, blobs: &[(&KappaLabel71, &[u8])]) -> Vec<u8> {
        let mut writer = HoloWriter::new();
        writer.set_app_manifest(manifest.canonicalize());
        for (kappa, bytes) in blobs {
            writer.add_content_blob(kappa.as_bytes(), *bytes);
        }
        writer.finish().expect("archive")
    }

    fn available(_: ProviderContext<'_>) -> ProviderAvailability {
        ProviderAvailability::Available {
            provider: "synthetic".to_owned(),
        }
    }

    #[test]
    fn preserves_multi_layer_order_and_deduplicates_equal_kappas() {
        let shared = b"shared layer bytes";
        let shared_kappa = address_bytes(shared);
        let manifest = AppManifest {
            primary: Some(0),
            requires: shared_kappa,
            layers: vec![
                Layer::wasm(shared_kappa, "primary"),
                Layer::wasm(shared_kappa, "secondary"),
            ],
            children: Vec::new(),
        };
        let bytes = write_archive(&manifest, &[(&shared_kappa, shared)]);

        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("plan");
        report.evaluate_providers(available);

        assert_eq!(report.layers.len(), 2);
        assert_eq!(report.layers[0].position, 0);
        assert_eq!(report.layers[0].entry, "primary");
        assert!(report.layers[0].primary);
        assert_eq!(report.layers[1].position, 1);
        assert_eq!(report.layers[1].entry, "secondary");
        assert!(!report.layers[1].primary);
        assert_eq!(report.objects.len(), 1);
        assert_eq!(report.resolved_bytes, shared.len() as u64);
        assert!(report.runnable());

        let plan = report.into_application_plan().expect("strict plan");
        assert_eq!(plan.layers.len(), 2);
        assert!(Arc::ptr_eq(
            &plan.layers[0].content,
            &plan.layers[1].content
        ));
        assert!(Arc::ptr_eq(
            &plan.required_capabilities,
            &plan.layers[0].content
        ));
    }

    #[test]
    fn missing_non_primary_content_names_kappa_and_layer_position() {
        let capabilities = b"capabilities";
        let primary = b"primary wasm";
        let secondary = b"secondary view";
        let capabilities_kappa = address_bytes(capabilities);
        let primary_kappa = address_bytes(primary);
        let secondary_kappa = address_bytes(secondary);
        let manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![
                Layer::wasm(primary_kappa, "run"),
                Layer::view(secondary_kappa, "portable"),
            ],
            children: Vec::new(),
        };
        let bytes = write_archive(
            &manifest,
            &[
                (&capabilities_kappa, capabilities),
                (&primary_kappa, primary),
            ],
        );

        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("explain");
        report.evaluate_providers(available);
        assert!(report.blockers.iter().any(|blocker| matches!(
            blocker,
            PlanBlocker::MissingObject {
                kappa,
                edge: ManifestEdge::Layer { position: 1 }
            } if kappa == &secondary_kappa.to_string()
        )));

        let error = report
            .into_application_plan()
            .expect_err("strict plan fails");
        assert_eq!(error.code(), "LIVE_NOT_FOUND");
        assert!(error.to_string().contains(&secondary_kappa.to_string()));
        assert!(error.to_string().contains("layer 1"));
    }

    #[test]
    fn forged_local_content_is_a_typed_mismatch_blocker() {
        let capabilities = b"capabilities";
        let layer = b"declared layer";
        let capabilities_kappa = address_bytes(capabilities);
        let layer_kappa = address_bytes(layer);
        let manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![Layer::wasm(layer_kappa, "run")],
            children: Vec::new(),
        };
        let bytes = write_archive(&manifest, &[]);
        let capabilities_key = capabilities_kappa.to_string();

        let report = explain_application(&bytes, PlanLimits::default(), |kappa| {
            if kappa == capabilities_key {
                Ok(Some(capabilities.to_vec()))
            } else {
                Ok(Some(b"forged".to_vec()))
            }
        })
        .expect("explain");
        assert!(report.blockers.iter().any(|blocker| matches!(
            blocker,
            PlanBlocker::ContentMismatch {
                kappa,
                edge: ManifestEdge::Layer { position: 0 },
                source: ResolutionSource::LocalStore,
            } if kappa == &layer_kappa.to_string()
        )));
        let error = report
            .into_application_plan()
            .expect_err("strict plan fails");
        assert_eq!(error.code(), "LIVE_HOLO_INVALID");
        assert!(error.to_string().contains("layer 0"));
    }

    #[test]
    fn fat_and_cache_resolved_thin_archives_have_equivalent_logical_plans() {
        let capabilities = b"capabilities";
        let layer = b"portable wasm";
        let capabilities_kappa = address_bytes(capabilities);
        let layer_kappa = address_bytes(layer);
        let manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![Layer::wasm(layer_kappa, "run")],
            children: Vec::new(),
        };
        let fat = write_archive(
            &manifest,
            &[(&capabilities_kappa, capabilities), (&layer_kappa, layer)],
        );
        let thin = write_archive(&manifest, &[]);
        let capabilities_key = capabilities_kappa.to_string();
        let layer_key = layer_kappa.to_string();

        let fat_report =
            explain_application(&fat, PlanLimits::default(), |_| Ok(None)).expect("fat plan");
        let thin_report = explain_application(&thin, PlanLimits::default(), |kappa| {
            if kappa == capabilities_key {
                Ok(Some(capabilities.to_vec()))
            } else if kappa == layer_key {
                Ok(Some(layer.to_vec()))
            } else {
                Ok(None)
            }
        })
        .expect("thin plan");

        assert_ne!(
            fat_report.identity.archive_kappa,
            thin_report.identity.archive_kappa
        );
        assert_eq!(
            fat_report.identity.application_kappa,
            thin_report.identity.application_kappa
        );
        assert_eq!(fat_report.primary_layer, thin_report.primary_layer);
        assert_eq!(fat_report.requires_kappa, thin_report.requires_kappa);
        assert_eq!(fat_report.layers[0].kind, thin_report.layers[0].kind);
        assert_eq!(fat_report.layers[0].entry, thin_report.layers[0].entry);
        assert_eq!(
            fat_report.objects[&layer_kappa.to_string()].bytes,
            thin_report.objects[&layer_kappa.to_string()].bytes
        );
        assert_eq!(
            fat_report.layers[0].resolution_source,
            Some(ResolutionSource::Embedded)
        );
        assert_eq!(
            thin_report.layers[0].resolution_source,
            Some(ResolutionSource::LocalStore)
        );
    }

    #[test]
    fn unsupported_service_provider_is_explainable_but_not_executable() {
        let capabilities = b"capabilities";
        let model = b"model bundle";
        let capabilities_kappa = address_bytes(capabilities);
        let model_kappa = address_bytes(model);
        let manifest = AppManifest {
            primary: None,
            requires: capabilities_kappa,
            layers: vec![Layer::inference_model(model_kappa, "ai.default", "uor-r4")],
            children: Vec::new(),
        };
        let bytes = write_archive(
            &manifest,
            &[(&capabilities_kappa, capabilities), (&model_kappa, model)],
        );

        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("explain");
        report.evaluate_providers(|context| ProviderAvailability::Unavailable {
            reason: format!(
                "provider {} is not connected for {}",
                context.aux, context.entry
            ),
        });
        assert!(!report.runnable());
        assert!(matches!(
            report.layers[0].provider,
            ProviderAvailability::Unavailable { .. }
        ));

        let error = report
            .into_application_plan()
            .expect_err("strict plan fails");
        assert_eq!(error.code(), "LIVE_CAPABILITY_MISSING");
        assert!(error.to_string().contains("ai.default"));
        assert!(error.to_string().contains("uor-r4"));
    }

    #[test]
    fn child_references_are_visible_blockers_until_attenuation_lands() {
        let capabilities = b"capabilities";
        let layer = b"wasm";
        let capabilities_kappa = address_bytes(capabilities);
        let layer_kappa = address_bytes(layer);
        let child_kappa = address_bytes(b"child application");
        let child_capabilities = address_bytes(b"delegated capabilities");
        let manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![Layer::wasm(layer_kappa, "run")],
            children: vec![(child_kappa, child_capabilities)],
        };
        let bytes = write_archive(
            &manifest,
            &[(&capabilities_kappa, capabilities), (&layer_kappa, layer)],
        );

        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("explain");
        report.evaluate_providers(available);
        assert_eq!(report.children.len(), 1);
        assert!(report.blockers.iter().any(|blocker| matches!(
            blocker,
            PlanBlocker::ChildClosureUnsupported {
                position: 0,
                application_kappa,
                capabilities_kappa,
            } if application_kappa == &child_kappa.to_string()
                && capabilities_kappa == &child_capabilities.to_string()
        )));
        assert_eq!(
            report
                .into_application_plan()
                .expect_err("child blocker")
                .code(),
            "LIVE_CAPABILITY_MISSING"
        );
    }

    #[test]
    fn root_plan_limits_block_before_strict_execution() {
        let capabilities = b"capabilities";
        let layer = b"wasm payload";
        let capabilities_kappa = address_bytes(capabilities);
        let layer_kappa = address_bytes(layer);
        let manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![Layer::wasm(layer_kappa, "run")],
            children: Vec::new(),
        };
        let bytes = write_archive(
            &manifest,
            &[(&capabilities_kappa, capabilities), (&layer_kappa, layer)],
        );

        let layer_limited = explain_application(
            &bytes,
            PlanLimits {
                max_layers: 0,
                ..PlanLimits::default()
            },
            |_| Ok(None),
        )
        .expect("layer-limit report");
        assert!(matches!(
            layer_limited.blockers[0],
            PlanBlocker::LimitExceeded {
                limit: "layers",
                ..
            }
        ));

        let object_limited = explain_application(
            &bytes,
            PlanLimits {
                max_objects: 1,
                ..PlanLimits::default()
            },
            |_| Ok(None),
        )
        .expect("object-limit report");
        assert!(matches!(
            object_limited.blockers[0],
            PlanBlocker::LimitExceeded {
                limit: "resolved_objects",
                ..
            }
        ));

        let byte_limited = explain_application(
            &bytes,
            PlanLimits {
                max_resolved_bytes: (capabilities.len() - 1) as u64,
                ..PlanLimits::default()
            },
            |_| Ok(None),
        )
        .expect("byte-limit report");
        assert!(matches!(
            byte_limited.blockers[0],
            PlanBlocker::LimitExceeded {
                limit: "resolved_bytes",
                edge: Some(ManifestEdge::RequiredCapabilities),
                ..
            }
        ));
    }
}
