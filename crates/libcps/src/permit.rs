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
//! Identity permits: the signing payload and a portable signed artifact.
//!
//! An identity account consents to be used as the identity of nodes in a scope
//! by signing [`IdentityPermitPayload::signing_bytes`]. Building the payload is
//! separate from signing, so any signer (software key, hardware wallet, remote
//! service) can produce the signature over the same deterministic bytes.

use crate::{
    error::{Error, Result},
    identity::{IdentityKey, IdentityScheme},
    types::{AccountId, Hash, IdentityAssignment, IdentityPermit, PermitSignature, ScopeId},
};
use parity_scale_codec::Encode;
use polkadot_sdk::sp_core::{ecdsa, ed25519, sr25519, Pair};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// Domain separator, the first element of the signed tuple (mirrors the runtime
/// constant `IDENTITY_PERMIT_DOMAIN`).
pub const IDENTITY_PERMIT_DOMAIN: &[u8] = b"robonomics:cps:node-identity:v1";

/// Version of the [`PermitArtifact`] JSON format.
pub const PERMIT_ARTIFACT_VERSION: u8 = 1;

/// Everything an identity account signs to issue a permit.
///
/// The permit is bound to one chain (`genesis_hash`) and one scope generation
/// (`scope`), not to a particular node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityPermitPayload {
    /// Genesis hash of the chain the permit is valid on.
    pub genesis_hash: Hash,
    /// Scope the identity consents to serve in.
    pub scope: ScopeId,
    /// Last block number at which the permit may be used.
    pub valid_until: u32,
}

impl IdentityPermitPayload {
    /// The exact bytes to sign: the SCALE encoding of
    /// `(IDENTITY_PERMIT_DOMAIN, genesis_hash, scope, valid_until)`, with the
    /// domain encoded as `Vec<u8>` (compact length prefix).
    pub fn signing_bytes(&self) -> Vec<u8> {
        (
            IDENTITY_PERMIT_DOMAIN,
            self.genesis_hash.0,
            self.scope.0,
            self.valid_until,
        )
            .encode()
    }

    /// Runtime permit carrying `signature`.
    pub fn into_permit(self, signature: PermitSignature) -> IdentityPermit {
        IdentityPermit {
            valid_until: self.valid_until,
            signature,
        }
    }

    /// Sign the payload with `key`, producing a self-describing artifact.
    pub fn sign(self, key: &IdentityKey) -> PermitArtifact {
        let signature = key.sign(&self.signing_bytes());
        PermitArtifact::new(self, key.account_id(), signature)
    }

    /// Artifact for `identity` from a signature made elsewhere.
    pub fn with_signature(self, identity: AccountId, signature: PermitSignature) -> PermitArtifact {
        PermitArtifact::new(self, identity, signature)
    }
}

/// Portable, self-describing identity permit.
///
/// JSON form (see [`PermitArtifact::to_json`]):
///
/// ```json
/// {
///   "version": 1,
///   "identity": "5FHneW...",
///   "scheme": "sr25519",
///   "genesis_hash": "0x1234...",
///   "scope": 7,
///   "valid_until": 12345678,
///   "signature": "0xabcd..."
/// }
/// ```
///
/// The runtime needs only `identity`, `valid_until` and `signature`; the other
/// fields make the artifact verifiable and safe to carry between commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PermitArtifact {
    /// What was signed.
    pub payload: IdentityPermitPayload,
    /// Identity account that signed.
    pub identity: AccountId,
    /// Signature over [`IdentityPermitPayload::signing_bytes`].
    pub signature: PermitSignature,
}

#[derive(Serialize, Deserialize)]
struct Repr {
    version: u8,
    identity: String,
    scheme: IdentityScheme,
    genesis_hash: String,
    scope: u64,
    valid_until: u32,
    signature: String,
}

impl PermitArtifact {
    /// Artifact of `payload` signed by `identity`.
    pub fn new(
        payload: IdentityPermitPayload,
        identity: AccountId,
        signature: PermitSignature,
    ) -> Self {
        Self {
            payload,
            identity,
            signature,
        }
    }

    /// Runtime assignment binding [`identity`](Self::identity) with this permit.
    pub fn into_assignment(self) -> IdentityAssignment {
        IdentityAssignment::with_permit(self.identity, self.payload.into_permit(self.signature))
    }

