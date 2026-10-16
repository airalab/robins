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
//! Scheme-tagged identity: the signer and its public key.

use super::scheme::Scheme;
use crate::{error::Error, types::AccountId};
use robonomics_runtime_subxt_api::{MultiSignature, RobonomicsConfig};
use sp_core::{ByteArray, Pair};
use std::{fmt, marker::PhantomData};

/// Public key of the scheme `S`.
///
/// The scheme tag makes it impossible to hand a key of the wrong scheme to a
/// [`Signer`] or [`Cipher`](super::Cipher): `Signer<Sr25519>` only accepts
/// `PublicKey<Sr25519>`.
pub struct PublicKey<S: Scheme> {
    bytes: [u8; 32],
    scheme: PhantomData<fn() -> S>,
}

impl<S: Scheme> PublicKey<S> {
    /// Raw 32-byte public key.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.bytes
    }

    /// Account identifier of the public key.
    pub fn account_id(&self) -> AccountId {
        AccountId::from(self.bytes)
    }
}

impl<S: Scheme> From<[u8; 32]> for PublicKey<S> {
    /// Tag raw bytes as a public key of the scheme `S`.
    fn from(bytes: [u8; 32]) -> Self {
        Self {
            bytes,
            scheme: PhantomData,
        }
    }
}

impl<S: Scheme> From<AccountId> for PublicKey<S> {
    fn from(account: AccountId) -> Self {
        Self::from(account.0)
    }
}

impl<S: Scheme> AsRef<[u8]> for PublicKey<S> {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl<S: Scheme> Clone for PublicKey<S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<S: Scheme> Copy for PublicKey<S> {}

impl<S: Scheme> PartialEq for PublicKey<S> {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes
    }
}

impl<S: Scheme> Eq for PublicKey<S> {}

impl<S: Scheme> std::hash::Hash for PublicKey<S> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.bytes.hash(state);
    }
}

impl<S: Scheme> fmt::Debug for PublicKey<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey<{}>({})", S::NAME, self.account_id())
    }
}

/// A CPS account identity of the scheme `S` ([`Sr25519`](super::Sr25519) or
/// [`Ed25519`](super::Ed25519)).
///
/// One `Signer` serves both purposes, kept separate:
/// - **signing**: it implements the Subxt signer interface, so
///   [`Client::submit`](crate::Client::submit) and
///   [`Client::submit_finalized`](crate::Client::submit_finalized) take it
///   directly while Subxt stays out of the public API. The account is the
///   public key. Transactions are signed only with the asymmetric signing key,
///   never with ECDH, HKDF or AEAD output.
/// - **encryption**: it implements [`SharedSecret`](super::SharedSecret) and
///   therefore [`Cipher`](super::Cipher), taking the receiver's
///   [`PublicKey<S>`] of the same scheme.
///
/// ```
/// use libcps::crypto::{Cipher, EncryptionAlgorithm, Signer, Sr25519};
///
/// let alice = Signer::<Sr25519>::from_suri("//Alice").unwrap();
/// let bob = Signer::<Sr25519>::from_suri("//Bob").unwrap();
///
/// let message = alice
///     .encrypt(b"secret", &bob.public_key(), EncryptionAlgorithm::XChaCha20Poly1305)
///     .unwrap();
/// assert_eq!(bob.decrypt(&message, Some(&alice.public_key())).unwrap(), b"secret");
/// ```
///
/// Mixing schemes does not compile:
///
/// ```compile_fail
/// use libcps::crypto::{Cipher, Ed25519, EncryptionAlgorithm, Signer, Sr25519};
///
/// let sr = Signer::<Sr25519>::from_suri("//Alice").unwrap();
/// let ed = Signer::<Ed25519>::from_suri("//Bob").unwrap();
/// let _ = sr.encrypt(b"x", &ed.public_key(), EncryptionAlgorithm::XChaCha20Poly1305);
/// ```
pub struct Signer<S: Scheme> {
    pair: S::Pair,
}

impl<S: Scheme> Signer<S> {
    /// Signer from a secret URI such as `//Alice` or a mnemonic phrase with
    /// optional derivation path.
    pub fn from_suri(suri: &str) -> Result<Self, Error> {
        S::Pair::from_string(suri, None)
            .map(|pair| Self { pair })
            .map_err(|e| Error::Crypto(format!("failed to parse {} keypair: {e:?}", S::NAME)))
    }

