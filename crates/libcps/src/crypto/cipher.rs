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
//! Encryption and decryption for the CPS [`Signer`](super::Signer).
//!
//! This module provides the [`Cipher`] trait, implemented for every
//! [`SharedSecret`] (the [`Signer`](super::Signer)). It handles ECDH key
//! agreement, HKDF key derivation and AEAD encryption/decryption directly from
//! the keypair, so no separate cipher object or copy of the key material is
//! needed.

use super::shared_secret::SharedSecret;
use super::types::{EncryptedMessage, EncryptionAlgorithm};
use aes_gcm::{
    aead::{Aead as AesAead, KeyInit as AesKeyInit},
    Aes256Gcm, Nonce as AesNonce,
};
use anyhow::{anyhow, Result};
use chacha20poly1305::{
    aead::Generate, ChaCha20Poly1305, Nonce as ChachaNonce, XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use sha2::Sha256;
use tracing::{debug, trace};

/// HKDF salt for key derivation.
const HKDF_SALT: &[u8] = b"robonomics-network";

/// Encryption and decryption with a CPS keypair.
///
/// Blanket-implemented for every [`SharedSecret`]; the [`Signer`](super::Signer) is
/// the keypair type. Import the trait to use it. Key agreement is
/// [`SharedSecret::derive_shared`]; the receiver's public key is the signer's
/// [`PublicKey`](super::PublicKey) of the same scheme, so a key of another
/// scheme cannot be passed.
///
/// # Examples
///
/// ```
/// use libcps::crypto::{Cipher, EncryptionAlgorithm, Signer, Sr25519};
///
/// let alice = Signer::<Sr25519>::from_suri("//Alice").unwrap();
/// let bob = Signer::<Sr25519>::from_suri("//Bob").unwrap();
///
/// let encrypted = alice
///     .encrypt(b"secret message", &bob.public_key(), EncryptionAlgorithm::XChaCha20Poly1305)
///     .unwrap();
/// let decrypted = bob.decrypt(&encrypted, Some(&alice.public_key())).unwrap();
/// assert_eq!(decrypted, b"secret message");
/// ```
pub trait Cipher: SharedSecret {
    /// Encrypt data for a specific receiver with inlined AEAD.
    ///
    /// The key is derived from the ECDH shared secret with `receiver_public` via
    /// HKDF-SHA256, and a fresh random nonce is generated for every message. The
    /// message records this keypair's public key as the sender (`from`).
    ///
    /// # Arguments
    ///
    /// * `plaintext` - The data to encrypt
    /// * `receiver_public` - The recipient's public key, of the same scheme as this keypair
    /// * `algorithm` - The encryption algorithm to use
    ///
    /// # Returns
    ///
    /// Returns an [`EncryptedMessage`] that can be serialized by the caller
    ///
    /// # Errors
    ///
    /// Returns error if the receiver's public key is invalid (not a valid curve point)
    /// or if the AEAD encryption fails. An invalid key can happen if:
    /// - The public key bytes don't represent a valid Ristretto255 point (SR25519)
    /// - The public key bytes don't represent a valid Edwards curve point (ED25519)
    /// - The receiver_public parameter contains corrupted or malicious data
    ///
    /// Note: Valid public keys from Substrate accounts will always succeed.
    fn encrypt(
        &self,
        plaintext: &[u8],
        receiver_public: &Self::Public,
        algorithm: EncryptionAlgorithm,
    ) -> Result<EncryptedMessage> {
        debug!("Encrypting {} bytes with {}", plaintext.len(), algorithm);
        trace!("Receiver public key provided; proceeding with ECDH");

        // Step 1: Derive shared secret using direct ECDH
        // This can fail if receiver_public is invalid
        trace!("Deriving shared secret via ECDH");
        let shared_secret = self.derive_shared(receiver_public)?;
        trace!("Shared secret derived successfully");

        // Step 2: Derive encryption key using HKDF with salt
        // HKDF expand can only fail if the output length exceeds the hash function's
        // maximum (255 * hash_len for SHA-256 = 8160 bytes), but we only request 32 bytes.
        // We propagate the error for defensive programming rather than panicking.
        trace!("Deriving encryption key with HKDF");
        let mut encryption_key = [0u8; 32];
        let hkdf = Hkdf::<Sha256>::new(Some(HKDF_SALT), &shared_secret);
        hkdf.expand(algorithm.info_string().as_bytes(), &mut encryption_key)
            .map_err(|e| anyhow!("HKDF expansion failed: {e}"))?;
        trace!("Encryption key derived");

        // Step 3: Encrypt with specified algorithm
        trace!("Encrypting plaintext with {:?}", algorithm);
        let (nonce_bytes, ciphertext) = match algorithm {
            EncryptionAlgorithm::XChaCha20Poly1305 => {
                let cipher = XChaCha20Poly1305::new(&encryption_key.into());
                let nonce = XNonce::generate();
                trace!("Generated XChaCha20 nonce: {} bytes", nonce.len());
                let ct = cipher
                    .encrypt(&nonce, plaintext)
                    .map_err(|e| anyhow!("XChaCha20 encryption failed: {e}"))?;
                (nonce.to_vec(), ct)
            }
            EncryptionAlgorithm::AesGcm256 => {
                let cipher = Aes256Gcm::new(&encryption_key.into());
                let nonce = AesNonce::generate();
                trace!("Generated AES-GCM nonce: {} bytes", nonce.len());
                let ct = cipher
                    .encrypt(&nonce, plaintext)
                    .map_err(|e| anyhow!("AES-GCM encryption failed: {e}"))?;
                (nonce.to_vec(), ct)
            }
            EncryptionAlgorithm::ChaCha20Poly1305 => {
                let cipher = ChaCha20Poly1305::new(&encryption_key.into());
                let nonce = ChachaNonce::generate();
                trace!("Generated ChaCha20 nonce: {} bytes", nonce.len());
                let ct = cipher
                    .encrypt(&nonce, plaintext)
                    .map_err(|e| anyhow!("ChaCha20 encryption failed: {e}"))?;
                (nonce.to_vec(), ct)
            }
        };

        // Step 4: Get sender's public key
        let sender_public: [u8; 32] = self
            .public_key()
            .as_ref()
            .try_into()
            .expect("public keys are 32 bytes");
        trace!("Sender public key: {:02x?}...", &sender_public[..8]);

        // Step 5: Create and return message structure with binary data
        debug!(
            "Encryption complete: {} bytes plaintext -> {} bytes ciphertext (+ {} bytes overhead)",
            plaintext.len(),
            ciphertext.len(),
            ciphertext.len() - plaintext.len()
        );
        Ok(EncryptedMessage::V1 {
            algorithm,
            from: sender_public,
            nonce: nonce_bytes,
            ciphertext,
        })
    }

    /// Decrypt data with inlined AEAD (algorithm auto-detected).
    ///
    /// The shared secret is derived with the sender key `from` recorded in the
    /// message, interpreted in this keypair's scheme.
    ///
    /// # Arguments
    ///
    /// * `message` - Encrypted message structure
    /// * `expected_sender` - Optional sender public key; if given, the message is
    ///   rejected unless its `from` equals it
    ///
    /// # Returns
    ///
    /// Returns decrypted plaintext bytes
    ///
    /// # Errors
    ///
    /// Returns error if the sender does not match `expected_sender`, the sender key
    /// is not a valid curve point, the nonce has the wrong length for the algorithm,
    /// or the AEAD decryption fails (wrong key or tampered data).
    fn decrypt(
        &self,
        message: &EncryptedMessage,
        expected_sender: Option<&Self::Public>,
    ) -> Result<Vec<u8>> {
        match message {
            EncryptedMessage::V1 {
                algorithm,
                from,
                nonce,
                ciphertext,
            } => {
                debug!("Decrypting message with {:?}", algorithm);
                trace!(
                    "Ciphertext: {} bytes, nonce: {} bytes",
                    ciphertext.len(),
                    nonce.len()
                );
                trace!("Sender public key: {:02x?}...", &from[..8]);

                // Step 1: Verify sender if expected
                if let Some(expected_pk) = expected_sender {
                    trace!("Verifying sender public key");
                    if from.as_slice() != expected_pk.as_ref() {
                        return Err(anyhow!(
                            "Sender public key mismatch: message from unexpected sender"
                        ));
                    }
                    trace!("Sender verification passed");
                }

                // Step 2: Derive shared secret using direct ECDH
                trace!("Deriving shared secret via ECDH");
                let shared_secret = self.derive_shared(&Self::Public::from(*from))?;
                trace!("Shared secret derived successfully");

                // Step 3: Derive encryption key using HKDF with salt
                trace!("Deriving decryption key with HKDF");
                let mut encryption_key = [0u8; 32];
                let hkdf = Hkdf::<Sha256>::new(Some(HKDF_SALT), &shared_secret);
                hkdf.expand(algorithm.info_string().as_bytes(), &mut encryption_key)
                    .map_err(|e| anyhow!("HKDF expansion failed: {e}"))?;
                trace!("Decryption key derived");

                // Step 4: Decrypt with appropriate algorithm
                trace!("Decrypting ciphertext with {:?}", algorithm);
                let plaintext = match algorithm {
                    EncryptionAlgorithm::XChaCha20Poly1305 => {
                        if nonce.len() != 24 {
                            return Err(anyhow!(
                                "Invalid XChaCha20 nonce length: expected 24 bytes"
                            ));
                        }
                        let nonce_array = XNonce::try_from(nonce.as_slice())
                            .map_err(|e| anyhow!("Invalid XChaCha20 nonce: {e}"))?;
                        let cipher = XChaCha20Poly1305::new(&encryption_key.into());
                        cipher
                            .decrypt(&nonce_array, ciphertext.as_ref())
                            .map_err(|e| anyhow!("XChaCha20 decryption failed: {e}"))
                    }
                    EncryptionAlgorithm::AesGcm256 => {
                        if nonce.len() != 12 {
                            return Err(anyhow!("Invalid AES-GCM nonce length: expected 12 bytes"));
                        }
                        let nonce_array = AesNonce::try_from(nonce.as_slice())
                            .map_err(|e| anyhow!("Invalid AES-GCM nonce: {e}"))?;
                        let cipher = Aes256Gcm::new(&encryption_key.into());
                        cipher
                            .decrypt(&nonce_array, ciphertext.as_ref())
                            .map_err(|e| anyhow!("AES-GCM decryption failed: {e}"))
                    }
                    EncryptionAlgorithm::ChaCha20Poly1305 => {
                        if nonce.len() != 12 {
                            return Err(anyhow!(
                                "Invalid ChaCha20 nonce length: expected 12 bytes"
                            ));
                        }
                        let nonce_array = ChachaNonce::try_from(nonce.as_slice())
                            .map_err(|e| anyhow!("Invalid ChaCha20 nonce: {e}"))?;
                        let cipher = ChaCha20Poly1305::new(&encryption_key.into());
                        cipher
                            .decrypt(&nonce_array, ciphertext.as_ref())
                            .map_err(|e| anyhow!("ChaCha20 decryption failed: {e}"))
                    }
                }?;

                debug!(
                    "Decryption complete: {} bytes ciphertext -> {} bytes plaintext",
                    ciphertext.len(),
                    plaintext.len()
                );
                Ok(plaintext)
            }
        }
    }
}

impl<K: SharedSecret> Cipher for K {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{Ed25519, PublicKey, Signer, Sr25519};

    fn sr(suri: &str) -> Signer<Sr25519> {
        Signer::from_suri(suri).unwrap()
    }

    fn ed(suri: &str) -> Signer<Ed25519> {
        Signer::from_suri(suri).unwrap()
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip_sr25519() {
        let cipher = sr("//Alice");

        // Alice's own public key, for self-encryption
        let public_key = cipher.public_key();

        let plaintext = b"Hello, World!";
        let encrypted = cipher
            .encrypt(
                plaintext,
                &public_key,
                EncryptionAlgorithm::XChaCha20Poly1305,
            )
            .unwrap();
        let decrypted = cipher.decrypt(&encrypted, None).unwrap();

        assert_eq!(plaintext.to_vec(), decrypted);
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip_ed25519() {
        let cipher = ed("//Alice");
        let public_key = cipher.public_key();

        let plaintext = b"Hello, World!";
        let encrypted = cipher
            .encrypt(plaintext, &public_key, EncryptionAlgorithm::AesGcm256)
            .unwrap();
        let decrypted = cipher.decrypt(&encrypted, None).unwrap();

        assert_eq!(plaintext.to_vec(), decrypted);
    }

    #[test]
    fn test_cross_party_encryption_sr25519() {
        let alice = sr("//Alice");
        let bob = sr("//Bob");

        let plaintext = b"Secret from Alice to Bob";
        let encrypted = alice
            .encrypt(
                plaintext,
                &bob.public_key(),
                EncryptionAlgorithm::XChaCha20Poly1305,
            )
            .unwrap();
        let decrypted = bob.decrypt(&encrypted, Some(&alice.public_key())).unwrap();

        assert_eq!(plaintext.to_vec(), decrypted);
    }

    #[test]
    fn test_cross_party_encryption_ed25519() {
        let alice = ed("//Alice");
        let bob = ed("//Bob");

        let encrypted = alice
            .encrypt(
                b"hi",
                &bob.public_key(),
                EncryptionAlgorithm::ChaCha20Poly1305,
            )
            .unwrap();

        assert_eq!(
            bob.decrypt(&encrypted, Some(&alice.public_key())).unwrap(),
            b"hi"
        );
    }

    #[test]
    fn test_sender_verification_fails() {
        let alice = sr("//Alice");
        let bob = sr("//Bob");
        let charlie = sr("//Charlie");

        let encrypted = alice
            .encrypt(
                b"From Alice",
                &bob.public_key(),
                EncryptionAlgorithm::XChaCha20Poly1305,
            )
            .unwrap();

        // Should fail: expecting message from Charlie, but it's from Alice
        let result = bob.decrypt(&encrypted, Some(&charlie.public_key()));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mismatch"));
    }

    #[test]
    fn test_derive_shared_sr25519() {
        let alice = sr("//Alice");
        let bob = sr("//Bob");

        // Diffie-Hellman property: both sides derive the same secret
        assert_eq!(
            alice.derive_shared(&bob.public_key()).unwrap(),
            bob.derive_shared(&alice.public_key()).unwrap()
        );
    }

    #[test]
    fn test_derive_shared_ed25519() {
        let alice = ed("//Alice");
        let bob = ed("//Bob");

        assert_eq!(
            alice.derive_shared(&bob.public_key()).unwrap(),
            bob.derive_shared(&alice.public_key()).unwrap()
        );
    }

    #[test]
    fn test_invalid_receiver_public_key_is_rejected() {
        // Not a valid compressed point encoding
        let invalid = PublicKey::<Sr25519>::from([0xffu8; 32]);
        assert!(sr("//Alice")
            .encrypt(b"x", &invalid, EncryptionAlgorithm::XChaCha20Poly1305)
            .is_err());
    }
}
