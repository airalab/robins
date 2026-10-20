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
//! Identity keys: generation, inspection and signing for identity permits.
//!
//! An identity account may use any signature scheme the runtime accepts for
//! identity permits: SR25519, ED25519 or ECDSA. Unlike [`crypto::Signer`](crate::crypto::Signer)
//! this module is only about signing, so it also covers ECDSA, which has no
//! ECDH-based message encryption in `libcps`.

use crate::{
    error::{Error, Result},
    types::{AccountId, PermitSignature},
};
use polkadot_sdk::sp_core::{ecdsa, ed25519, sr25519, ByteArray, Pair};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// Signature scheme of an identity key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IdentityScheme {
    /// Schnorrkel SR25519 (the Substrate default).
    #[default]
    Sr25519,
    /// ED25519.
    Ed25519,
    /// ECDSA over secp256k1; the account is the BLAKE2-256 hash of the compressed key.
    Ecdsa,
}

impl fmt::Display for IdentityScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Sr25519 => "sr25519",
            Self::Ed25519 => "ed25519",
            Self::Ecdsa => "ecdsa",
        })
    }
}

impl FromStr for IdentityScheme {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "sr25519" | "sr" => Ok(Self::Sr25519),
            "ed25519" | "ed" => Ok(Self::Ed25519),
            "ecdsa" => Ok(Self::Ecdsa),
            other => Err(Error::Crypto(format!(
                "unknown scheme `{other}`, expected sr25519, ed25519 or ecdsa"
            ))),
        }
    }
}

/// Secret key of an identity account together with its scheme.
///
/// The secret never leaves the value: [`Debug`] prints only the scheme and account.
#[derive(Clone)]
pub struct IdentityKey {
    pair: KeyPair,
}

#[derive(Clone)]
enum KeyPair {
    Sr25519(sr25519::Pair),
    Ed25519(ed25519::Pair),
    Ecdsa(ecdsa::Pair),
}

/// A freshly generated [`IdentityKey`] with the mnemonic phrase that recreates it.
pub struct GeneratedKey {
    /// The generated key.
    pub key: IdentityKey,
    /// BIP-39 mnemonic phrase; this is the only copy of the secret.
    pub phrase: String,
}

fn crypto<E: fmt::Debug>(e: E) -> Error {
    Error::Crypto(format!("{e:?}"))
}

impl IdentityKey {
    /// Generate a new random key of `scheme`.
    pub fn generate(scheme: IdentityScheme) -> Result<GeneratedKey> {
        let (key, phrase) = match scheme {
            IdentityScheme::Sr25519 => {
                let (pair, phrase, _) = sr25519::Pair::generate_with_phrase(None);
                (KeyPair::Sr25519(pair), phrase)
            }
            IdentityScheme::Ed25519 => {
                let (pair, phrase, _) = ed25519::Pair::generate_with_phrase(None);
                (KeyPair::Ed25519(pair), phrase)
            }
            IdentityScheme::Ecdsa => {
                let (pair, phrase, _) = ecdsa::Pair::generate_with_phrase(None);
                (KeyPair::Ecdsa(pair), phrase)
            }
        };
        Ok(GeneratedKey {
            key: Self { pair: key },
            phrase,
        })
    }

    /// Key of `scheme` from a secret URI: `//Alice`, a mnemonic phrase with
    /// optional derivation path, or a `0x` hex seed.
    pub fn from_suri(scheme: IdentityScheme, suri: &str) -> Result<Self> {
        let pair = match scheme {
            IdentityScheme::Sr25519 => {
                KeyPair::Sr25519(sr25519::Pair::from_string(suri, None).map_err(crypto)?)
            }
            IdentityScheme::Ed25519 => {
                KeyPair::Ed25519(ed25519::Pair::from_string(suri, None).map_err(crypto)?)
            }
            IdentityScheme::Ecdsa => {
                KeyPair::Ecdsa(ecdsa::Pair::from_string(suri, None).map_err(crypto)?)
            }
        };
        Ok(Self { pair })
    }