    /// Signer from a 32-byte secret seed (the secret key material, not the public key).
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            pair: S::Pair::from_seed_slice(&seed)
                .expect("every supported scheme has a 32-byte seed"),
        }
    }

    /// Public key of the signer.
    pub fn public_key(&self) -> PublicKey<S> {
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(self.pair.public().as_slice());
        PublicKey::from(bytes)
    }

    /// Account identifier of the signer (its public key).
    pub fn account_id(&self) -> AccountId {
        self.public_key().account_id()
    }

    /// First 32 bytes of the raw secret, as used for key agreement.
    pub(crate) fn secret_bytes(&self) -> [u8; 32] {
        let raw = self.pair.to_raw_vec();
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&raw[..32]);
        secret
    }
}

impl<S: Scheme> Clone for Signer<S> {
    fn clone(&self) -> Self {
        Self {
            pair: self.pair.clone(),
        }
    }
}

impl<S: Scheme> subxt::transactions::Signer<RobonomicsConfig> for Signer<S> {
    fn account_id(&self) -> AccountId {
        Signer::account_id(self)
    }

    fn sign(&self, signer_payload: &[u8]) -> MultiSignature {
        S::sign_payload(&self.pair, signer_payload)
    }
}

impl<S: Scheme> fmt::Debug for Signer<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signer")
            .field("scheme", &S::NAME)
            .field("account_id", &self.account_id())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{Ed25519, Sr25519};
    use sp_core::{ed25519, sr25519};
    use subxt::transactions::Signer as _;

    #[test]
    fn constructors_agree_on_suri_and_seed() {
        let alice = Signer::<Sr25519>::from_suri("//Alice").unwrap();
        assert_eq!(
            alice.public_key(),
            Signer::<Sr25519>::from_suri("//Alice")
                .unwrap()
                .public_key()
        );
        assert_ne!(
            alice.public_key(),
            Signer::<Sr25519>::from_suri("//Bob").unwrap().public_key()
        );
        assert_eq!(
            Signer::<Ed25519>::from_seed([2u8; 32]).public_key(),
            Signer::<Ed25519>::from_seed([2u8; 32]).public_key()
        );
        let seeded = Signer::<Sr25519>::from_seed([1u8; 32]);
        assert_eq!(seeded.account_id().0, seeded.public_key().to_bytes());
    }

    #[test]
    fn sr25519_signer_signs_with_its_key() {
        let signer = Signer::<Sr25519>::from_suri("//Alice").unwrap();

        let MultiSignature::Sr25519(raw) = signer.sign(b"payload") else {
            panic!("expected an SR25519 signature");
        };
        assert!(sr25519::Pair::verify(
            &sr25519::Signature::from_raw(raw),
            b"payload",
            &sr25519::Public::from_raw(signer.public_key().to_bytes())
        ));
    }

    #[test]
    fn ed25519_signer_signs_with_its_key() {
        let signer = Signer::<Ed25519>::from_suri("//Alice").unwrap();

        let MultiSignature::Ed25519(raw) = signer.sign(b"payload") else {
            panic!("expected an ED25519 signature");
        };
        assert!(ed25519::Pair::verify(
            &ed25519::Signature::from_raw(raw),
            b"payload",
            &ed25519::Public::from_raw(signer.public_key().to_bytes())
        ));
    }

    #[test]
    fn schemes_produce_distinct_accounts() {
        assert_ne!(
            Signer::<Sr25519>::from_suri("//Alice")
                .unwrap()
                .account_id(),
            Signer::<Ed25519>::from_suri("//Alice")
                .unwrap()
                .account_id()
        );
    }

    #[test]
    fn debug_hides_secret() {
        let signer = Signer::<Sr25519>::from_suri("//Alice").unwrap();
        assert!(!format!("{signer:?}").contains("secret"));
    }

    #[test]
    fn invalid_suri_is_rejected() {
        assert!(Signer::<Sr25519>::from_suri("not a valid suri").is_err());
        assert!(Signer::<Ed25519>::from_suri("not a valid suri").is_err());
    }

    #[test]
    fn public_key_round_trips_bytes() {
        let key = Signer::<Ed25519>::from_suri("//Alice")
            .unwrap()
            .public_key();
        assert_eq!(PublicKey::<Ed25519>::from(key.to_bytes()), key);
    }
}
