# C. Encryption internals

This chapter documents how `libcps` encrypts data. It applies to the library (`libcps::crypto`) and to
the CLI (`--receiver-public`, `--decrypt`).

## Cryptographic Schemes

Two cryptographic schemes are supported for ECDH key agreement:

| Feature | SR25519 | ED25519 |
|---------|---------|---------|
| **Curve** | Ristretto255 | Curve25519 (via X25519) |
| **ECDH** | Ristretto255 scalar multiplication | ED25519 → X25519 |
| **Best For** | Substrate blockchain operations | IoT devices, Home Assistant |
| **Compatibility** | Native to Polkadot ecosystem | Standard ED25519 implementations |
| **Key Agreement** | `scalar * point` on Ristretto255 | ED25519 → Curve25519 → X25519 |

### **SR25519** (Default - Substrate Native)

- Uses Ristretto255 curve for ECDH
- Native to Substrate/Polkadot ecosystem
- Best for: Substrate blockchain operations
- Key agreement: Ristretto255 scalar multiplication

### **ED25519** (IoT Compatible)

- Uses X25519 ECDH (ED25519 → Curve25519 conversion)
- Compatible with standard ED25519 implementations
- Best for: IoT devices, Home Assistant integration, standard cryptography
- Key agreement: ED25519 → Curve25519 → X25519

## Encryption Algorithms

Three AEAD ciphers are supported:

1. **XChaCha20-Poly1305** (Default)
   - 24-byte nonce (collision-resistant)
   - ~680 MB/s software performance
   - Best for: General purpose, portable

2. **AES-256-GCM**
   - 12-byte nonce
   - ~2-3 GB/s with AES-NI hardware acceleration
   - Best for: High throughput with hardware support

3. **ChaCha20-Poly1305**
   - 12-byte nonce
   - ~600 MB/s software performance
   - Best for: Portable performance without hardware acceleration

## How it works

1. **Key Derivation (ECDH + HKDF)**
   - For SR25519: Derive shared secret using Ristretto255 ECDH
   - For ED25519: Derive shared secret using X25519 ECDH
   - Apply HKDF-SHA256 with algorithm-specific info string

2. **Encryption (AEAD)**
   - Encrypt data with derived 32-byte key
   - Generate random nonce per message (size varies by algorithm)
   - Add authentication tag (AEAD)

3. **Self-Describing Message Format**
   
   The encrypted message uses SCALE codec for efficient binary serialization on the blockchain.
   The message format is defined as a versioned Rust enum:
   
   ```rust
   pub enum EncryptedMessage {
       V1 {
           algorithm: EncryptionAlgorithm,  // XChaCha20Poly1305, AesGcm256, or ChaCha20Poly1305
           from: [u8; 32],                  // Sender's 32-byte public key
           nonce: Vec<u8>,                  // 24 bytes for XChaCha20, 12 for AES-GCM/ChaCha20
           ciphertext: Vec<u8>,             // Encrypted data with authentication tag
       }
   }
   ```
   
   The message is serialized using **SCALE codec** (Simple Concatenated Aggregate Little-Endian),
   the native encoding format for Substrate blockchains, providing:
   
   - **Blockchain efficiency**: Compact binary format minimizes on-chain storage costs
   - **Automatic algorithm detection**: Receiver knows which cipher to use from the enum
   - **Sender identification**: The `from` field contains sender's raw 32-byte public key
   - **Version compatibility**: Enum variants enable future protocol upgrades
   - **Type safety**: Compile-time guarantee of message structure validity with Encode/Decode derives
   - **Future-proof**: New versions can be added as additional enum variants (e.g., `V2 { ... }`)
   - **Native integration**: SCALE codec is the standard for all Substrate/Polkadot data


## Key Derivation (HKDF-SHA256)

The encryption scheme uses HKDF (RFC 5869) for deriving encryption keys from shared secrets:

### Process:

1. **ECDH Key Agreement**
   - SR25519: Ristretto255 scalar multiplication
   - ED25519: X25519 (ED25519 → Curve25519 → X25519)
   - Result: 32-byte shared secret

2. **HKDF Extract**
   ```
   salt = "robonomics-network"  (constant, for domain separation)
   PRK = HMAC-SHA256(salt, shared_secret)
   ```

3. **HKDF Expand**
   ```
   info = algorithm-specific string:
     - "xchacha20poly1305"
     - "aesgcm256"
     - "chacha20poly1305"
   
   OKM = HMAC-SHA256(PRK, info || 0x01)[0..32]
   ```

### Security Properties:

- **Domain Separation**: Keys bound to Robonomics network context
- **Algorithm Binding**: Different algorithms produce independent keys
- **Key Independence**: Each (shared_secret, algorithm) pair → unique key
- **Security Enhancement**: Constant salt strengthens key derivation even with low-entropy secrets

