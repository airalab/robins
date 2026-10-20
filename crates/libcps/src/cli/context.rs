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
//! Per-invocation context shared by the command implementations.

use super::{
    args::ReadArgs,
    config::{Config, EffectiveConfig, WaitPolicy},
    output::{self, hex0x, Output},
    target::Target,
};
use anyhow::{anyhow, bail, Result};
use libcps::{
    crypto::{Signer, Sr25519},
    Cps, CpsAt, NodeId, Transaction, TxReceipt,
};
use serde_json::{json, Map, Value};
use std::io::{BufRead, IsTerminal, Write};

/// Effective settings, signer and output of one invocation.
pub struct Ctx {
    /// Effective configuration (endpoint, output format, wait policy).
    pub cfg: EffectiveConfig,
    /// The parsed configuration file.
    pub file: Config,
    /// Account secret URI (`--suri`).
    pub suri: Option<String>,
    /// Result renderer.
    pub out: Output,
}

impl Ctx {
    /// Settings of the named profile alone, ignoring command-line and environment
    /// overrides.
    pub fn resolved_profile(&self, name: &str) -> Result<EffectiveConfig> {
        let profile = self.file.profile(name)?;
        Ok(EffectiveConfig::of_profile(
            name,
            profile,
            self.cfg.config_path.clone(),
        ))
    }

    /// Connect to the chain, reporting progress on stderr.
    pub async fn cps(&self) -> Result<Cps> {
        output::progress("Connecting to blockchain...");
        let cps = Cps::connect(&self.cfg.url).await?;
        output::info(&format!("Connected to {}", self.cfg.url));
        Ok(cps)
    }

    /// SR25519 transaction signer from `--suri`; fails when none was supplied.
    pub fn signer(&self) -> Result<Signer<Sr25519>> {
        let suri = self.suri.as_deref().ok_or_else(|| {
            anyhow!("this operation requires an account: provide --suri or set ROBONOMICS_SURI")
        })?;
        let signer = Signer::<Sr25519>::from_suri(suri)?;
        output::info(&format!("Using account: {}", signer.account_id()));
        Ok(signer)
    }

    /// The `--suri` value, for encryption and decryption.
    pub fn suri(&self) -> Option<&str> {
        self.suri.as_deref()
    }

    /// Read context: the block selected by `--at`, else the latest finalized block.
    pub async fn reader(&self, cps: &Cps, read: &ReadArgs) -> Result<CpsAt> {
        Ok(match read.at {
            Some(hash) => cps.at(hash).await?,
            None => cps.finalized().await?,
        })
    }

    /// Canonical node of a write target, resolved at the latest finalized block.
    pub async fn canonical(&self, cps: &Cps, target: &Target) -> Result<NodeId> {
        target.node_id(&cps.finalized().await?).await
    }

    /// Sign, submit and wait as the wait policy says (`best` or `finalized`),
    /// with a spinner on stderr.
    pub async fn submit<T>(
        &self,
        cps: &Cps,
        tx: &Transaction<T>,
        signer: &Signer<Sr25519>,
    ) -> Result<TxReceipt<T>> {
        let spinner = output::spinner("Submitting transaction...");
        let receipt = async {
            let pending = cps.submit(tx, signer).await?;
            match self.cfg.wait {
                WaitPolicy::Finalized => pending.wait_finalized().await,
                WaitPolicy::Best => pending.wait_in_block().await,
            }
        }
        .await;
        spinner.finish_and_clear();
        Ok(receipt?)
    }

    /// Print the outcome of a mutation: `human` text, or JSON with the receipt
    /// hashes and the `extra` fields.
    pub fn done<T>(&self, receipt: &TxReceipt<T>, human: String, extra: Value) {
        let mut json = Map::new();
        json.insert("ok".into(), json!(true));
        json.insert("block_hash".into(), json!(hex0x(&receipt.block_hash.0)));
        json.insert(
            "extrinsic_hash".into(),
            json!(hex0x(&receipt.extrinsic_hash.0)),
        );
        if let Value::Object(extra) = extra {
            json.extend(extra);
        }
        self.out.result(|| human, Value::Object(json));
    }
}

/// Ask `question` on the terminal. Without `yes`, a non-interactive standard
/// input is an error rather than an implicit answer.
pub fn confirm(question: &str, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        bail!("confirmation required: run interactively or pass --yes");
    }
    eprint!("{question} [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
