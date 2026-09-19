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
//! Connectivity Protocol Meshtastic Transport v1 frame parsing.
//!
//! Implements the receiver-side control-byte and frame validation rules from
//! `src/protobufs/transport/meshtastic/v1.md` §7-9 and §14 (receiver
//! algorithm §27) exactly: this module performs *no* reassembly and *no* I/O,
//! it only turns one `Data.payload` byte string into a validated [`Frame`] or
//! rejects it. All numeric limits here are wire-protocol constants, not
//! configuration: they can only be made *stricter* by configuration, never
//! relaxed.

use bytes::Bytes;
use sha2::{Digest, Sha256};

/// Transport version this parser implements (spec §7).
pub const TRANSPORT_VERSION: u8 = 1;

/// `SINGLE` control byte (`type = 00`, `version = 1`).
pub const SINGLE_CONTROL: u8 = 0x01;
/// `FRAGMENT` control byte (`type = 01`, `version = 1`).
pub const FRAGMENT_CONTROL: u8 = 0x41;

/// Maximum upper-layer payload carried by one `SINGLE` frame (spec §8).
pub const MAX_SINGLE_BODY: usize = 220;

/// Length of the `FRAGMENT` header: control byte + message id + descriptor
/// (spec §9).
pub const FRAGMENT_HEADER_LEN: usize = 8;
/// Length of the `message_id` field, in bytes (spec §9.1).
pub const MESSAGE_ID_LEN: usize = 6;
/// Maximum fragment body size (spec §9).
pub const MAX_FRAGMENT_BODY: usize = 213;
/// Minimum valid `fragment_count` (spec §9.2).
pub const MIN_FRAGMENT_COUNT: u8 = 2;
/// Maximum valid `fragment_count` (spec §9.2, §17).
pub const MAX_FRAGMENT_COUNT: usize = 16;
/// Maximum reassembled payload size: `MAX_FRAGMENT_COUNT * MAX_FRAGMENT_BODY`
/// (spec §10, §17).
pub const MAX_REASSEMBLED_BYTES: usize = MAX_FRAGMENT_COUNT * MAX_FRAGMENT_BODY;

/// A `message_id`: the first six raw bytes of `SHA256(raw_payload)`, copied in
/// digest order (spec §9.1). This is a transport-local reassembly tag, never
/// an authentication token.
pub type MessageId = [u8; MESSAGE_ID_LEN];

/// One validated `FRAGMENT` frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FragmentFrame {
    /// Reassembly tag shared by every fragment of one upper-layer payload.
    pub message_id: MessageId,
    /// Zero-based fragment position.
    pub index: u8,
    /// Total number of fragments for this payload (`2..=16`).
    pub count: u8,
    /// This fragment's exact body bytes.
    pub body: Bytes,
}

/// A structurally validated Transport v1 frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// A complete upper-layer payload carried in one frame.
    Single(Bytes),
    /// One fragment of a multi-frame upper-layer payload.
    Fragment(FragmentFrame),
}

/// Reasons a `Data.payload` byte string is not a valid Transport v1 frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// `Data.payload` was empty; there is no control byte to parse.
    #[error("empty payload")]
    EmptyPayload,
    /// The control byte's version field is not `1`.
    #[error("unsupported transport version {0}")]
    UnsupportedVersion(u8),
    /// The control byte's type field is one of the two reserved values.
    #[error("reserved frame type")]
    ReservedFrameType,
    /// A `SINGLE` frame carried zero body bytes.
    #[error("empty SINGLE body")]
    EmptySingleBody,
    /// A `SINGLE` frame's body exceeded [`MAX_SINGLE_BODY`].
    #[error("SINGLE body too large: {0} bytes (max {MAX_SINGLE_BODY})")]
    SingleBodyTooLarge(usize),
    /// A `FRAGMENT` frame was shorter than [`FRAGMENT_HEADER_LEN`].
    #[error("truncated FRAGMENT header: {0} bytes (min {FRAGMENT_HEADER_LEN})")]
    TruncatedFragmentHeader(usize),
    /// The descriptor decoded to a `fragment_count` outside `2..=16`.
    #[error("invalid fragment_count {0}")]
    InvalidFragmentCount(u8),
    /// The descriptor's `fragment_index` was `>= fragment_count`.
    #[error("fragment_index {index} out of bounds for fragment_count {count}")]
    FragmentIndexOutOfBounds {
        /// Decoded zero-based fragment index.
        index: u8,
        /// Decoded fragment count.
        count: u8,
    },
    /// A `FRAGMENT` body was empty or exceeded [`MAX_FRAGMENT_BODY`].
    #[error("fragment body length {0} out of bounds (1..={MAX_FRAGMENT_BODY})")]
    FragmentBodyOutOfBounds(usize),
    /// A non-final fragment's body was not exactly [`MAX_FRAGMENT_BODY`] bytes.
    #[error(
        "non-final fragment_index {index} of fragment_count {count} has body length {len} \
         (must equal {MAX_FRAGMENT_BODY})"
    )]
    NonFinalFragmentShort {
        /// Decoded zero-based fragment index.
        index: u8,
        /// Decoded fragment count.
        count: u8,
        /// Actual body length.
        len: usize,
    },
}

