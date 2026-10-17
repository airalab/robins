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
//! Configuration of the [`Batcher`](super::Batcher) (`[batcher]`).

use super::Compression;
use crate::batch_store::FormatDescriptor;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Protobuf message every batch payload decodes to.
pub const PAYLOAD_SCHEMA: &str = "SignedEnvelopeBatch";
/// Version of [`PAYLOAD_SCHEMA`] as used by this gateway.
pub const PAYLOAD_SCHEMA_VERSION: u32 = 1;
/// Serialization format of the payload.
pub const PAYLOAD_ENCODING: &str = "protobuf";

/// Batching limits and payload format (`[batcher]`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BatcherConfig {
    /// Maximum time the oldest message may wait in the open batch, in
    /// seconds, measured from its persisted receive time.
    pub max_batch_age_secs: u64,
    /// Maximum size in bytes of the final payload (after compression).
    ///
    /// Must not exceed the CPS payload limit ([`libcps::MAX_PAYLOAD_SIZE`]).
    pub max_batch_size: usize,
    /// Maximum number of envelopes in one batch.
    pub max_messages: usize,
    /// Compression applied to the serialized batch.
    pub compression: Compression,
}

impl Default for BatcherConfig {
    fn default() -> Self {
        Self {
            max_batch_age_secs: 30,
            max_batch_size: libcps::MAX_PAYLOAD_SIZE,
            max_messages: 100,
            compression: Compression::default(),
        }
    }
}

impl BatcherConfig {
    /// [`max_batch_age_secs`](Self::max_batch_age_secs) as a [`Duration`].
    pub fn max_batch_age(&self) -> Duration {
        Duration::from_secs(self.max_batch_age_secs)
    }

    /// Check the invariants serde cannot express.
    pub fn validate(&self) -> Result<(), String> {
        if self.max_batch_age_secs == 0 {
            return Err("batcher.max_batch_age_secs must be non-zero".to_string());
        }
        if self.max_messages == 0 {
            return Err("batcher.max_messages must be non-zero".to_string());
        }
        if self.max_batch_size == 0 {
            return Err("batcher.max_batch_size must be non-zero".to_string());
        }
        if self.max_batch_size > libcps::MAX_PAYLOAD_SIZE {
            return Err(format!(
                "batcher.max_batch_size must not exceed the CPS payload limit of {} bytes",
                libcps::MAX_PAYLOAD_SIZE
            ));
        }
        Ok(())
    }

    /// How payloads produced under this configuration must be interpreted.
    ///
    /// This is the single source for both the local format guard and the CPS
    /// metadata, so the two can never disagree.
    pub fn format_descriptor(&self) -> FormatDescriptor {
        FormatDescriptor {
            schema: PAYLOAD_SCHEMA.to_string(),
            schema_version: PAYLOAD_SCHEMA_VERSION,
            encoding: PAYLOAD_ENCODING.to_string(),
            compression: self.compression.as_str().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_cps_sized() {
        let config = BatcherConfig::default();
        config.validate().unwrap();
        assert_eq!(config.max_batch_size, 8192);
    }

    #[test]
    fn rejects_invalid_limits() {
        for config in [
            BatcherConfig {
                max_batch_age_secs: 0,
                ..Default::default()
            },
            BatcherConfig {
                max_messages: 0,
                ..Default::default()
            },
            BatcherConfig {
                max_batch_size: 0,
                ..Default::default()
            },
            BatcherConfig {
                max_batch_size: 8193,
                ..Default::default()
            },
        ] {
            assert!(config.validate().is_err(), "{config:?}");
        }
    }

    #[test]
    fn descriptor_reflects_compression() {
        let config = BatcherConfig {
            compression: Compression::None,
            ..Default::default()
        };
        let d = config.format_descriptor();
        assert_eq!(d.compression, "none");
        assert_eq!(d.schema, "SignedEnvelopeBatch");
        assert_eq!(d.encoding, "protobuf");
        assert_eq!(d.schema_version, 1);
    }
}
