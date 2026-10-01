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
//! CLI command implementations.

pub mod create;
pub mod remove;
pub mod set_meta;
pub mod set_payload;
pub mod show;

use crate::display;
use anyhow::Result;
use libcps::crypto::{Cipher, EncryptionAlgorithm, PublicKey, Scheme, Signer, Sr25519};
use libcps::{AccountId, Client};
use parity_scale_codec::Encode;

/// Connection settings shared by all commands.
pub struct Connection {
    /// RPC endpoint; `None` starts the embedded light client.
    pub url: Option<String>,
    /// Secret URI of the account: signs transactions (SR25519) and, for encryption
    /// and decryption, is the keypair of the selected scheme.
    pub suri: Option<String>,
}

impl Connection {
    /// Connect to the chain, reporting progress.
    pub async fn client(&self) -> Result<Client> {
        display::progress("Connecting to blockchain...");
        let client = Client::connect(self.url.as_deref()).await?;
        match &self.url {
            Some(url) => display::info(&format!("Connected to {url}")),
            None => display::info("Connected through the embedded light client"),
        }
        Ok(client)
    }

    /// SR25519 transaction signer from the SURI; fails when no SURI was supplied.
    pub fn signer(&self) -> Result<Signer<Sr25519>> {
        let suri = self.suri.as_deref().ok_or_else(|| {
            anyhow::anyhow!(
                "This operation requires an account. Please provide --suri or set ROBONOMICS_SURI environment variable."
            )
        })?;
        let signer = Signer::<Sr25519>::from_suri(suri)?;
        display::info(&format!("Using account: {}", signer.account_id()));
        Ok(signer)
    }
}

/// Encrypt `data` for `receiver_public` (interpreted in the scheme `S` of `cipher`)
/// and SCALE-encode the message.
///
/// `what` names the data in the progress output. Fails if `cipher` or `algorithm`
/// is missing or the encryption fails.
pub fn encrypt<S: Scheme>(
    what: &str,
    cipher: Option<&Signer<S>>,
    algorithm: Option<EncryptionAlgorithm>,
    receiver_public: &[u8; 32],
    data: &[u8],
) -> Result<Vec<u8>> {
    let cipher = cipher.ok_or_else(|| anyhow::anyhow!("Cipher required for encryption"))?;
    let algorithm =
        algorithm.ok_or_else(|| anyhow::anyhow!("Algorithm required for encryption"))?;
    display::info(&format!("[E] Encrypting {what} with {algorithm}"));
    display::info(&format!(
        "[K] Receiver: {}",
        AccountId::from(*receiver_public)
    ));
    let receiver = PublicKey::<S>::from(*receiver_public);
    Ok(cipher.encrypt(data, &receiver, algorithm)?.encode())
}
