use kameo::actor::{ActorRef, Spawn};
use kameo::Actor;

/// Root of the daemon's local supervision tree.
///
/// Network transparency deliberately does not live in the actor system. Remote
/// communication crosses the authenticated gRPC boundary instead.
#[derive(Actor)]
pub struct RootSupervisor;

#[derive(Clone)]
pub struct ActorSystem {
    root: ActorRef<RootSupervisor>,
}

impl ActorSystem {
    pub fn start() -> Self {
        Self {
            root: RootSupervisor::spawn(RootSupervisor),
        }
    }

    pub fn root(&self) -> &ActorRef<RootSupervisor> {
        &self.root
    }
}