/// Parse and validate one `Data.payload` byte string as a Transport v1 frame.
///
/// This performs only the structural checks defined by the spec (§14); it
/// does not perform reassembly, message-id verification, or PortNum/PKI
/// filtering — those are the caller's responsibility.
pub fn parse_frame(payload: Bytes) -> Result<Frame, FrameError> {
    let control = *payload.first().ok_or(FrameError::EmptyPayload)?;
    let version = control & 0x3f;
    let frame_type = control >> 6;

    if version != TRANSPORT_VERSION {
        return Err(FrameError::UnsupportedVersion(version));
    }

    match frame_type {
        0b00 => parse_single(payload),
        0b01 => parse_fragment(payload),
        _ => Err(FrameError::ReservedFrameType),
    }
}

/// Parse the body of a `SINGLE` frame (control byte already validated).
fn parse_single(payload: Bytes) -> Result<Frame, FrameError> {
    let body = payload.slice(1..);
    if body.is_empty() {
        return Err(FrameError::EmptySingleBody);
    }
    if body.len() > MAX_SINGLE_BODY {
        return Err(FrameError::SingleBodyTooLarge(body.len()));
    }
    Ok(Frame::Single(body))
}

/// Parse the body of a `FRAGMENT` frame (control byte already validated).
fn parse_fragment(payload: Bytes) -> Result<Frame, FrameError> {
    if payload.len() < FRAGMENT_HEADER_LEN {
        return Err(FrameError::TruncatedFragmentHeader(payload.len()));
    }

    let mut message_id: MessageId = [0u8; MESSAGE_ID_LEN];
    message_id.copy_from_slice(&payload[1..1 + MESSAGE_ID_LEN]);

    let descriptor = payload[1 + MESSAGE_ID_LEN];
    let count = ((descriptor >> 4) & 0x0f) + 1;
    let index = descriptor & 0x0f;

    if !(MIN_FRAGMENT_COUNT..=MAX_FRAGMENT_COUNT as u8).contains(&count) {
        return Err(FrameError::InvalidFragmentCount(count));
    }
    if index >= count {
        return Err(FrameError::FragmentIndexOutOfBounds { index, count });
    }

    let body = payload.slice(FRAGMENT_HEADER_LEN..);
    if body.is_empty() || body.len() > MAX_FRAGMENT_BODY {
        return Err(FrameError::FragmentBodyOutOfBounds(body.len()));
    }
    // Every non-final fragment MUST carry a full-size body (spec §14).
    if index < count - 1 && body.len() != MAX_FRAGMENT_BODY {
        return Err(FrameError::NonFinalFragmentShort {
            index,
            count,
            len: body.len(),
        });
    }

    Ok(Frame::Fragment(FragmentFrame {
        message_id,
        index,
        count,
        body,
    }))
}

// ---------------------------------------------------------------------------
// Encoding (sender side)
// ---------------------------------------------------------------------------
//
// Used by `edge sensor meshtastic` to turn one exact, already-encoded
// `SignedEnvelope` byte string into the `Data.payload` byte strings sent over
// the mesh. This is the exact inverse of `parse_frame`/`parse_fragment` above
// and MUST stay in lock-step with their constants — there is deliberately
// only one copy of `SINGLE_CONTROL`, `FRAGMENT_CONTROL`, `MAX_SINGLE_BODY`,
// `MAX_FRAGMENT_BODY` and `MAX_FRAGMENT_COUNT` in this module, shared by both
// the ingress (receive) and sensor (send) code paths.