    /// Check offline that the signature is valid for the identity over the payload.
    pub fn verify(&self) -> bool {
        let message = self.payload.signing_bytes();
        match self.signature {
            PermitSignature::Sr25519(s) => sr25519::Pair::verify(
                &sr25519::Signature::from_raw(s),
                &message,
                &sr25519::Public::from_raw(self.identity.0),
            ),
            PermitSignature::Ed25519(s) => ed25519::Pair::verify(
                &ed25519::Signature::from_raw(s),
                &message,
                &ed25519::Public::from_raw(self.identity.0),
            ),
            PermitSignature::Ecdsa(s) => ecdsa::Signature::from_raw(s)
                .recover(&message)
                .is_some_and(|public| {
                    polkadot_sdk::sp_crypto_hashing::blake2_256(public.as_ref()) == self.identity.0
                }),
        }
    }

    /// Pretty-printed JSON form.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&Repr {
            version: PERMIT_ARTIFACT_VERSION,
            identity: self.identity.to_string(),
            scheme: self.signature.scheme(),
            genesis_hash: format!("0x{}", hex::encode(self.payload.genesis_hash.0)),
            scope: self.payload.scope.0,
            valid_until: self.payload.valid_until,
            signature: format!("0x{}", hex::encode(self.signature.to_bytes())),
        })
        .expect("plain data serializes")
    }

    /// Parse the JSON form.
    ///
    /// # Errors
    ///
    /// [`Error::Crypto`] for malformed JSON, an unsupported version or
    /// malformed fields.
    pub fn from_json(json: &str) -> Result<Self> {
        let bad = |what: &str| Error::Crypto(format!("invalid permit: {what}"));
        let repr: Repr = serde_json::from_str(json).map_err(|e| bad(&e.to_string()))?;
        if repr.version != PERMIT_ARTIFACT_VERSION {
            return Err(bad(&format!("unsupported version {}", repr.version)));
        }
        let decode = |text: &str| hex::decode(text.strip_prefix("0x").unwrap_or(text));
        let genesis: [u8; 32] = decode(&repr.genesis_hash)
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| bad("genesis_hash must be 32 bytes of hex"))?;
        let signature = decode(&repr.signature).map_err(|_| bad("signature is not hex"))?;
        Ok(Self {
            payload: IdentityPermitPayload {
                genesis_hash: Hash::from(genesis),
                scope: ScopeId(repr.scope),
                valid_until: repr.valid_until,
            },
            identity: AccountId::from_str(&repr.identity)
                .map_err(|_| bad("identity is not an SS58 address"))?,
            signature: PermitSignature::from_bytes(repr.scheme, &signature)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> IdentityPermitPayload {
        IdentityPermitPayload {
            genesis_hash: Hash::from([0xab; 32]),
            scope: ScopeId(7),
            valid_until: 1000,
        }
    }

    #[test]
    fn signing_bytes_are_the_scale_tuple() {
        let mut expected = vec![(IDENTITY_PERMIT_DOMAIN.len() as u8) << 2];
        expected.extend_from_slice(IDENTITY_PERMIT_DOMAIN);
        expected.extend_from_slice(&[0xab; 32]);
        expected.extend_from_slice(&7u64.to_le_bytes());
        expected.extend_from_slice(&1000u32.to_le_bytes());
        assert_eq!(payload().signing_bytes(), expected);
    }

    #[test]
    fn artifacts_verify_and_round_trip_for_every_scheme() {
        for scheme in [
            IdentityScheme::Sr25519,
            IdentityScheme::Ed25519,
            IdentityScheme::Ecdsa,
        ] {
            let key = IdentityKey::from_suri(scheme, "//Device").unwrap();
            let artifact = payload().sign(&key);
            assert!(artifact.verify(), "{scheme}");
            assert_eq!(artifact.identity, key.account_id());

            let parsed = PermitArtifact::from_json(&artifact.to_json()).unwrap();
            assert_eq!(parsed, artifact);
            assert!(parsed.verify());
        }
    }

    #[test]
    fn tampered_artifact_does_not_verify() {
        let key = IdentityKey::from_suri(IdentityScheme::Sr25519, "//Device").unwrap();
        let mut artifact = payload().sign(&key);
        artifact.payload.scope = ScopeId(8);
        assert!(!artifact.verify());
    }

    #[test]
    fn malformed_artifacts_are_rejected() {
        assert!(PermitArtifact::from_json("{}").is_err());
        let key = IdentityKey::from_suri(IdentityScheme::Sr25519, "//Device").unwrap();
        let json = payload()
            .sign(&key)
            .to_json()
            .replace("\"version\": 1", "\"version\": 2");
        assert!(PermitArtifact::from_json(&json).is_err());
    }

    #[test]
    fn assignment_carries_the_permit() {
        let key = IdentityKey::from_suri(IdentityScheme::Ed25519, "//Device").unwrap();
        let artifact = payload().sign(&key);
        let assignment = artifact.into_assignment();
        assert_eq!(assignment.account, key.account_id());
        assert_eq!(assignment.permit.unwrap().valid_until, 1000);
    }
}
