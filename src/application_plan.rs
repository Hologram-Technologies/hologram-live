use crate::error::{LiveError, Result};
use crate::holo_capability::{
    authorize_child_delegation, CapabilityDecision, DelegatedCapabilities, EffectiveGrant,
    RequestedCapabilities,
};
use crate::util::hex;
use hologram::archive::HoloLoader;
use hologram::space::{address_bytes, AppManifest, LayerKind, Realization};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
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
    pub max_applications: usize,
    pub max_depth: usize,
    pub max_objects: usize,
    pub max_resolved_bytes: u64,
}

impl Default for PlanLimits {
    fn default() -> Self {
        Self {
            max_layers: 256,
            max_applications: 64,
            max_depth: 16,
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
    Layer {
        position: u32,
    },
    ChildApplication {
        parent_application_kappa: String,
        position: u32,
    },
    DelegatedCapabilities {
        parent_application_kappa: String,
        position: u32,
    },
    ChildRequiredCapabilities {
        application_kappa: String,
    },
    ChildLayer {
        application_kappa: String,
        position: u32,
    },
}

impl std::fmt::Display for ManifestEdge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RequiredCapabilities => formatter.write_str("required capabilities"),
            Self::Layer { position } => write!(formatter, "layer {position}"),
            Self::ChildApplication {
                parent_application_kappa,
                position,
            } => write!(
                formatter,
                "child {position} application of {parent_application_kappa}"
            ),
            Self::DelegatedCapabilities {
                parent_application_kappa,
                position,
            } => write!(
                formatter,
                "child {position} delegated capabilities of {parent_application_kappa}"
            ),
            Self::ChildRequiredCapabilities { application_kappa } => {
                write!(
                    formatter,
                    "required capabilities of child {application_kappa}"
                )
            }
            Self::ChildLayer {
                application_kappa,
                position,
            } => write!(formatter, "layer {position} of child {application_kappa}"),
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
    pub parent_application_kappa: String,
    pub application_kappa: String,
    pub capabilities_kappa: String,
    pub depth: u32,
    pub requires_kappa: Option<String>,
    pub layer_count: Option<usize>,
    pub application_resolution_source: Option<ResolutionSource>,
    pub capabilities_resolution_source: Option<ResolutionSource>,
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
    InvalidCapabilitySet {
        kappa: String,
        reason: String,
    },
    ProviderUnavailable {
        application_kappa: String,
        position: u32,
        kind: String,
        entry: String,
        reason: String,
    },
    InvalidChildManifest {
        kappa: String,
        edge: ManifestEdge,
        reason: String,
    },
    ChildCycle {
        path: Vec<String>,
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
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::MissingObject { .. } => "missing_object",
            Self::ContentMismatch { .. } => "content_mismatch",
            Self::InvalidCapabilitySet { .. } => "invalid_capability_set",
            Self::ProviderUnavailable { .. } => "provider_unavailable",
            Self::InvalidChildManifest { .. } => "invalid_child_manifest",
            Self::ChildCycle { .. } => "child_cycle",
            Self::LimitExceeded { .. } => "limit_exceeded",
            Self::ExecutionShapeUnsupported { .. } => "execution_shape_unsupported",
        }
    }

    pub const fn error_code(&self) -> &'static str {
        match self {
            Self::MissingObject { .. } => "LIVE_NOT_FOUND",
            Self::ContentMismatch { .. }
            | Self::InvalidCapabilitySet { .. }
            | Self::InvalidChildManifest { .. }
            | Self::ChildCycle { .. }
            | Self::LimitExceeded { .. } => "LIVE_HOLO_INVALID",
            Self::ProviderUnavailable { .. } | Self::ExecutionShapeUnsupported { .. } => {
                "LIVE_CAPABILITY_MISSING"
            }
        }
    }

    pub fn message(&self, application_kappa: &str) -> String {
        match self {
            Self::MissingObject { kappa, edge } => format!(
                "application {application_kappa} cannot resolve {kappa} referenced by {edge} from embedded content or the local store"
            ),
            Self::ContentMismatch {
                kappa,
                edge,
                source,
            } => format!(
                "application {application_kappa} resolved {kappa} for {edge} from {} but the bytes do not match the declared kappa",
                resolution_source_name(source)
            ),
            Self::InvalidCapabilitySet { kappa, reason } => format!(
                "application {application_kappa} required capability object {kappa} is not a canonical CapabilitySet: {reason}"
            ),
            Self::ProviderUnavailable {
                application_kappa: provider_application,
                position,
                kind,
                entry,
                reason,
            } => format!(
                "application {provider_application} layer {position} ({kind}, entry {entry}) has no available provider while planning root {application_kappa}: {reason}"
            ),
            Self::InvalidChildManifest {
                kappa,
                edge,
                reason,
            } => format!(
                "application {application_kappa} resolved invalid child manifest {kappa} referenced by {edge}: {reason}"
            ),
            Self::ChildCycle { path } => format!(
                "application {application_kappa} child graph contains a cycle: {}",
                path.join(" -> ")
            ),
            Self::LimitExceeded {
                limit,
                maximum,
                actual,
                edge,
            } => format!(
                "application {application_kappa} exceeds planning limit {limit}: maximum {maximum}, actual {actual}{}",
                edge.as_ref().map_or_else(String::new, |edge| format!(" while resolving {edge}"))
            ),
            Self::ExecutionShapeUnsupported { reason } => format!(
                "application {application_kappa} cannot execute with the current lifecycle: {reason}"
            ),
        }
    }

    fn into_error(self, application_kappa: &str) -> LiveError {
        let message = self.message(application_kappa);
        match self.error_code() {
            "LIVE_NOT_FOUND" => LiveError::NotFound(message),
            "LIVE_HOLO_INVALID" => LiveError::InvalidHolo(message),
            _ => LiveError::Capability(message),
        }
    }
}

