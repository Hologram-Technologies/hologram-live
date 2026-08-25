use crate::actor::RootSupervisor;
use crate::error::{LiveError, Result};
use crate::holo_capability::CapabilityDecision;
use crate::util::now_millis;
use kameo::actor::{ActorRef, Spawn};
use kameo::mailbox;
use kameo::message::{Context, Message};
use kameo::Actor;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncWriteExt, BufWriter};

#[derive(Debug, Clone, Serialize)]
pub struct AuditEvent {
    pub timestamp_millis: u64,
    pub principal: String,
    pub operation: String,
    pub resource: Option<String>,
    pub outcome: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capability_decision: Option<CapabilityDecision>,
}

impl AuditEvent {
    pub fn new(
        principal: impl Into<String>,
        operation: impl Into<String>,
        resource: Option<String>,
        outcome: impl Into<String>,
    ) -> Self {
        Self {
            timestamp_millis: now_millis(),
            principal: principal.into(),
            operation: operation.into(),
            resource,
            outcome: outcome.into(),
            capability_decision: None,
        }
    }

    pub fn capability_decision(principal: impl Into<String>, decision: CapabilityDecision) -> Self {
        Self {
            timestamp_millis: now_millis(),
            principal: principal.into(),
            operation: "holo.capability.authorize".to_owned(),
            resource: Some(decision.application_kappa.clone()),
            outcome: decision.outcome.clone(),
            capability_decision: Some(decision),
        }
    }
}

struct Record(AuditEvent);
struct Flush;

#[derive(Actor)]
struct AuditActor {
    writer: BufWriter<File>,
}

impl Message<Record> for AuditActor {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        message: Record,
        _context: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        let mut encoded = serde_json::to_vec(&message.0)?;
        encoded.push(b'\n');
        self.writer.write_all(&encoded).await?;
        Ok(())
    }
}

impl Message<Flush> for AuditActor {
    type Reply = Result<()>;

    async fn handle(
        &mut self,
        _message: Flush,
        _context: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.writer.flush().await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct AuditLog {
    actor: ActorRef<AuditActor>,
    path: PathBuf,
}

impl AuditLog {
    pub async fn open(
        path: impl Into<PathBuf>,
        mailbox_capacity: usize,
        supervisor: &ActorRef<RootSupervisor>,
    ) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| LiveError::io(parent, error))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await
            .map_err(|error| LiveError::io(&path, error))?;
        let actor = AuditActor::spawn_link_with_mailbox(
            supervisor,
            AuditActor {
                writer: BufWriter::new(file),
            },
            mailbox::bounded(mailbox_capacity),
        )
        .await;
        Ok(Self { actor, path })
    }

    pub async fn record(&self, event: AuditEvent) -> Result<()> {
        self.actor
            .ask(Record(event))
            .await
            .map_err(|error| LiveError::Conflict(format!("write audit event: {error}")))?;
        Ok(())
    }

    pub async fn flush(&self) -> Result<()> {
        self.actor
            .ask(Flush)
            .await
            .map_err(|error| LiveError::Conflict(format!("flush audit log: {error}")))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::ActorSystem;

    #[tokio::test]
    async fn capability_records_are_flushed_and_contain_only_non_secret_evidence() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("audit.jsonl");
        let actors = ActorSystem::start();
        let audit = AuditLog::open(&path, 4, actors.root())
            .await
            .expect("audit log");
        let decision = CapabilityDecision {
            application_kappa: "blake3:application".to_owned(),
            parent_application_kappa: None,
            requested_capabilities_kappa: "blake3:request".to_owned(),
            effective_grant_kappa: "blake3:grant".to_owned(),
            grant_source: "local_baseline".to_owned(),
            relation: "application_request".to_owned(),
            outcome: "denied".to_owned(),
        };
        audit
            .record(AuditEvent::capability_decision("local-cli", decision))
            .await
            .expect("record");
        audit.flush().await.expect("flush");

        let encoded = std::fs::read_to_string(path).expect("read audit");
        let value: serde_json::Value = serde_json::from_str(encoded.trim()).expect("JSONL row");
        assert_eq!(value["principal"], "local-cli");
        assert_eq!(value["operation"], "holo.capability.authorize");
        assert_eq!(value["outcome"], "denied");
        assert_eq!(
            value["capability_decision"]["requested_capabilities_kappa"],
            "blake3:request"
        );
        for forbidden in ["token", "payload", "source_document", "storage_roots"] {
            assert!(!encoded.contains(forbidden), "audit leaked {forbidden}");
        }
    }
}
