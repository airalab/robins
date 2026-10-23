# Encrypt your data

> **Example:** `08_encryption` &nbsp;·&nbsp; **Needs:** nothing (works offline) &nbsp;·&nbsp; **API:** `crypto::{Signer, Sr25519, Ed25519, Cipher, EncryptedMessage, EncryptionAlgorithm}`

Everything on chain is public, including what you write. When a payload must stay private, encrypt
it for the receiver's public key *before* it is stored. Anyone can still see that a message exists,
who sent it and its size, but only the receiver can read it.

```bash
cargo run -p libcps --example 08_encryption --no-default-features
```

```rust,ignore
{{#include ../../../crates/libcps/examples/08_encryption.rs:code}}
```

The example shows, in order:

1. **Signers** for both supported key schemes, **SR25519** (Substrate native) and **ED25519** (IoT
   devices, Home Assistant), and their on-chain account ids.
2. **Encryption for a receiver** with each AEAD algorithm: XChaCha20-Poly1305 (default),
   AES-256-GCM and ChaCha20-Poly1305. Sender and receiver derive the same key from an ECDH key
   agreement; no secret is ever transmitted.
3. **The encoding.** An `EncryptedMessage` is self-describing (algorithm, sender public key, nonce,
   ciphertext); its SCALE bytes are exactly what you store as a node payload.
4. **Verification.** Decrypting with the wrong expected sender, or after tampering with a single
   byte, fails.

To store a message, pass its SCALE bytes to `set_payload` like any other value. To read it, fetch the
payload, decode the `EncryptedMessage` and call `receiver.decrypt(&message, Some(&sender_public))`.
Details of the schemes, key derivation and message format are in
[Encryption internals](../reference/encryption-internals.md).

**CLI twin:** [`--receiver-public` and `--decrypt`](../01-cli/07-encryption.md).

## End of Part II

You have explored the live network, watched it change, launched your own chain, built a tree, given
it an identity, shared write access and protected data, all from Rust. For the operator's view of
the same story see [Part I: CLI](../01-cli/01-installation.md); for conveniences see
[Appendix A](../appendix/profiles.md).