pub struct ProviderContext<'a> {
    pub application_kappa: &'a str,
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
    pub referenced_object_count: usize,
    pub embedded_object_count: usize,
    pub application_count: usize,
    pub max_depth: usize,
    pub limits: PlanLimits,
    requested_capabilities: Option<RequestedCapabilities>,
    child_admissions: Vec<PendingChildAdmission>,
    manifest: AppManifest,
}

struct PendingChildAdmission {
    position: u32,
    parent_application_index: usize,
    parent_application_kappa: String,
    application_index: usize,
    application_kappa: String,
    delegated_capabilities: DelegatedCapabilities,
    requested_capabilities: Option<RequestedCapabilities>,
    primary_layer: Option<u32>,
    layers: Vec<PlannedLayer>,
}

#[derive(Debug, Clone)]
pub struct ResolvedChild {
    pub position: u32,
    pub parent_application_index: usize,
    pub parent_application_kappa: String,
    pub application_index: usize,
    pub application_kappa: String,
    pub delegated_capabilities: DelegatedCapabilities,
    pub requested_capabilities: RequestedCapabilities,
    pub primary_layer: Option<u32>,
    pub layers: Vec<ResolvedLayer>,
}

impl ApplicationPlanReport {
    pub fn evaluate_providers<F>(&mut self, mut evaluate: F)
    where
        F: FnMut(ProviderContext<'_>) -> ProviderAvailability,
    {
        evaluate_layer_set(
            &self.identity.application_kappa,
            &mut self.layers,
            &self.objects,
            &mut self.blockers,
            &mut evaluate,
        );
        for child in &mut self.child_admissions {
            evaluate_layer_set(
                &child.application_kappa,
                &mut child.layers,
                &self.objects,
                &mut self.blockers,
                &mut evaluate,
            );
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
            && self.child_admissions.iter().all(|child| {
                child
                    .layers
                    .iter()
                    .all(|layer| matches!(layer.provider, ProviderAvailability::Available { .. }))
            })
    }

    pub fn into_application_plan(self) -> Result<ApplicationPlan> {
        if let Some(blocker) = self.blockers.into_iter().next() {
            return Err(blocker.into_error(&self.identity.application_kappa));
        }
        let requested_capabilities = self.requested_capabilities.ok_or_else(|| {
            LiveError::Conflict("planner lost decoded requested capabilities".to_owned())
        })?;
        let layers = strict_layers(&self.identity.application_kappa, self.layers, &self.objects)?;
        let children = self
            .child_admissions
            .into_iter()
            .map(|child| {
                let application_kappa = child.application_kappa;
                let layers = strict_layers(&application_kappa, child.layers, &self.objects)?;
                Ok(ResolvedChild {
                    position: child.position,
                    parent_application_index: child.parent_application_index,
                    parent_application_kappa: child.parent_application_kappa,
                    application_index: child.application_index,
                    application_kappa,
                    delegated_capabilities: child.delegated_capabilities,
                    requested_capabilities: child.requested_capabilities.ok_or_else(|| {
                        LiveError::Conflict(
                            "planner lost decoded child requested capabilities".to_owned(),
                        )
                    })?,
                    primary_layer: child.primary_layer,
                    layers,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(ApplicationPlan {
            identity: self.identity,
            primary_layer: self.primary_layer,
            requires_kappa: self.requires_kappa,
            requested_capabilities,
            layers,
            children,
            objects: self.objects,
            manifest: self.manifest,
        })
    }
}

fn evaluate_layer_set<F>(
    application_kappa: &str,
    layers: &mut [PlannedLayer],
    objects: &BTreeMap<String, ResolvedObject>,
    blockers: &mut Vec<PlanBlocker>,
    evaluate: &mut F,
) where
    F: FnMut(ProviderContext<'_>) -> ProviderAvailability,
{
    let layer_count = layers.len();
    for layer in layers {
        let Some(object) = objects.get(&layer.content_kappa) else {
            continue;
        };
        let availability = evaluate(ProviderContext {
            application_kappa,
            position: layer.position,
            kind: layer.kind,
            entry: &layer.entry,
            aux: &layer.aux,
            primary: layer.primary,
            layer_count,
            content: &object.bytes,
        });
        if let ProviderAvailability::Unavailable { reason } = &availability {
            blockers.push(PlanBlocker::ProviderUnavailable {
                application_kappa: application_kappa.to_owned(),
                position: layer.position,
                kind: layer_kind_name(layer.kind).to_owned(),
                entry: layer.entry.clone(),
                reason: reason.clone(),
            });
        }
        layer.provider = availability;
    }
}

fn strict_layers(
    application_kappa: &str,
    layers: Vec<PlannedLayer>,
    objects: &BTreeMap<String, ResolvedObject>,
) -> Result<Vec<ResolvedLayer>> {
    layers
        .into_iter()
        .map(|layer| {
            let object = objects.get(&layer.content_kappa).ok_or_else(|| {
                LiveError::Conflict(format!(
                    "planner lost resolved content {} for application {application_kappa} layer {}",
                    layer.content_kappa, layer.position
                ))
            })?;
            let provider = match layer.provider {
                ProviderAvailability::Available { provider } => provider,
                ProviderAvailability::Unchecked => {
                    return Err(LiveError::Capability(format!(
                        "provider availability was not evaluated for application {application_kappa} layer {}",
                        layer.position
                    )));
                }
                ProviderAvailability::Unavailable { reason } => {
                    return Err(LiveError::Capability(reason));
                }
            };
            Ok(ResolvedLayer {
                position: layer.position,
                kind: layer.kind,
                content_kappa: layer.content_kappa,
                entry: layer.entry,
                aux: layer.aux,
                primary: layer.primary,
                content: object.bytes.clone(),
                resolution_source: object.source.clone(),
                provider,
            })
        })
        .collect()
}

pub struct ApplicationPlan {
    pub identity: HoloIdentity,
    pub primary_layer: Option<u32>,
    pub requires_kappa: String,
    pub requested_capabilities: RequestedCapabilities,
    pub layers: Vec<ResolvedLayer>,
    pub children: Vec<ResolvedChild>,
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
            .field("children", &self.children)
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

    pub fn authorize_capability_tree(&self, effective_grant: &EffectiveGrant) -> Result<()> {
        self.admitted_grants(effective_grant).map(|_| ())
    }

    pub fn admitted_grants(
        &self,
        effective_grant: &EffectiveGrant,
    ) -> Result<HashMap<usize, EffectiveGrant>> {
        self.admitted_grants_with(effective_grant, |_| {})
    }

    pub fn admitted_grants_with<F>(
        &self,
        effective_grant: &EffectiveGrant,
        mut observe: F,
    ) -> Result<HashMap<usize, EffectiveGrant>>
    where
        F: FnMut(CapabilityDecision),
    {
        let root_allowed = effective_grant
            .capabilities
            .admits(&self.requested_capabilities.capabilities);
        observe(CapabilityDecision::application_request(
            &self.identity.application_kappa,
            None,
            &self.requested_capabilities,
            effective_grant,
            root_allowed,
        ));
        effective_grant.authorize(
            &self.identity.application_kappa,
            &self.requested_capabilities,
        )?;
        let mut grants = HashMap::from([(0usize, effective_grant.clone())]);
        for child in &self.children {
            let parent_grant = grants.get(&child.parent_application_index).ok_or_else(|| {
                LiveError::Conflict(format!(
                    "planner lost parent application {} for child {}",
                    child.parent_application_kappa, child.application_kappa
                ))
            })?;
            let delegation_allowed = parent_grant
                .capabilities
                .admits(&child.delegated_capabilities.capabilities);
            observe(CapabilityDecision::child_delegation(
                &child.parent_application_kappa,
                &child.application_kappa,
                &child.delegated_capabilities,
                parent_grant,
                delegation_allowed,
            ));
            if delegation_allowed {
                let child_grant = EffectiveGrant::from_delegation(&child.delegated_capabilities);
                let request_allowed = child_grant
                    .capabilities
                    .admits(&child.requested_capabilities.capabilities);
                observe(CapabilityDecision::application_request(
                    &child.application_kappa,
                    Some(&child.parent_application_kappa),
                    &child.requested_capabilities,
                    &child_grant,
                    request_allowed,
                ));
            }
            authorize_child_delegation(
                &child.parent_application_kappa,
                &parent_grant.kappa,
                &parent_grant.capabilities,
                &child.application_kappa,
                &child.delegated_capabilities,
                &child.requested_capabilities,
            )?;
            grants.insert(
                child.application_index,
                EffectiveGrant::from_delegation(&child.delegated_capabilities),
            );
        }
        Ok(grants)
    }
}

#[derive(Clone)]
enum ResolutionFailure {
    Missing,
    Mismatch { source: ResolutionSource },
}

struct ClosureResolver<'a, F> {
    embedded: HashMap<String, &'a [u8]>,
    resolve_local: &'a mut F,
    limits: PlanLimits,
    referenced: HashSet<String>,
    failures: HashMap<String, ResolutionFailure>,
    objects: BTreeMap<String, ResolvedObject>,
    blockers: Vec<PlanBlocker>,
    resolved_bytes: u64,
    embedded_object_count: usize,
    halted: bool,
}

impl<F> ClosureResolver<'_, F>
where
    F: FnMut(&str) -> Result<Option<Vec<u8>>>,
{
    fn resolve(&mut self, kappa: &str, edge: ManifestEdge) -> Result<Option<ResolvedObject>> {
        if let Some(object) = self.objects.get(kappa) {
            return Ok(Some(object.clone()));
        }
        if let Some(failure) = self.failures.get(kappa) {
            self.blockers.push(match failure {
                ResolutionFailure::Missing => PlanBlocker::MissingObject {
                    kappa: kappa.to_owned(),
                    edge,
                },
                ResolutionFailure::Mismatch { source } => PlanBlocker::ContentMismatch {
                    kappa: kappa.to_owned(),
                    edge,
                    source: source.clone(),
                },
            });
            return Ok(None);
        }
        if self.halted {
            return Ok(None);
        }
        if !self.referenced.contains(kappa) {
            let actual = self.referenced.len().saturating_add(1);
            if actual > self.limits.max_objects {
                self.blockers.push(PlanBlocker::LimitExceeded {
                    limit: "resolved_objects",
                    maximum: self.limits.max_objects.try_into().unwrap_or(u64::MAX),
                    actual: actual.try_into().unwrap_or(u64::MAX),
                    edge: Some(edge),
                });
                self.halted = true;
                return Ok(None);
            }
            self.referenced.insert(kappa.to_owned());
            if self.embedded.contains_key(kappa) {
                self.embedded_object_count = self.embedded_object_count.saturating_add(1);
            }
        }

        let (bytes, source) = if let Some(bytes) = self.embedded.get(kappa) {
            (Some(bytes.to_vec()), ResolutionSource::Embedded)
        } else {
            ((self.resolve_local)(kappa)?, ResolutionSource::LocalStore)
        };
        let Some(bytes) = bytes else {
            self.failures
                .insert(kappa.to_owned(), ResolutionFailure::Missing);
            self.blockers.push(PlanBlocker::MissingObject {
                kappa: kappa.to_owned(),
                edge,
            });
            return Ok(None);
        };
        if address_bytes(&bytes) != kappa {
            self.failures.insert(
                kappa.to_owned(),
                ResolutionFailure::Mismatch {
                    source: source.clone(),
                },
            );
            self.blockers.push(PlanBlocker::ContentMismatch {
                kappa: kappa.to_owned(),
                edge,
                source,
            });
            return Ok(None);
        }
        let byte_length = bytes.len().try_into().unwrap_or(u64::MAX);
        let next_total = self.resolved_bytes.saturating_add(byte_length);
        if next_total > self.limits.max_resolved_bytes {
            self.blockers.push(PlanBlocker::LimitExceeded {
                limit: "resolved_bytes",
                maximum: self.limits.max_resolved_bytes,
                actual: next_total,
                edge: Some(edge),
            });
            self.halted = true;
            return Ok(None);
        }
        self.resolved_bytes = next_total;
        let object = ResolvedObject {
            kappa: kappa.to_owned(),
            bytes: Arc::from(bytes),
            source,
        };
        self.objects.insert(kappa.to_owned(), object.clone());
        Ok(Some(object))
    }
}

struct PendingApplication {
    application_index: usize,
    admission_index: Option<usize>,
    application_kappa: String,
    manifest: AppManifest,
    depth: usize,
    path: Vec<String>,
}

fn planned_layers(manifest: &AppManifest) -> Vec<PlannedLayer> {
    manifest
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
        .collect()
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
    tracing::info!(
        application_kappa = %identity.application_kappa,
        archive_kappa = %identity.archive_kappa,
        layer_count = manifest.layers.len(),
        child_count = manifest.children.len(),
        lifecycle_phase = "plan",
        "planning holo application"
    );

    let layers = planned_layers(&manifest);

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
    let mut closure = ClosureResolver {
        embedded,
        resolve_local: &mut resolve_local,
        limits,
        referenced: HashSet::new(),
        failures: HashMap::new(),
        objects: BTreeMap::new(),
        blockers: Vec::new(),
        resolved_bytes: 0,
        embedded_object_count: 0,
        halted: false,
    };
    let root_application_kappa = identity.application_kappa.clone();
    let root_plan_manifest = AppManifest::decode(&canonical_manifest).map_err(|error| {
        LiveError::InvalidHolo(format!("decode canonical application manifest: {error:?}"))
    })?;
    let mut pending = VecDeque::from([PendingApplication {
        application_index: 0,
        admission_index: None,
        application_kappa: root_application_kappa.clone(),
        manifest: root_plan_manifest,
        depth: 0,
        path: vec![root_application_kappa.clone()],
    }]);
    let mut children = Vec::new();
    let mut child_admissions: Vec<PendingChildAdmission> = Vec::new();
    let mut requested_capabilities = None;
    let mut application_count = 1usize;
    let mut max_depth = 0usize;
    let mut total_layers = 0usize;
    let mut applications_limited = false;
    if application_count > limits.max_applications {
        closure.blockers.push(PlanBlocker::LimitExceeded {
            limit: "applications",
            maximum: limits.max_applications.try_into().unwrap_or(u64::MAX),
            actual: application_count.try_into().unwrap_or(u64::MAX),
            edge: None,
        });
        applications_limited = true;
        pending.clear();
    }

    while let Some(application) = pending.pop_front() {
        if let Some(index) = application.admission_index {
            child_admissions[index].primary_layer = application.manifest.primary;
            child_admissions[index].layers = planned_layers(&application.manifest);
        }
        total_layers = total_layers.saturating_add(application.manifest.layers.len());
        if total_layers > limits.max_layers {
            closure.blockers.push(PlanBlocker::LimitExceeded {
                limit: "layers",
                maximum: limits.max_layers.try_into().unwrap_or(u64::MAX),
                actual: total_layers.try_into().unwrap_or(u64::MAX),
                edge: None,
            });
            break;
        }

        let requires_kappa = application.manifest.requires.to_string();
        let requires_edge = if application.depth == 0 {
            ManifestEdge::RequiredCapabilities
        } else {
            ManifestEdge::ChildRequiredCapabilities {
                application_kappa: application.application_kappa.clone(),
            }
        };
        if let Some(object) = closure.resolve(&requires_kappa, requires_edge)? {
            match RequestedCapabilities::decode(&object.kappa, object.bytes.clone()) {
                Ok(capabilities) if application.depth == 0 => {
                    requested_capabilities = Some(capabilities);
                }
                Ok(capabilities) => {
                    if let Some(index) = application.admission_index {
                        child_admissions[index].requested_capabilities = Some(capabilities);
                    }
                }
                Err(error) => closure.blockers.push(PlanBlocker::InvalidCapabilitySet {
                    kappa: object.kappa,
                    reason: error.to_string(),
                }),
            }
        }
        for (position, layer) in application.manifest.layers.iter().enumerate() {
            let edge = if application.depth == 0 {
                ManifestEdge::Layer {
                    position: position.try_into().unwrap_or(u32::MAX),
                }
            } else {
                ManifestEdge::ChildLayer {
                    application_kappa: application.application_kappa.clone(),
                    position: position.try_into().unwrap_or(u32::MAX),
                }
            };
            let _ = closure.resolve(layer.content.as_ref(), edge)?;
        }
        if closure.halted {
            break;
        }

        for (position, (child_application, delegated_capabilities)) in
            application.manifest.children.iter().enumerate()
        {
            let position = position.try_into().unwrap_or(u32::MAX);
            let child_kappa = child_application.to_string();
            let capabilities_kappa = delegated_capabilities.to_string();
            let child_depth = application.depth.saturating_add(1);
            let application_edge = ManifestEdge::ChildApplication {
                parent_application_kappa: application.application_kappa.clone(),
                position,
            };
            let capabilities_edge = ManifestEdge::DelegatedCapabilities {
                parent_application_kappa: application.application_kappa.clone(),
                position,
            };
            children.push(ChildReference {
                position,
                parent_application_kappa: application.application_kappa.clone(),
                application_kappa: child_kappa.clone(),
                capabilities_kappa: capabilities_kappa.clone(),
                depth: child_depth.try_into().unwrap_or(u32::MAX),
                requires_kappa: None,
                layer_count: None,
                application_resolution_source: None,
                capabilities_resolution_source: None,
            });
            let child_index = children.len() - 1;

            if let Some(path) = child_cycle_path(&application.path, &child_kappa) {
                closure.blockers.push(PlanBlocker::ChildCycle { path });
                continue;
            }
            if child_depth > limits.max_depth {
                closure.blockers.push(PlanBlocker::LimitExceeded {
                    limit: "application_depth",
                    maximum: limits.max_depth.try_into().unwrap_or(u64::MAX),
                    actual: child_depth.try_into().unwrap_or(u64::MAX),
                    edge: Some(application_edge),
                });
                continue;
            }
            if applications_limited || application_count >= limits.max_applications {
                if !applications_limited {
                    closure.blockers.push(PlanBlocker::LimitExceeded {
                        limit: "applications",
                        maximum: limits.max_applications.try_into().unwrap_or(u64::MAX),
                        actual: application_count
                            .saturating_add(1)
                            .try_into()
                            .unwrap_or(u64::MAX),
                        edge: Some(application_edge),
                    });
                    applications_limited = true;
                }
                continue;
            }
            let child_application_index = application_count;
            application_count = application_count.saturating_add(1);
            max_depth = max_depth.max(child_depth);

            let delegated = closure.resolve(&capabilities_kappa, capabilities_edge)?;
            let delegated = delegated.and_then(|object| {
                children[child_index].capabilities_resolution_source = Some(object.source.clone());
                match DelegatedCapabilities::decode(&object.kappa, object.bytes.clone()) {
                    Ok(capabilities) => Some(capabilities),
                    Err(error) => {
                        closure.blockers.push(PlanBlocker::InvalidCapabilitySet {
                            kappa: object.kappa.clone(),
                            reason: error.to_string(),
                        });
                        None
                    }
                }
            });
            let child_object = closure.resolve(&child_kappa, application_edge.clone())?;
            let Some(child_object) = child_object else {
                continue;
            };
            children[child_index].application_resolution_source = Some(child_object.source.clone());
            let child_manifest = match decode_child_manifest(&child_object) {
                Ok(manifest) => manifest,
                Err(reason) => {
                    closure.blockers.push(PlanBlocker::InvalidChildManifest {
                        kappa: child_kappa,
                        edge: application_edge,
                        reason,
                    });
                    continue;
                }
            };
            children[child_index].requires_kappa = Some(child_manifest.requires.to_string());
            children[child_index].layer_count = Some(child_manifest.layers.len());
            let admission_index = delegated.map(|delegated_capabilities| {
                child_admissions.push(PendingChildAdmission {
                    position,
                    parent_application_index: application.application_index,
                    parent_application_kappa: application.application_kappa.clone(),
                    application_index: child_application_index,
                    application_kappa: child_object.kappa.clone(),
                    delegated_capabilities,
                    requested_capabilities: None,
                    primary_layer: None,
                    layers: Vec::new(),
                });
                child_admissions.len() - 1
            });
            let mut path = application.path.clone();
            path.push(child_object.kappa.clone());
            pending.push_back(PendingApplication {
                application_index: child_application_index,
                admission_index,
                application_kappa: child_object.kappa,
                manifest: child_manifest,
                depth: child_depth,
                path,
            });
        }
        if closure.halted {
            break;
        }
    }

    let mut layers = layers;
    for layer in &mut layers {
        layer.resolution_source = closure
            .objects
            .get(&layer.content_kappa)
            .map(|object| object.source.clone());
    }
    for child in &mut child_admissions {
        for layer in &mut child.layers {
            layer.resolution_source = closure
                .objects
                .get(&layer.content_kappa)
                .map(|object| object.source.clone());
        }
    }
    Ok(ApplicationPlanReport {
        identity,
        primary_layer: manifest.primary,
        requires_kappa: manifest.requires.to_string(),
        layers,
        children,
        objects: closure.objects,
        blockers: closure.blockers,
        resolved_bytes: closure.resolved_bytes,
        referenced_object_count: closure.referenced.len(),
        embedded_object_count: closure.embedded_object_count,
        application_count,
        max_depth,
        limits,
        requested_capabilities,
        child_admissions,
        manifest,
    })
}

fn decode_child_manifest(object: &ResolvedObject) -> std::result::Result<AppManifest, String> {
    let manifest =
        AppManifest::decode(&object.bytes).map_err(|error| format!("decode: {error:?}"))?;
    manifest
        .validate()
        .map_err(|error| format!("validate: {error:?}"))?;
    if manifest.canonicalize().as_slice() != object.bytes.as_ref() {
        return Err("manifest is not canonically encoded".to_owned());
    }
    Ok(manifest)
}

fn child_cycle_path(path: &[String], child_kappa: &str) -> Option<Vec<String>> {
    path.iter()
        .any(|ancestor| ancestor == child_kappa)
        .then(|| {
            let mut cycle = path.to_vec();
            cycle.push(child_kappa.to_owned());
            cycle
        })
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

    fn test_capabilities() -> &'static [u8] {
        static CAPABILITIES: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
        CAPABILITIES.get_or_init(crate::holo_capability::empty_canonical)
    }

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
        let shared = test_capabilities();
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
            &plan.requested_capabilities.canonical,
            &plan.layers[0].content
        ));
    }

    #[test]
    fn missing_non_primary_content_names_kappa_and_layer_position() {
        let capabilities = test_capabilities();
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
    fn malformed_capability_object_is_an_explainable_typed_blocker() {
        let capabilities = b"not a canonical capability set";
        let layer = b"wasm";
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

        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("explain");
        report.evaluate_providers(available);

        assert!(!report.runnable());
        assert!(report.blockers.iter().any(|blocker| matches!(
            blocker,
            PlanBlocker::InvalidCapabilitySet { kappa, .. }
                if kappa == &capabilities_kappa.to_string()
        )));
        let error = report
            .into_application_plan()
            .expect_err("invalid capabilities block execution");
        assert_eq!(error.code(), "LIVE_HOLO_INVALID");
        assert!(error.to_string().contains("CapabilitySet"), "{error}");
    }

    #[test]
    fn forged_local_content_is_a_typed_mismatch_blocker() {
        let capabilities = test_capabilities();
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
        let capabilities = test_capabilities();
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

        let mut fat_report =
            explain_application(&fat, PlanLimits::default(), |_| Ok(None)).expect("fat plan");
        let mut thin_report = explain_application(&thin, PlanLimits::default(), |kappa| {
            if kappa == capabilities_key {
                Ok(Some(capabilities.to_vec()))
            } else if kappa == layer_key {
                Ok(Some(layer.to_vec()))
            } else {
                Ok(None)
            }
        })
        .expect("thin plan");
        fat_report.evaluate_providers(available);
        thin_report.evaluate_providers(available);

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

        let grant = EffectiveGrant::local_baseline();
        let mut fat_decisions = Vec::new();
        fat_report
            .into_application_plan()
            .expect("fat strict plan")
            .admitted_grants_with(&grant, |decision| fat_decisions.push(decision))
            .expect("fat admission");
        let mut thin_decisions = Vec::new();
        thin_report
            .into_application_plan()
            .expect("thin strict plan")
            .admitted_grants_with(&grant, |decision| thin_decisions.push(decision))
            .expect("thin admission");
        assert_eq!(fat_decisions, thin_decisions);
    }

    #[test]
    fn unsupported_service_provider_is_explainable_but_not_executable() {
        let capabilities = test_capabilities();
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
    fn child_references_resolve_into_strict_admission_edges() {
        let capabilities = test_capabilities();
        let layer = b"parent wasm";
        let child_layer = b"child wasm";
        let capabilities_kappa = address_bytes(capabilities);
        let layer_kappa = address_bytes(layer);
        let child_layer_kappa = address_bytes(child_layer);
        let child_manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![Layer::wasm(child_layer_kappa, "child_run")],
            children: Vec::new(),
        };
        let child_manifest_bytes = child_manifest.canonicalize();
        let child_kappa = address_bytes(&child_manifest_bytes);
        let manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![Layer::wasm(layer_kappa, "run")],
            children: vec![(child_kappa, capabilities_kappa)],
        };
        let bytes = write_archive(
            &manifest,
            &[
                (&capabilities_kappa, capabilities),
                (&layer_kappa, layer),
                (&child_kappa, &child_manifest_bytes),
                (&child_layer_kappa, child_layer),
            ],
        );

        let mut report =
            explain_application(&bytes, PlanLimits::default(), |_| Ok(None)).expect("explain");
        report.evaluate_providers(available);
        assert_eq!(report.children.len(), 1);
        assert_eq!(report.application_count, 2);
        assert_eq!(report.max_depth, 1);
        assert_eq!(report.referenced_object_count, 4);
        assert_eq!(
            report.children[0].parent_application_kappa,
            report.identity.application_kappa
        );
        let capabilities_kappa_string = capabilities_kappa.to_string();
        assert_eq!(
            report.children[0].requires_kappa.as_deref(),
            Some(capabilities_kappa_string.as_str())
        );
        assert_eq!(report.children[0].layer_count, Some(1));
        assert!(report.blockers.is_empty());
        assert!(report.runnable());
        let plan = report.into_application_plan().expect("strict child plan");
        assert_eq!(plan.children.len(), 1);
        let mut decisions = Vec::new();
        plan.admitted_grants_with(&EffectiveGrant::local_baseline(), |decision| {
            decisions.push(decision);
        })
        .expect("empty child attenuation");
        assert_eq!(
            decisions
                .iter()
                .map(|decision| (decision.relation.as_str(), decision.outcome.as_str()))
                .collect::<Vec<_>>(),
            [
                ("application_request", "allowed"),
                ("child_delegation", "allowed"),
                ("application_request", "allowed"),
            ]
        );
        assert_eq!(decisions[0].grant_source, "local_baseline");
        assert_eq!(decisions[2].grant_source, "child_delegation");
    }

    #[test]
    fn recursively_resolves_nested_child_manifests_capabilities_and_layers() {
        let capabilities = test_capabilities();
        let capabilities_kappa = address_bytes(capabilities);
        let root_layer = b"root wasm";
        let child_layer = b"child wasm";
        let grandchild_layer = b"grandchild wasm";
        let root_layer_kappa = address_bytes(root_layer);
        let child_layer_kappa = address_bytes(child_layer);
        let grandchild_layer_kappa = address_bytes(grandchild_layer);
        let grandchild_manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![Layer::wasm(grandchild_layer_kappa, "grandchild_run")],
            children: Vec::new(),
        };
        let grandchild_bytes = grandchild_manifest.canonicalize();
        let grandchild_kappa = address_bytes(&grandchild_bytes);
        let child_manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![Layer::wasm(child_layer_kappa, "child_run")],
            children: vec![(grandchild_kappa, capabilities_kappa)],
        };
        let child_bytes = child_manifest.canonicalize();
        let child_kappa = address_bytes(&child_bytes);
        let root_manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![Layer::wasm(root_layer_kappa, "root_run")],
            children: vec![(child_kappa, capabilities_kappa)],
        };
        let archive = write_archive(
            &root_manifest,
            &[
                (&capabilities_kappa, capabilities),
                (&root_layer_kappa, root_layer),
                (&child_kappa, &child_bytes),
                (&child_layer_kappa, child_layer),
                (&grandchild_kappa, &grandchild_bytes),
                (&grandchild_layer_kappa, grandchild_layer),
            ],
        );

