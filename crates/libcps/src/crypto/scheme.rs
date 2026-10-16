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
//! Cryptographic schemes as zero-sized type tags.
//!
//! [`Signer`](super::Signer) and [`PublicKey`](super::PublicKey) are tagged with
//! the scheme they belong to, so a key of one scheme cannot be used with a
//! signer of the other: the mismatch is a compile error.

use anyhow::{anyhow, Result};
use robonomics_runtime_subxt_api::MultiSignature;
use sha2::{Digest, Sha512};
use sp_core::{ed25519, sr25519, Pair};

mod sealed {
    pub trait Sealed {}
}

/// A supported cryptographic scheme: [`Sr25519`] or [`Ed25519`].
///
/// Sealed: the set of schemes is fixed. The methods are implementation details
/// of [`Signer`](super::Signer) and [`Cipher`](super::Cipher).
pub trait Scheme: sealed::Sealed + Send + Sync + 'static {
    /// The underlying `sp_core` pair.
    #[doc(hidden)]
    type Pair: Pair + Clone + Send + Sync;

    /// Human-readable scheme name.
    const NAME: &'static str;

    /// Sign a transaction payload with the asymmetric signing key.
    #[doc(hidden)]
    fn sign_payload(pair: &Self::Pair, message: &[u8]) -> MultiSignature;

    /// Scheme-specific ECDH from the 32-byte secret and the other party's
    /// 32-byte public key.
    #[doc(hidden)]
    fn derive_shared(secret: [u8; 32], receiver_public: &[u8; 32]) -> Result<[u8; 32]>;
}

/// SR25519 (Schnorrkel over Ristretto255): the Substrate-native scheme and the
/// default CPS account scheme. Key agreement is Ristretto255 scalar
/// multiplication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Sr25519;

/// ED25519: used by IoT devices and Home Assistant. Key agreement is X25519
/// (Edwards to Montgomery conversion).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ed25519;

impl sealed::Sealed for Sr25519 {}
impl sealed::Sealed for Ed25519 {}

impl Scheme for Sr25519 {
    type Pair = sr25519::Pair;

    const NAME: &'static str = "SR25519";

    fn sign_payload(pair: &Self::Pair, message: &[u8]) -> MultiSignature {
        MultiSignature::Sr25519(pair.sign(message).0)
    }

    fn derive_shared(secret: [u8; 32], receiver_public: &[u8; 32]) -> Result<[u8; 32]> {
        use curve25519_dalek::ristretto::CompressedRistretto;
        use curve25519_dalek::scalar::Scalar;

        let scalar = Scalar::from_bytes_mod_order(secret);

        let public_point = CompressedRistretto(*receiver_public)
            .decompress()
            .ok_or_else(|| anyhow!("Failed to decompress Ristretto255 public key"))?;

        let shared_compressed = (scalar * public_point).compress();

        // Hash for uniform distribution
        let mut hasher = Sha512::new();
        hasher.update(b"robonomics-cps-ecdh");
        hasher.update(shared_compressed.as_bytes());
        let hash_output = hasher.finalize();

        let mut result = [0u8; 32];
        result.copy_from_slice(&hash_output[..32]);
        Ok(result)
    }
}

impl Scheme for Ed25519 {
    type Pair = ed25519::Pair;

    const NAME: &'static str = "ED25519";

    fn sign_payload(pair: &Self::Pair, message: &[u8]) -> MultiSignature {
        MultiSignature::Ed25519(pair.sign(message).0)
    }

    fn derive_shared(secret: [u8; 32], receiver_public: &[u8; 32]) -> Result<[u8; 32]> {
        use curve25519_dalek::edwards::CompressedEdwardsY;

        // Hash and clamp secret for X25519
        let mut hasher = Sha512::new();
        hasher.update(secret);
        let hash = hasher.finalize();

        let mut scalar_bytes = [0u8; 32];
        scalar_bytes.copy_from_slice(&hash[..32]);
        scalar_bytes[0] &= 248;
        scalar_bytes[31] &= 127;
        scalar_bytes[31] |= 64;

        let my_x25519_secret = x25519_dalek::StaticSecret::from(scalar_bytes);

        // Convert Ed25519 public key to X25519
        let edwards_point = CompressedEdwardsY(*receiver_public)
            .decompress()
            .ok_or_else(|| anyhow!("Failed to decompress ED25519 public key"))?;
        let their_x25519_public =
            x25519_dalek::PublicKey::from(edwards_point.to_montgomery().to_bytes());

        Ok(*my_x25519_secret
            .diffie_hellman(&their_x25519_public)
            .as_bytes())
    }
}
