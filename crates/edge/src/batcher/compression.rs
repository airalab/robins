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
//! Payload compression applied by the [`Batcher`](super::Batcher).
//!
//! Compression is a batcher concern only: the anchor treats the resulting
//! bytes as opaque. The chosen mode is never written into the payload; it is
//! described by the CPS metadata and the local format descriptor instead.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Compression applied to the serialized `SignedEnvelopeBatch`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Compression {
    /// Anchor the serialized batch as is.
    None,
    /// Compress the serialized batch as a single zstd frame.
    #[default]
    Zstd,
}

impl Compression {
    /// Stable lowercase name used in configuration, metadata and the local
    /// format descriptor.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Zstd => "zstd",
        }
    }

    /// Apply this compression to `serialized`.
    pub fn compress(self, serialized: &[u8]) -> std::io::Result<Vec<u8>> {
        match self {
            Self::None => Ok(serialized.to_vec()),
            Self::Zstd => zstd::stream::encode_all(serialized, zstd::DEFAULT_COMPRESSION_LEVEL),
        }
    }

    /// Invert [`compress`](Self::compress); used by readers and tests.
    pub fn decompress(self, payload: &[u8]) -> std::io::Result<Vec<u8>> {
        match self {
            Self::None => Ok(payload.to_vec()),
            Self::Zstd => zstd::stream::decode_all(payload),
        }
    }
}

impl fmt::Display for Compression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrips() {
        let data = b"hello hello hello hello hello hello".repeat(20);
        for compression in [Compression::None, Compression::Zstd] {
            let packed = compression.compress(&data).unwrap();
            assert_eq!(compression.decompress(&packed).unwrap(), data);
        }
    }

    #[test]
    fn none_is_identity_and_zstd_shrinks_repetitive_data() {
        let data = vec![7u8; 4096];
        assert_eq!(Compression::None.compress(&data).unwrap(), data);
        assert!(Compression::Zstd.compress(&data).unwrap().len() < data.len() / 4);
    }

    #[test]
    fn parses_lowercase_names() {
        #[derive(Deserialize)]
        struct W {
            c: Compression,
        }
        assert_eq!(
            toml::from_str::<W>("c = \"none\"").unwrap().c,
            Compression::None
        );
        assert_eq!(
            toml::from_str::<W>("c = \"zstd\"").unwrap().c,
            Compression::Zstd
        );
        assert!(toml::from_str::<W>("c = \"xz\"").is_err());
    }
}
