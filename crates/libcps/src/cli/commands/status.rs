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
//! `cps status`: effective configuration plus live chain information.
//!
//! The chain is queried with plain JSON-RPC requests, so a full node, a remote RPC
//! service and the local light client (`cps lc`) are treated identically. Fields
//! an endpoint cannot provide degrade to "unknown" instead of failing the command.

use crate::cli::{
    config::{display_path, EffectiveConfig},
    context::Ctx,
    output::wide_fields,
};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::{process::ExitCode, time::Duration};
use subxt::rpcs::{rpc_params, RpcClient};

/// Upper bound for connecting and collecting the whole status.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Live information about the connected chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainStatus {
    /// Chain name (`system_chain`, else the runtime spec name).
    pub name: Option<String>,
    /// Genesis block hash.
    pub genesis_hash: String,
    /// Runtime specification version.
    pub spec_version: u32,
    /// Runtime implementation version.
    pub impl_version: u32,
    /// Best block number.
    pub best: u64,
    /// Finalized block number.
    pub finalized: u64,
    /// Whether the node is synchronizing; `None` if the endpoint does not say.
    pub syncing: Option<bool>,
    /// Current block while synchronizing, if reported.
    pub current_block: Option<u64>,
    /// Highest known block while synchronizing, if reported.
    pub highest_block: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeVersion {
    spec_name: Option<String>,
    spec_version: u32,
    impl_version: u32,
}

#[derive(Deserialize)]
struct Header {
    number: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Health {
    is_syncing: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SyncState {
    current_block: u64,
    highest_block: u64,
}

/// Block number from a header's `0x` hex `number` field.
fn block_number(header: &Header) -> Result<u64> {
    let digits = header.number.trim_start_matches("0x");
    u64::from_str_radix(digits, 16)
        .map_err(|_| anyhow!("malformed block number `{}`", header.number))
}

/// Readable text of an RPC error: drops the repeated wrapper prefixes.
fn rpc_error(error: subxt::rpcs::Error) -> anyhow::Error {
    let mut text = error.to_string();
    while let Some(rest) = text
        .strip_prefix("RPC error: ")
        .or_else(|| text.strip_prefix("client error: "))
    {
        text = rest.to_string();
    }
    anyhow!(text)
}

/// Query the endpoint. Genesis, runtime version and the best and finalized
/// blocks are required; the chain name and sync state are best effort.
pub async fn query(url: &str) -> Result<ChainStatus> {
    let client = RpcClient::from_insecure_url(url).await.map_err(rpc_error)?;
    let rpc = rpc_error;

    let genesis_hash: String = client
        .request("chain_getBlockHash", rpc_params![0])
        .await
        .map_err(rpc)
        .context("cannot read the genesis hash")?;
    let version: RuntimeVersion = client
        .request("state_getRuntimeVersion", rpc_params![])
        .await
        .map_err(rpc)
        .context("cannot read the runtime version")?;
    let best: Header = client
        .request("chain_getHeader", rpc_params![])
        .await
        .map_err(rpc)
        .context("cannot read the best block")?;
    let finalized_hash: String = client
        .request("chain_getFinalizedHead", rpc_params![])
        .await
        .map_err(rpc)
        .context("cannot read the finalized head")?;
    let finalized: Header = client
        .request("chain_getHeader", rpc_params![finalized_hash])
        .await
        .map_err(rpc)
        .context("cannot read the finalized block")?;

    let name = client
        .request::<String>("system_chain", rpc_params![])
        .await
        .ok()
        .or(version.spec_name);
    let syncing = client
        .request::<Health>("system_health", rpc_params![])
        .await
        .ok()
        .map(|health| health.is_syncing);
    let sync_state = match syncing {
        Some(true) => client
            .request::<SyncState>("system_syncState", rpc_params![])
            .await
            .ok(),
        _ => None,
    };

    Ok(ChainStatus {
        name,
        genesis_hash,
        spec_version: version.spec_version,
        impl_version: version.impl_version,
        best: block_number(&best)?,
        finalized: block_number(&finalized)?,
        syncing,
        current_block: sync_state.as_ref().map(|s| s.current_block),
        highest_block: sync_state.as_ref().map(|s| s.highest_block),
    })
}

/// JSON document of a status report (stable schema for scripts).
fn status_json(cfg: &EffectiveConfig, live: &Result<ChainStatus>) -> Value {
    let mut json = Map::new();
    json.insert("profile".into(), json!(cfg.profile));
    json.insert("config".into(), json!(cfg.config_path));
    json.insert("rpc".into(), json!(cfg.url));
    match live {
        Ok(chain) => {
            json.insert("connected".into(), json!(true));
            json.insert(
                "chain".into(),
                json!({
                    "name": chain.name,
                    "genesis_hash": chain.genesis_hash,
                    "spec_version": chain.spec_version,
                    "impl_version": chain.impl_version,
                    "best": chain.best,
                    "finalized": chain.finalized,
                    "syncing": chain.syncing,
                    "current_block": chain.current_block,
                    "highest_block": chain.highest_block,
                }),
            );
        }
        Err(error) => {
            json.insert("connected".into(), json!(false));
            json.insert("error".into(), json!(format!("{error:#}")));
        }
    }
    Value::Object(json)
}

/// Human-readable status report.
fn status_human(cfg: &EffectiveConfig, live: &Result<ChainStatus>) -> String {
    let mut rows = vec![
        ("Profile", cfg.profile.clone().unwrap_or_else(|| "-".into())),
        ("Config", display_path(&cfg.config_path)),
        ("RPC", cfg.url.clone()),
        ("Connected", if live.is_ok() { "yes" } else { "no" }.into()),
    ];
    let chain = match live {
        Ok(chain) => chain,
        Err(error) => {
            return format!("{}\n\nError:\n  {error:#}", wide_fields(&rows));
        }
    };
    rows.push((
        "Chain",
        chain.name.clone().unwrap_or_else(|| "unknown".into()),
    ));
    rows.push(("Genesis", chain.genesis_hash.clone()));
    rows.push(("Spec version", chain.spec_version.to_string()));
    rows.push(("Impl version", chain.impl_version.to_string()));
    rows.push(("Best", format!("#{}", chain.best)));
    rows.push(("Finalized", format!("#{}", chain.finalized)));
    rows.push((
        "Syncing",
        match chain.syncing {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown",
        }
        .into(),
    ));
    if let Some(current) = chain.current_block {
        rows.push(("Current block", format!("#{current}")));
    }
    if let Some(highest) = chain.highest_block {
        rows.push(("Highest block", format!("#{highest}")));
    }
    wide_fields(&rows)
}

/// Run `cps status`. Exits with status 2 when the live check fails, after
/// printing the effective configuration anyway.
pub async fn run(ctx: &Ctx) -> Result<ExitCode> {
    let live = match tokio::time::timeout(TIMEOUT, query(&ctx.cfg.url)).await {
        Ok(result) => result,
        Err(_) => Err(anyhow!("timed out after {}s", TIMEOUT.as_secs())),
    };
    ctx.out.result(
        || status_human(&ctx.cfg, &live),
        status_json(&ctx.cfg, &live),
    );
    Ok(if live.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_numbers_are_hex() {
        let header = Header {
            number: "0x1f".into(),
        };
        assert_eq!(block_number(&header).unwrap(), 31);
        let bad = Header {
            number: "zz".into(),
        };
        assert!(block_number(&bad).is_err());
    }
}
