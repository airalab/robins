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
//! ECDH key agreement for the CPS [`Signer`].

use super::scheme::Scheme;
use super::signer::{PublicKey, Signer};
use anyhow::Result;

/// ECDH key agreement of a keypair, scheme-specific:
///
/// - SR25519: Ristretto255 scalar multiplication
/// - ED25519: X25519 key agreement (Edwards to Montgomery conversion)
///
/// Both parties derive the same secret from their own keypair and the other's
/// public key. It is the key-agreement input of [`Cipher`](super::Cipher),
/// which is implemented for every `SharedSecret`. Implemented by
/// [`Signer<S>`](Signer), whose [`Public`](SharedSecret::Public) is
/// [`PublicKey<S>`]: a key of another scheme cannot be passed.
pub trait SharedSecret {
    /// Public key type accepted from the other party.
    type Public: Clone + AsRef<[u8]> + From<[u8; 32]>;

    /// Public key of this keypair.
    fn public_key(&self) -> Self::Public;

    /// Derive the 32-byte shared secret with `receiver_public`.
    ///
    /// # Errors
    ///
    /// Returns an error if the public key cannot be decompressed into a valid
    /// curve point. Not all 32-byte arrays represent valid curve points.
    fn derive_shared(&self, receiver_public: &Self::Public) -> Result<[u8; 32]>;
}

impl<S: Scheme> SharedSecret for Signer<S> {
    type Public = PublicKey<S>;

    fn public_key(&self) -> PublicKey<S> {
        Signer::public_key(self)
    }

    fn derive_shared(&self, receiver_public: &PublicKey<S>) -> Result<[u8; 32]> {
        S::derive_shared(self.secret_bytes(), &receiver_public.to_bytes())
    }
}
