use crate::error::{LiveError, Result};
use crate::protocol::OperationKind;

#[derive(Debug, Clone)]
pub struct Principal {
    pub id: String,
    pub scope: String,
}

pub trait Authorizer: Send + Sync {
    fn authorize(&self, principal: &Principal, operation: &str, kind: OperationKind) -> Result<()>;
}

pub struct LocalAuthorizer;

impl Authorizer for LocalAuthorizer {
    fn authorize(
        &self,
        _principal: &Principal,
        _operation: &str,
        _kind: OperationKind,
    ) -> Result<()> {
        Ok(())
    }
}

pub struct DenyMutationAuthorizer;

impl Authorizer for DenyMutationAuthorizer {
    fn authorize(
        &self,
        _principal: &Principal,
        operation: &str,
        kind: OperationKind,
    ) -> Result<()> {
        if kind == OperationKind::Read {
            Ok(())
        } else {
            Err(LiveError::Authorization(format!(
                "operation {operation} is not authorized"
            )))
        }
    }
}