/// Reasons `raw_payload` cannot be encoded as Transport v1 frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    /// There is nothing to send.
    #[error("empty payload")]
    EmptyPayload,
    /// The payload would require more fragments than [`MAX_FRAGMENT_COUNT`]
    /// (the wire `fragment_count` nibble cannot represent more, and the
    /// sender must not silently truncate data).
    #[error("payload requires {0} fragments, exceeding the maximum of {MAX_FRAGMENT_COUNT}")]
    TooManyFragments(usize),
}

/// Compute the transport-level `message_id`: the first [`MESSAGE_ID_LEN`]
/// bytes of `SHA256(raw_payload)`, copied in digest order (spec §9.1).
///
/// This is a transport-local reassembly tag, distinct from (and much shorter
/// than) any upper-layer envelope/dedup id.
pub fn message_id(raw_payload: &[u8]) -> MessageId {
    let digest = Sha256::digest(raw_payload);
    let mut id: MessageId = [0u8; MESSAGE_ID_LEN];
    id.copy_from_slice(&digest[..MESSAGE_ID_LEN]);
    id
}

/// Encode `raw_payload` (the exact, already-serialized upper-layer bytes,
/// e.g. a `SignedEnvelope`) into one or more Transport v1 wire frames, ready
/// to be sent one-per-packet as `Data.payload`.
///
/// A payload fitting within [`MAX_SINGLE_BODY`] becomes a single `SINGLE`
/// frame. A larger payload is split into fixed-size [`MAX_FRAGMENT_BODY`]
/// chunks (the last chunk may be shorter) and emitted as `FRAGMENT` frames
/// sharing one [`message_id`]. Never truncates: payloads that would need
/// more than [`MAX_FRAGMENT_COUNT`] fragments are rejected outright.
pub fn encode_frames(raw_payload: &[u8]) -> Result<Vec<Vec<u8>>, EncodeError> {
    if raw_payload.is_empty() {
        return Err(EncodeError::EmptyPayload);
    }

    if raw_payload.len() <= MAX_SINGLE_BODY {
        let mut frame = Vec::with_capacity(1 + raw_payload.len());
        frame.push(SINGLE_CONTROL);
        frame.extend_from_slice(raw_payload);
        return Ok(vec![frame]);
    }

    let chunks: Vec<&[u8]> = raw_payload.chunks(MAX_FRAGMENT_BODY).collect();
    if chunks.len() > MAX_FRAGMENT_COUNT {
        return Err(EncodeError::TooManyFragments(chunks.len()));
    }
    // `raw_payload.len() > MAX_SINGLE_BODY (220)` and `MAX_FRAGMENT_BODY`
    // (213) together guarantee at least two chunks, matching
    // `MIN_FRAGMENT_COUNT`.
    let count = chunks.len() as u8;
    let id = message_id(raw_payload);

    Ok(chunks
        .into_iter()
        .enumerate()
        .map(|(index, body)| {
            let index = index as u8;
            let descriptor = ((count - 1) << 4) | index;
            let mut frame = Vec::with_capacity(FRAGMENT_HEADER_LEN + body.len());
            frame.push(FRAGMENT_CONTROL);
            frame.extend_from_slice(&id);
            frame.push(descriptor);
            frame.extend_from_slice(body);
            frame
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(body: &[u8]) -> Bytes {
        let mut payload = vec![SINGLE_CONTROL];
        payload.extend_from_slice(body);
        Bytes::from(payload)
    }

    fn fragment(message_id: [u8; 6], index: u8, count: u8, body: &[u8]) -> Bytes {
        let descriptor = ((count - 1) << 4) | index;
        let mut payload = vec![FRAGMENT_CONTROL];
        payload.extend_from_slice(&message_id);
        payload.push(descriptor);
        payload.extend_from_slice(body);
        Bytes::from(payload)
    }

    #[test]
    fn valid_single_frame_is_accepted() {
        let frame = parse_frame(single(b"hello")).expect("valid frame");
        assert_eq!(frame, Frame::Single(Bytes::from_static(b"hello")));
    }

    #[test]
    fn maximum_size_single_body_is_accepted() {
        let body = vec![0xabu8; MAX_SINGLE_BODY];
        let frame = parse_frame(single(&body)).expect("valid frame");
        assert_eq!(frame, Frame::Single(Bytes::from(body)));
    }

    #[test]
    fn oversized_single_body_is_rejected() {
        let body = vec![0xabu8; MAX_SINGLE_BODY + 1];
        let err = parse_frame(single(&body)).unwrap_err();
        assert_eq!(err, FrameError::SingleBodyTooLarge(MAX_SINGLE_BODY + 1));
    }

    #[test]
    fn zero_length_single_body_is_rejected() {
        let err = parse_frame(Bytes::from_static(&[SINGLE_CONTROL])).unwrap_err();
        assert_eq!(err, FrameError::EmptySingleBody);
    }

    #[test]
    fn empty_payload_is_rejected() {
        let err = parse_frame(Bytes::new()).unwrap_err();
        assert_eq!(err, FrameError::EmptyPayload);
    }

    #[test]
    fn unsupported_version_is_rejected() {
        // Version field = 2, type bits = SINGLE.
        let err = parse_frame(Bytes::from_static(&[0x02, 0xaa])).unwrap_err();
        assert_eq!(err, FrameError::UnsupportedVersion(2));
    }

    #[test]
    fn reserved_frame_types_are_rejected() {
        for control in [0x81u8, 0xc1u8] {
            let err = parse_frame(Bytes::from(vec![control, 0xaa])).unwrap_err();
            assert_eq!(err, FrameError::ReservedFrameType);
        }
    }

    #[test]
    fn valid_fragment_frame_is_accepted() {
        let id = [0x2c, 0xf2, 0x4d, 0xba, 0x5f, 0xb0];
        let body = vec![0x11u8; MAX_FRAGMENT_BODY];
        let frame = parse_frame(fragment(id, 1, 3, &body)).expect("valid frame");
        match frame {
            Frame::Fragment(f) => {
                assert_eq!(f.message_id, id);
                assert_eq!(f.index, 1);
                assert_eq!(f.count, 3);
                assert_eq!(f.body, Bytes::from(body));
            }
            other => panic!("expected fragment, got {other:?}"),
        }
    }

    #[test]
    fn final_fragment_may_be_shorter() {
        let id = [1, 2, 3, 4, 5, 6];
        let body = vec![0x22u8; 1];
        let frame = parse_frame(fragment(id, 2, 3, &body)).expect("valid frame");
        assert_eq!(
            frame,
            Frame::Fragment(FragmentFrame {
                message_id: id,
                index: 2,
                count: 3,
                body: Bytes::from(body),
            })
        );
    }

    #[test]
    fn maximum_fragment_body_is_accepted() {
        let id = [0u8; 6];
        let body = vec![0x33u8; MAX_FRAGMENT_BODY];
        parse_frame(fragment(id, 0, 2, &body)).expect("valid frame");
    }

    #[test]
    fn payload_shorter_than_fragment_header_is_rejected() {
        // Control byte + 5 bytes: too short for a full 8-byte header.
        let payload = Bytes::from(vec![FRAGMENT_CONTROL, 1, 2, 3, 4, 5]);
        let err = parse_frame(payload).unwrap_err();
        assert_eq!(err, FrameError::TruncatedFragmentHeader(6));
    }

    #[test]
    fn zero_fragment_count_is_rejected() {
        // descriptor = 0x00 -> count = 1, which is malformed per spec §9.2.
        let id = [0u8; 6];
        let err = parse_frame(fragment(id, 0, 1, &[0x01])).unwrap_err();
        assert_eq!(err, FrameError::InvalidFragmentCount(1));
    }

    #[test]
    fn fragment_index_out_of_bounds_is_rejected() {
        // Manually craft descriptor with index >= count (not reachable via
        // the `fragment` helper, which always emits a valid index).
        let id = [0u8; 6];
        let descriptor = ((2u8 - 1) << 4) | 2; // count=2, index=2
        let mut payload = vec![FRAGMENT_CONTROL];
        payload.extend_from_slice(&id);
        payload.push(descriptor);
        payload.push(0x01);
        let err = parse_frame(Bytes::from(payload)).unwrap_err();
        assert_eq!(
            err,
            FrameError::FragmentIndexOutOfBounds { index: 2, count: 2 }
        );
    }

    #[test]
    fn oversized_fragment_body_is_rejected() {
        let id = [0u8; 6];
        let body = vec![0x44u8; MAX_FRAGMENT_BODY + 1];
        let err = parse_frame(fragment(id, 1, 2, &body)).unwrap_err();
        assert_eq!(
            err,
            FrameError::FragmentBodyOutOfBounds(MAX_FRAGMENT_BODY + 1)
        );
    }

    #[test]
    fn short_non_final_fragment_body_is_rejected() {
        let id = [0u8; 6];
        let err = parse_frame(fragment(id, 0, 2, &[0x01, 0x02])).unwrap_err();
        assert_eq!(
            err,
            FrameError::NonFinalFragmentShort {
                index: 0,
                count: 2,
                len: 2,
            }
        );
    }

    #[test]
    fn message_id_bytes_are_preserved_in_digest_order() {
        // message_id is copied byte-for-byte, never byte-swapped.
        let id = [0xffu8, 0x00, 0x11, 0x22, 0x33, 0x44];
        let frame = parse_frame(fragment(id, 0, 2, &[0u8; MAX_FRAGMENT_BODY])).unwrap();
        match frame {
            Frame::Fragment(f) => assert_eq!(f.message_id, id),
            other => panic!("expected fragment, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Encoding (sender side)
    // -----------------------------------------------------------------

    #[test]
    fn empty_payload_is_rejected_for_encoding() {
        assert_eq!(encode_frames(&[]).unwrap_err(), EncodeError::EmptyPayload);
    }

    #[test]
    fn small_payload_encodes_as_single_frame() {
        let payload = b"telemetry envelope bytes";
        let frames = encode_frames(payload).expect("encodes");
        assert_eq!(frames.len(), 1);
        assert_eq!(
            parse_frame(Bytes::from(frames[0].clone())).unwrap(),
            Frame::Single(Bytes::from_static(payload))
        );
    }

    #[test]
    fn maximum_size_payload_still_encodes_as_single_frame() {
        let payload = vec![0xab; MAX_SINGLE_BODY];
        let frames = encode_frames(&payload).expect("encodes");
        assert_eq!(frames.len(), 1);
        assert_eq!(
            parse_frame(Bytes::from(frames[0].clone())).unwrap(),
            Frame::Single(Bytes::from(payload))
        );
    }

    #[test]
    fn one_byte_over_single_body_requires_two_fragments() {
        let payload = vec![0xcd; MAX_SINGLE_BODY + 1];
        let frames = encode_frames(&payload).expect("encodes");
        assert_eq!(frames.len(), 2);
        for frame in &frames {
            match parse_frame(Bytes::from(frame.clone())).unwrap() {
                Frame::Fragment(f) => assert_eq!(f.count, 2),
                other => panic!("expected fragment, got {other:?}"),
            }
        }
    }

    #[test]
    fn multi_fragment_envelope_round_trips_exactly() {
        // Large enough to require several `MAX_FRAGMENT_BODY`-sized chunks
        // plus a shorter final one.
        let payload: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let frames = encode_frames(&payload).expect("encodes");
        assert!(frames.len() > 2, "expected a genuinely multi-fragment case");

        let mut reassembled = Vec::new();
        let mut expected_id = None;
        for (expected_index, frame) in frames.iter().enumerate() {
            match parse_frame(Bytes::from(frame.clone())).unwrap() {
                Frame::Fragment(f) => {
                    assert_eq!(f.index as usize, expected_index);
                    assert_eq!(f.count as usize, frames.len());
                    let id = *expected_id.get_or_insert(f.message_id);
                    assert_eq!(f.message_id, id, "message_id must be shared");
                    reassembled.extend_from_slice(&f.body);
                }
                other => panic!("expected fragment, got {other:?}"),
            }
        }
        assert_eq!(reassembled, payload);
        assert_eq!(expected_id.unwrap(), message_id(&payload));
    }

    #[test]
    fn message_id_is_first_six_sha256_bytes() {
        let payload = b"hello meshtastic";
        let digest = Sha256::digest(payload);
        assert_eq!(message_id(payload), digest[..MESSAGE_ID_LEN]);
    }

    #[test]
    fn oversized_envelope_is_rejected_rather_than_truncated() {
        // One byte more than fits in `MAX_FRAGMENT_COUNT * MAX_FRAGMENT_BODY`.
        let payload = vec![0u8; MAX_REASSEMBLED_BYTES + 1];
        let err = encode_frames(&payload).unwrap_err();
        assert_eq!(err, EncodeError::TooManyFragments(MAX_FRAGMENT_COUNT + 1));
    }
}