    /// Scheme of the key.
    pub fn scheme(&self) -> IdentityScheme {
        match self.pair {
            KeyPair::Sr25519(_) => IdentityScheme::Sr25519,
            KeyPair::Ed25519(_) => IdentityScheme::Ed25519,
            KeyPair::Ecdsa(_) => IdentityScheme::Ecdsa,
        }
    }

    /// Raw public key: 32 bytes, or the 33-byte compressed key for ECDSA.
    pub fn public(&self) -> Vec<u8> {
        match &self.pair {
            KeyPair::Sr25519(p) => p.public().to_raw_vec(),
            KeyPair::Ed25519(p) => p.public().to_raw_vec(),
            KeyPair::Ecdsa(p) => p.public().to_raw_vec(),
        }
    }

    /// Account of the key, as the runtime derives it from the signature scheme.
    pub fn account_id(&self) -> AccountId {
        match &self.pair {
            KeyPair::Ecdsa(p) => AccountId::from(polkadot_sdk::sp_crypto_hashing::blake2_256(
                p.public().as_ref(),
            )),
            _ => AccountId::from(<[u8; 32]>::try_from(self.public()).expect("32-byte key")),
        }
    }

    /// Sign `message` with the key.
    ///
    /// ECDSA hashes the message with BLAKE2-256 first, as the runtime verifier does.
    pub fn sign(&self, message: &[u8]) -> PermitSignature {
        match &self.pair {
            KeyPair::Sr25519(p) => PermitSignature::Sr25519(p.sign(message).0),
            KeyPair::Ed25519(p) => PermitSignature::Ed25519(p.sign(message).0),
            KeyPair::Ecdsa(p) => PermitSignature::Ecdsa(p.sign(message).0),
        }
    }
}

impl fmt::Debug for IdentityKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdentityKey")
            .field("scheme", &self.scheme())
            .field("account", &self.account_id().to_string())
            .finish_non_exhaustive()
    }
}

impl PermitSignature {
    /// Scheme that produced the signature.
    pub fn scheme(&self) -> IdentityScheme {
        match self {
            Self::Sr25519(_) => IdentityScheme::Sr25519,
            Self::Ed25519(_) => IdentityScheme::Ed25519,
            Self::Ecdsa(_) => IdentityScheme::Ecdsa,
        }
    }

    /// Raw signature bytes: 64 bytes, or 65 for ECDSA.
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Self::Sr25519(s) | Self::Ed25519(s) => s.to_vec(),
            Self::Ecdsa(s) => s.to_vec(),
        }
    }

    /// Signature of `scheme` from raw bytes.
    ///
    /// # Errors
    ///
    /// [`Error::Crypto`] if the length does not match the scheme.
    pub fn from_bytes(scheme: IdentityScheme, bytes: &[u8]) -> Result<Self> {
        let wrong = |expected: usize| {
            Error::Crypto(format!(
                "{scheme} signature must be {expected} bytes, got {}",
                bytes.len()
            ))
        };
        Ok(match scheme {
            IdentityScheme::Sr25519 => Self::Sr25519(bytes.try_into().map_err(|_| wrong(64))?),
            IdentityScheme::Ed25519 => Self::Ed25519(bytes.try_into().map_err(|_| wrong(64))?),
            IdentityScheme::Ecdsa => Self::Ecdsa(bytes.try_into().map_err(|_| wrong(65))?),
        })
    }
}