        let mut report =
            explain_application(&archive, PlanLimits::default(), |_| Ok(None)).expect("closure");
        assert_eq!(report.application_count, 3);
        assert_eq!(report.max_depth, 2);
        assert_eq!(report.children.len(), 2);
        assert_eq!(
            report.children[1].parent_application_kappa,
            child_kappa.to_string()
        );
        assert_eq!(
            report.children[1].application_kappa,
            grandchild_kappa.to_string()
        );
        assert_eq!(report.referenced_object_count, 6);
        assert_eq!(report.objects.len(), 6);
        assert!(report.blockers.is_empty());
        assert!(!report.runnable());
        report.evaluate_providers(available);
        assert!(report.runnable());
        let plan = report.into_application_plan().expect("nested strict plan");
        assert_eq!(plan.children.len(), 2);
        plan.authorize_capability_tree(&EffectiveGrant::local_baseline())
            .expect("nested empty attenuation");
    }

    #[test]
    fn child_depth_and_application_limits_cover_the_complete_tree() {
        let capabilities = test_capabilities();
        let capabilities_kappa = address_bytes(capabilities);
        let layer = b"shared wasm";
        let layer_kappa = address_bytes(layer);
        let leaf_manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![Layer::wasm(layer_kappa, "leaf")],
            children: Vec::new(),
        };
        let leaf_bytes = leaf_manifest.canonicalize();
        let leaf_kappa = address_bytes(&leaf_bytes);
        let child_manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![Layer::wasm(layer_kappa, "child")],
            children: vec![(leaf_kappa, capabilities_kappa)],
        };
        let child_bytes = child_manifest.canonicalize();
        let child_kappa = address_bytes(&child_bytes);
        let root_manifest = AppManifest {
            primary: Some(0),
            requires: capabilities_kappa,
            layers: vec![Layer::wasm(layer_kappa, "root")],
            children: vec![(child_kappa, capabilities_kappa)],
        };
        let archive = write_archive(
            &root_manifest,
            &[
                (&capabilities_kappa, capabilities),
                (&layer_kappa, layer),
                (&child_kappa, &child_bytes),
                (&leaf_kappa, &leaf_bytes),
            ],
        );

        let depth_limited = explain_application(
            &archive,
            PlanLimits {
                max_depth: 1,
                ..PlanLimits::default()
            },
            |_| Ok(None),
        )
        .expect("depth report");
        assert!(depth_limited.blockers.iter().any(|blocker| matches!(
            blocker,
            PlanBlocker::LimitExceeded {
                limit: "application_depth",
                maximum: 1,
                actual: 2,
                ..
            }
        )));

        let application_limited = explain_application(
            &archive,
            PlanLimits {
                max_applications: 2,
                ..PlanLimits::default()
            },
            |_| Ok(None),
        )
        .expect("application report");
        assert!(application_limited.blockers.iter().any(|blocker| matches!(
            blocker,
            PlanBlocker::LimitExceeded {
                limit: "applications",
                maximum: 2,
                actual: 3,
                ..
            }
        )));
    }

    #[test]
    fn cycle_paths_include_the_repeated_application() {
        let root = "blake3:root".to_owned();
        let child = "blake3:child".to_owned();
        assert_eq!(
            child_cycle_path(&[root.clone(), child.clone()], &root),
            Some(vec![root.clone(), child, root])
        );
        assert_eq!(
            child_cycle_path(&["blake3:root".to_owned()], "blake3:new"),
            None
        );
    }

    #[test]
    fn root_plan_limits_block_before_strict_execution() {
        let capabilities = test_capabilities();
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
