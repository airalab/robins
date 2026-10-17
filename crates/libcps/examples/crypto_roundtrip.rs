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
//! Encrypt and decrypt data offline with the `crypto` module.
//!
//! No chain connection is needed. The example shows, in order:
//!
//! 1. SR25519 and ED25519 signers from a SURI and their on-chain account ids;
//! 2. encryption for a receiver with every supported AEAD algorithm;
//! 3. the SCALE and JSON encodings of an [`EncryptedMessage`] (the SCALE bytes are
//!    what is stored as a node payload);
//! 4. sender verification and tamper detection on decryption.
//!
//! ```text
//! cargo run -p libcps --example crypto_roundtrip --no-default-features
//! ```

use libcps::crypto::{
    Cipher, Ed25519, EncryptedMessage, EncryptionAlgorithm, Scheme, Signer, Sr25519,
};
use parity_scale_codec::{Decode, Encode};

const ALGORITHMS: [EncryptionAlgorithm; 3] = [
    EncryptionAlgorithm::XChaCha20Poly1305,
    EncryptionAlgorithm::AesGcm256,
    EncryptionAlgorithm::ChaCha20Poly1305,
];

/// Encrypt from `sender` to `receiver` with every algorithm and decrypt again.
///
/// The scheme `S` is a type parameter: a receiver key of another scheme does not
/// compile.
fn roundtrip<S: Scheme>(name: &str, sender_suri: &str, receiver_suri: &str) -> anyhow::Result<()> {
    let sender = Signer::<S>::from_suri(sender_suri)?;
    let receiver = Signer::<S>::from_suri(receiver_suri)?;
    println!("[{name}] sender account:   {}", sender.account_id());
    println!("[{name}] receiver account: {}", receiver.account_id());

    for algorithm in ALGORITHMS {
        let message = sender.encrypt(b"temperature=24C", &receiver.public_key(), algorithm)?;

        // Passing the expected sender key makes decryption fail for any other sender.
        let plain = receiver.decrypt(&message, Some(&sender.public_key()))?;
        assert_eq!(plain, b"temperature=24C");
        println!(
            "[{name}] {algorithm}: {} bytes encrypted, decrypted ok",
            message.encode().len()
        );
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    roundtrip::<Sr25519>("sr25519", "//Alice", "//Bob")?;
    roundtrip::<Ed25519>("ed25519", "//Alice", "//Bob")?;

    let alice = Signer::<Sr25519>::from_suri("//Alice")?;
    let bob = Signer::<Sr25519>::from_suri("//Bob")?;
    let eve = Signer::<Sr25519>::from_suri("//Eve")?;
    let message = alice.encrypt(b"secret", &bob.public_key(), EncryptionAlgorithm::default())?;

    // SCALE bytes: what `Node::set_payload` stores on chain. They decode back.
    let stored = message.encode();
    let decoded = EncryptedMessage::decode(&mut stored.as_slice())?;
    println!(
        "stored {} SCALE bytes, decoded {:?}",
        stored.len(),
        kind(&decoded)
    );

    // JSON: a readable form of the same message, for logs and tooling.
    println!("as JSON: {}", serde_json::to_string(&decoded)?);

    // Wrong sender: the message claims Alice, the receiver expects Eve.
    let err = bob.decrypt(&decoded, Some(&eve.public_key())).unwrap_err();
    println!("wrong sender rejected: {err}");

    // Tampering with the ciphertext is detected by the AEAD tag.
    let EncryptedMessage::V1 {
        algorithm,
        from,
        nonce,
        mut ciphertext,
    } = decoded;
    ciphertext[0] ^= 1;
    let tampered = EncryptedMessage::V1 {
        algorithm,
        from,
        nonce,
        ciphertext,
    };
    let err = bob.decrypt(&tampered, None).unwrap_err();
    println!("tampered message rejected: {err}");

    // Only the intended receiver can decrypt.
    assert!(eve.decrypt(&message, None).is_err());
    println!("a third party cannot decrypt");
    Ok(())
}

/// Algorithm of a message, for printing.
fn kind(message: &EncryptedMessage) -> EncryptionAlgorithm {
    match message {
        EncryptedMessage::V1 { algorithm, .. } => *algorithm,
    }
}
