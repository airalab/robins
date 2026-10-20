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
//! Binary-safe data input, encryption and display of stored bytes.

use super::{
    args::{CipherScheme, DataInputArgs, DecryptArgs, EncryptionArgs},
    output::hex0x,
};
use anyhow::{anyhow, bail, Context, Result};
use libcps::crypto::{Cipher, EncryptedMessage, EncryptionAlgorithm, PublicKey, Scheme, Signer};
use parity_scale_codec::{Decode, Encode};
use serde_json::{json, Value};
use std::{
    io::Read,
    path::Path,
    str::FromStr,
    sync::atomic::{AtomicBool, Ordering},
};

static STDIN_USED: AtomicBool = AtomicBool::new(false);

/// Read all of standard input; it can be consumed only once per invocation, so
/// two arguments cannot both claim it.
pub fn read_stdin() -> Result<Vec<u8>> {
    if STDIN_USED.swap(true, Ordering::SeqCst) {
        bail!("standard input (`-`) can be used for only one argument");
    }
    let mut buf = Vec::new();
    std::io::stdin()
        .read_to_end(&mut buf)
        .context("failed to read standard input")?;
    Ok(buf)
}

/// Bytes of a `DATA` argument: the text itself, or standard input for `-`.
pub fn read_arg(data: &str) -> Result<Vec<u8>> {
    if data == "-" {
        read_stdin()
    } else {
        Ok(data.as_bytes().to_vec())
    }
}

/// Bytes of a file argument; `-` is standard input.
pub fn read_file(path: &Path) -> Result<Vec<u8>> {
    if path == Path::new("-") {
        return read_stdin();
    }
    std::fs::read(path).with_context(|| format!("failed to read {}", path.display()))
}

impl DataInputArgs {
    /// The bytes to store.
    pub fn read(&self) -> Result<Vec<u8>> {
        match (&self.data, &self.file) {
            (Some(data), None) => read_arg(data),
            (None, Some(file)) => read_file(file),
            _ => bail!("provide either DATA or --file"),
        }
    }
}

/// Run `$body` with `$keypair` aliased to the scheme marker type of a [`CipherScheme`].
macro_rules! with_scheme {
    ($scheme:expr, $keypair:ident => $body:expr) => {
        match $scheme {
            CipherScheme::Sr25519 => {
                type $keypair = libcps::crypto::Sr25519;
                $body
            }
            CipherScheme::Ed25519 => {
                type $keypair = libcps::crypto::Ed25519;
                $body
            }
        }
    };
}

fn keypair<S: Scheme>(suri: Option<&str>, purpose: &str) -> Result<Signer<S>> {
    let suri = suri.ok_or_else(|| anyhow!("--suri is required for {purpose}"))?;
    Ok(Signer::from_suri(suri)?)
}

/// Encrypt `data` for the receiver when one is requested; otherwise return it
/// unchanged. The result is the SCALE-encoded [`EncryptedMessage`].
pub fn seal(args: &EncryptionArgs, suri: Option<&str>, data: Vec<u8>) -> Result<Vec<u8>> {
    let Some(receiver) = args.receiver_public.as_deref() else {
        return Ok(data);
    };
    let receiver =
        libcps::identity::parse_account(receiver).map_err(|e| anyhow!("invalid receiver: {e}"))?;
    let algorithm =
        EncryptionAlgorithm::from_str(&args.cipher).map_err(|e| anyhow!("invalid cipher: {e}"))?;
    with_scheme!(args.scheme, P => {
        let signer = keypair::<P>(suri, "encryption")?;
        let receiver = PublicKey::<P>::from(receiver.0);
        Ok(signer.encrypt(&data, &receiver, algorithm)?.encode())
    })
}

/// Decrypt `data` when `--decrypt` is set and it is an encrypted message; any
/// other data is returned unchanged.
pub fn open(args: &DecryptArgs, suri: Option<&str>, data: Vec<u8>) -> Result<Vec<u8>> {
    if !args.decrypt {
        return Ok(data);
    }
    let mut slice = data.as_slice();
    let message = match EncryptedMessage::decode(&mut slice) {
        Ok(message) if slice.is_empty() => message,
        _ => return Ok(data),
    };
    with_scheme!(args.scheme, P => {
        let signer = keypair::<P>(suri, "decryption")?;
        signer
            .decrypt(&message, None)
            .map_err(|e| anyhow!("failed to decrypt: {e}"))
    })
}

/// Stored bytes with text and hex renderings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataView(pub Vec<u8>);

impl DataView {
    /// UTF-8 text, if the bytes are valid UTF-8.
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    /// Single-line rendering for human output: text if valid UTF-8, hex otherwise.
    pub fn human(&self) -> String {
        match self.text() {
            Some(text) => text.to_string(),
            None => hex0x(&self.0),
        }
    }

    /// JSON object with the hex bytes and, for valid UTF-8, the text.
    pub fn json(&self) -> Value {
        json!({ "hex": hex0x(&self.0), "text": self.text() })
    }
}

/// Human rendering of optional data, `-` when unset.
pub fn human_opt(data: &Option<DataView>) -> String {
    data.as_ref().map_or_else(|| "-".into(), DataView::human)
}

/// JSON of optional data, `null` when unset.
pub fn json_opt(data: &Option<DataView>) -> Value {
    data.as_ref().map_or(Value::Null, DataView::json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(receiver: &str) -> EncryptionArgs {
        EncryptionArgs {
            receiver_public: Some(receiver.to_string()),
            cipher: "xchacha20".into(),
            scheme: CipherScheme::Sr25519,
        }
    }

    #[test]
    fn views_render_text_and_binary() {
        assert_eq!(DataView(b"hello".to_vec()).human(), "hello");
        let binary = DataView(vec![0xff, 0x00]);
        assert_eq!(binary.human(), "0xff00");
        assert_eq!(binary.json()["text"], Value::Null);
        assert_eq!(binary.json()["hex"], "0xff00");
        assert_eq!(human_opt(&None), "-");
        assert_eq!(json_opt(&None), Value::Null);
    }

    #[test]
    fn seal_then_open_round_trips() {
        let bob = Signer::<libcps::crypto::Sr25519>::from_suri("//Bob").unwrap();
        let receiver = bob.account_id().to_string();
        let sealed = seal(&enc(&receiver), Some("//Alice"), b"secret".to_vec()).unwrap();
        assert_ne!(sealed, b"secret");

        let dec = DecryptArgs {
            decrypt: true,
            scheme: CipherScheme::Sr25519,
        };
        assert_eq!(open(&dec, Some("//Bob"), sealed).unwrap(), b"secret");
    }

    #[test]
    fn plain_data_passes_through() {
        let none = EncryptionArgs {
            receiver_public: None,
            cipher: "xchacha20".into(),
            scheme: CipherScheme::Sr25519,
        };
        assert_eq!(seal(&none, None, b"x".to_vec()).unwrap(), b"x");
        let dec = DecryptArgs {
            decrypt: true,
            scheme: CipherScheme::Sr25519,
        };
        assert_eq!(
            open(&dec, None, b"not a message".to_vec()).unwrap(),
            b"not a message"
        );
    }

    #[test]
    fn encryption_needs_a_suri() {
        let err = seal(
            &enc("5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"),
            None,
            vec![1],
        )
        .unwrap_err();
        assert!(err.to_string().contains("--suri"));
    }
}
