///////////////////////////////////////////////////////////////////////////////
//
//  Copyright 2018-2026 Robonomics Network <research@robonomics.network>
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
//
///////////////////////////////////////////////////////////////////////////////
//! [`CpsTarget`] backed by a live `libcps` connection.

use super::{AnchorError, Confirmation, CpsTarget};
use crate::chain::Chain;
use async_trait::async_trait;
use libcps::crypto::{Signer, Sr25519};
use libcps::{Capability, Error as CpsError, NodeId};
use std::sync::Arc;

/// A CPS node reachable through `libcps`, with the account that signs for it.
pub struct LibcpsTarget {
    /// Shared chain connection (and the embedded light node behind it, if any).
    chain: Arc<Chain>,
    signer: Signer<Sr25519>,
    node: NodeId,
}

impl LibcpsTarget {
    /// Verify over `chain` that the account can write to `node`.
    ///
    /// # Errors
    ///
    /// Connection failures are transient; an invalid key, a missing node or a
    /// missing write capability are permanent.
    pub async fn connect(chain: Arc<Chain>, suri: &str, node: NodeId) -> Result<Self, AnchorError> {
        let signer = Signer::<Sr25519>::from_suri(suri)
            .map_err(|e| AnchorError::Permanent(format!("invalid signing key: {e}")))?;
        let account = signer.account_id();
        let can_write = chain
            .cps()
            .node(node)
            .has_capability(&account, Capability::Write)
            .await
            .map_err(classify)?;
        if !can_write {
            return Err(AnchorError::Permanent(format!(
                "account {account} has no write access to CPS node {node}"
            )));
        }
        Ok(Self {
            chain,
            signer,
            node,
        })
    }
}

#[async_trait]
impl CpsTarget for LibcpsTarget {
    async fn set_payload(&self, payload: &[u8]) -> Result<Confirmation, AnchorError> {
        let tx = self
            .chain
            .cps()
            .node(self.node)
            .set_payload(payload)
            .map_err(classify)?;
        let receipt = self
            .chain
            .cps()
            .submit_finalized(&tx, &self.signer)
            .await
            .map_err(classify)?;
        Ok(Confirmation {
            extrinsic_hash: receipt.extrinsic_hash.to_string(),
            block_hash: receipt.block_hash.to_string(),
        })
    }
}

/// Split `libcps` failures into retryable and permanent ones.
///
/// A finalized dispatch failure ([`CpsError::Dispatch`]) is permanent: the
/// runtime would reject the same call again.
fn classify(error: CpsError) -> AnchorError {
    match error {
        CpsError::Connection(_)
        | CpsError::Rpc(_)
        | CpsError::Runtime(_)
        | CpsError::Transaction(_) => AnchorError::Transient(error.to_string()),
        other => AnchorError::Permanent(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_failures_are_permanent_and_submission_failures_transient() {
        assert!(!classify(CpsError::AccessDenied).is_transient());
        assert!(classify(CpsError::Transaction("connection dropped".into())).is_transient());
    }
}
