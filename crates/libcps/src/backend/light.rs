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
//! Embedded light client.

use super::Api;
use crate::error::{Error, Result};
use subxt::lightclient::LightClient;

/// Start a light client on the embedded Polkadot relay chain and connect to the
/// Robonomics parachain.
///
/// The Subxt client owns the light-client state, so nothing else has to be kept
/// alive alongside it; it lives as long as the [`Api`] (and thus the
/// [`crate::Client`]).
pub(crate) async fn connect_light() -> Result<Api> {
    let (light_client, _) = LightClient::relay_chain(robonomics_chain_spec::POLKADOT_RELAY_RAW)
        .map_err(|e| Error::LightClient(Box::new(e)))?;
    let rpc = light_client
        .parachain(robonomics_chain_spec::POLKADOT_PARACHAIN_RAW)
        .map_err(|e| Error::LightClient(Box::new(e)))?;
    Api::from_rpc_client(rpc)
        .await
        .map_err(|e| Error::LightClient(Box::new(e)))
}
