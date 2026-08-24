use crate::actor::RootSupervisor;
use crate::error::{LiveError, Result};
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
            .tell(Record(event))
            .await
            .map_err(|error| LiveError::Conflict(format!("send audit event: {error}")))
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
