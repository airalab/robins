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
//! Cryptographic types for encryption.
//!
//! This module provides types for encryption algorithms and encrypted message
//! formats used throughout the library.

use parity_scale_codec::{Decode, Encode};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Supported encryption algorithms.
///
/// All algorithms use AEAD (Authenticated Encryption with Associated Data)
/// to provide both confidentiality and authenticity.
///
/// Parsed from the names `xchacha20`, `aesgcm256` and `chacha20` (case-insensitive,
/// with a few aliases such as `aes-256-gcm`); `Display` prints the HKDF info
/// string (`xchacha20poly1305`, `aesgcm256`, `chacha20poly1305`).///
/// # Examples
///
/// ```
/// use libcps::crypto::EncryptionAlgorithm;
///
/// let algo = EncryptionAlgorithm::XChaCha20Poly1305;
/// assert_eq!(algo.name(), "XChaCha20-Poly1305");
/// assert_eq!(algo.nonce_size(), 24);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, Serialize, Deserialize, Default)]
pub enum EncryptionAlgorithm {
    /// XChaCha20-Poly1305 AEAD (24-byte nonce)
    #[default]
    XChaCha20Poly1305,
    /// AES-256-GCM AEAD (12-byte nonce)
    AesGcm256,
    /// ChaCha20-Poly1305 AEAD (12-byte nonce)
    ChaCha20Poly1305,
}

impl EncryptionAlgorithm {
    /// Get the HKDF info string for this algorithm.
    ///
    /// The info string is used for domain separation in HKDF key derivation,
    /// ensuring keys derived for different algorithms are independent.
    ///
    /// # Examples
    ///
    /// ```
    /// use libcps::crypto::EncryptionAlgorithm;
    ///
    /// let algo = EncryptionAlgorithm::XChaCha20Poly1305;
    /// assert_eq!(algo.info_string(), "xchacha20poly1305");
    /// ```
    pub fn info_string(&self) -> &'static str {
        match self {
            Self::XChaCha20Poly1305 => "xchacha20poly1305",
            Self::AesGcm256 => "aesgcm256",
            Self::ChaCha20Poly1305 => "chacha20poly1305",
        }
    }

    /// Get the nonce size in bytes for this algorithm.
    ///
    /// # Examples
    ///
    /// ```
    /// use libcps::crypto::EncryptionAlgorithm;
    ///
    /// assert_eq!(EncryptionAlgorithm::XChaCha20Poly1305.nonce_size(), 24);
    /// assert_eq!(EncryptionAlgorithm::AesGcm256.nonce_size(), 12);
    /// assert_eq!(EncryptionAlgorithm::ChaCha20Poly1305.nonce_size(), 12);
    /// ```
    pub fn nonce_size(&self) -> usize {
        match self {
            Self::XChaCha20Poly1305 => 24,
            Self::AesGcm256 => 12,
            Self::ChaCha20Poly1305 => 12,
        }
    }

    /// Get the key size in bytes for this algorithm.
    ///
    /// All supported algorithms use 256-bit (32-byte) keys.
    ///
    /// # Examples
    ///
    /// ```
    /// use libcps::crypto::EncryptionAlgorithm;
    ///
    /// assert_eq!(EncryptionAlgorithm::XChaCha20Poly1305.key_size(), 32);
    /// ```
    pub fn key_size(&self) -> usize {
        32 // All algorithms use 256-bit keys
    }

    /// Get a human-readable name for this algorithm.
    ///
    /// # Examples
    ///
    /// ```
    /// use libcps::crypto::EncryptionAlgorithm;
    ///
    /// assert_eq!(EncryptionAlgorithm::XChaCha20Poly1305.name(), "XChaCha20-Poly1305");
    /// ```
    pub fn name(&self) -> &'static str {
        match self {
            Self::XChaCha20Poly1305 => "XChaCha20-Poly1305",
            Self::AesGcm256 => "AES-256-GCM",
            Self::ChaCha20Poly1305 => "ChaCha20-Poly1305",
        }
    }
}

impl fmt::Display for EncryptionAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.info_string())
    }
}

impl FromStr for EncryptionAlgorithm {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "xchacha20" | "xchacha20poly1305" | "xchacha20-poly1305" => Ok(Self::XChaCha20Poly1305),
            "aesgcm256" | "aes-256-gcm" | "aes256gcm" | "aesgcm" => Ok(Self::AesGcm256),
            "chacha20" | "chacha20poly1305" | "chacha20-poly1305" => Ok(Self::ChaCha20Poly1305),
            _ => Err(format!(
                "Unknown encryption algorithm: '{s}'. Supported: xchacha20, aesgcm256, chacha20"
            )),
        }
    }
}

/// Encrypted message format stored on-chain.
///
/// This enum is versioned to support future format changes while maintaining
/// backward compatibility. Uses SCALE codec for efficient binary serialization.
#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize)]
pub enum EncryptedMessage {
    /// Version 1 of the encrypted message format.
    ///
    /// Uses AEAD encryption with algorithm identifier, sender's public key,
    /// nonce, and ciphertext as binary data.
    V1 {
        /// Encryption algorithm
        algorithm: EncryptionAlgorithm,
        /// Sender's public key (32 bytes)
        #[serde(with = "easy_hex::serde")]
        from: [u8; 32],
        /// Nonce for the encryption (size varies by algorithm: 24 bytes for XChaCha20, 12 for AES-GCM/ChaCha20)
        #[serde(with = "easy_hex::serde")]
        nonce: Vec<u8>,
        /// Encrypted ciphertext with authentication tag
        #[serde(with = "easy_hex::serde")]
        ciphertext: Vec<u8>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_algorithm_from_str() {
        assert_eq!(
            EncryptionAlgorithm::from_str("xchacha20").unwrap(),
            EncryptionAlgorithm::XChaCha20Poly1305
        );
        assert_eq!(
            EncryptionAlgorithm::from_str("aesgcm256").unwrap(),
            EncryptionAlgorithm::AesGcm256
        );
        assert_eq!(
            EncryptionAlgorithm::from_str("chacha20").unwrap(),
            EncryptionAlgorithm::ChaCha20Poly1305
        );
    }

    #[test]
    fn test_algorithm_info_strings() {
        assert_eq!(
            EncryptionAlgorithm::XChaCha20Poly1305.info_string(),
            "xchacha20poly1305"
        );
        assert_eq!(EncryptionAlgorithm::AesGcm256.info_string(), "aesgcm256");
        assert_eq!(
            EncryptionAlgorithm::ChaCha20Poly1305.info_string(),
            "chacha20poly1305"
        );
    }

    #[test]
    fn test_nonce_sizes() {
        assert_eq!(EncryptionAlgorithm::XChaCha20Poly1305.nonce_size(), 24);
        assert_eq!(EncryptionAlgorithm::AesGcm256.nonce_size(), 12);
        assert_eq!(EncryptionAlgorithm::ChaCha20Poly1305.nonce_size(), 12);
    }

    #[test]
    fn test_default() {
        assert_eq!(
            EncryptionAlgorithm::default(),
            EncryptionAlgorithm::XChaCha20Poly1305
        );
    }
}