/// Parse an account from an SS58 address or a `0x`-prefixed 32-byte hex public key.
pub fn parse_account(text: &str) -> Result<AccountId> {
    if let Ok(account) = AccountId::from_str(text) {
        return Ok(account);
    }
    let bytes = hex::decode(text.strip_prefix("0x").unwrap_or(text))
        .map_err(|_| Error::Crypto(format!("`{text}` is neither an SS58 address nor hex")))?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|b: Vec<u8>| {
        Error::Crypto(format!("public key must be 32 bytes, got {}", b.len()))
    })?;
    Ok(AccountId::from(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use polkadot_sdk::sp_core::crypto::Ss58Codec;

    #[test]
    fn schemes_parse_and_display() {
        for scheme in [
            IdentityScheme::Sr25519,
            IdentityScheme::Ed25519,
            IdentityScheme::Ecdsa,
        ] {
            assert_eq!(
                scheme.to_string().parse::<IdentityScheme>().unwrap(),
                scheme
            );
        }
        assert!("rsa".parse::<IdentityScheme>().is_err());
    }

    #[test]
    fn alice_sr25519_matches_well_known_address() {
        let key = IdentityKey::from_suri(IdentityScheme::Sr25519, "//Alice").unwrap();
        assert_eq!(
            key.account_id().to_string(),
            "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"
        );
    }

    #[test]
    fn ecdsa_account_is_blake2_of_compressed_key() {
        let key = IdentityKey::from_suri(IdentityScheme::Ecdsa, "//Alice").unwrap();
        assert_eq!(key.public().len(), 33);
        let expected = polkadot_sdk::sp_crypto_hashing::blake2_256(&key.public());
        assert_eq!(key.account_id(), AccountId::from(expected));
    }

    #[test]
    fn signatures_verify_for_every_scheme() {
        let message = b"permit";
        let sr = IdentityKey::from_suri(IdentityScheme::Sr25519, "//Alice").unwrap();
        let ed = IdentityKey::from_suri(IdentityScheme::Ed25519, "//Alice").unwrap();
        let ec = IdentityKey::from_suri(IdentityScheme::Ecdsa, "//Alice").unwrap();

        let PermitSignature::Sr25519(s) = sr.sign(message) else {
            panic!()
        };
        let public = sr25519::Public::from_raw(sr.public().try_into().unwrap());
        assert!(sr25519::Pair::verify(
            &sr25519::Signature::from_raw(s),
            message,
            &public
        ));

        let PermitSignature::Ed25519(s) = ed.sign(message) else {
            panic!()
        };
        let public = ed25519::Public::from_raw(ed.public().try_into().unwrap());
        assert!(ed25519::Pair::verify(
            &ed25519::Signature::from_raw(s),
            message,
            &public
        ));

        let PermitSignature::Ecdsa(s) = ec.sign(message) else {
            panic!()
        };
        let public = ecdsa::Public::from_raw(ec.public().try_into().unwrap());
        assert!(ecdsa::Pair::verify(
            &ecdsa::Signature::from_raw(s),
            message,
            &public
        ));
    }

    #[test]
    fn generated_phrase_recreates_the_key() {
        for scheme in [
            IdentityScheme::Sr25519,
            IdentityScheme::Ed25519,
            IdentityScheme::Ecdsa,
        ] {
            let generated = IdentityKey::generate(scheme).unwrap();
            let again = IdentityKey::from_suri(scheme, &generated.phrase).unwrap();
            assert_eq!(generated.key.account_id(), again.account_id());
        }
    }

    #[test]
    fn signature_bytes_round_trip() {
        let key = IdentityKey::from_suri(IdentityScheme::Ecdsa, "//Bob").unwrap();
        let signature = key.sign(b"x");
        let bytes = signature.to_bytes();
        assert_eq!(bytes.len(), 65);
        assert_eq!(
            PermitSignature::from_bytes(IdentityScheme::Ecdsa, &bytes).unwrap(),
            signature
        );
        assert!(PermitSignature::from_bytes(IdentityScheme::Sr25519, &bytes).is_err());
    }

    #[test]
    fn account_parses_from_ss58_and_hex() {
        let alice = sr25519::Pair::from_string("//Alice", None)
            .unwrap()
            .public();
        let ss58 = alice.to_ss58check();
        let from_ss58 = parse_account(&ss58).unwrap();
        let from_hex =
            parse_account(&format!("0x{}", hex::encode(alice.as_ref() as &[u8]))).unwrap();
        assert_eq!(from_ss58, from_hex);
        assert!(parse_account("0x1234").is_err());
        assert!(parse_account("nonsense").is_err());
    }
}
