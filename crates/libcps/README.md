# 🌳 libcps - Robonomics CPS Library & CLI

A comprehensive Rust library and command-line interface for managing hierarchical Cyber-Physical Systems on the Robonomics blockchain.

## 📦 Packages

This crate provides two components:

### 1. **libcps** (Library)
A reusable library for building applications that interact with the Robonomics CPS pallet.

### 2. **cps** (CLI Binary)
A command-line interface for quick access to CPS pallet functionality.

## ✨ Features

- 🔐 **Multi-algorithm AEAD encryption** (XChaCha20-Poly1305, AES-256-GCM, ChaCha20-Poly1305)
- 🔑 **Dual keypair support** (SR25519 for Substrate, ED25519 for IoT/Home Assistant)
- 🌲 **Hierarchical tree visualization** of CPS nodes (CLI)
- ⚙️ **Flexible configuration** via environment variables or CLI args
- 🔒 **Secure by design** with proper key management and ECDH key agreement
- 📚 **Comprehensive documentation** for library API
- 🔧 **Type-safe blockchain integration** via subxt
- 🎛️ **Feature flags** for flexible dependency management

## 🏗️ Architecture

```
┌────────────────────────────────────────────────────┐
│                    libcps CLI                      │
├────────────────────────────────────────────────────┤
│      Commands │ Display │ Crypto │ Blockchain       │
└────────────────────────────────────────────────────┘
           ↓          ↓         ↓         ↓
┌────────────────────────────────────────────────────┐
│                 libcps Library                     │
├──────────────┬──────────────┬──────────────────────┤
│   Cipher     │  Types       │  Generated Runtime   │
│   - SR25519  │  - BoundedVec│  - subxt codegen     │
│   - ED25519  │  - NodeId    │  - CPS pallet API    │
└──────────────┴──────────────┴──────────────────────┘
      ↓
┌─────────────────────┐
│  Substrate Node     │
│  - CPS Pallet       │
└─────────────────────┘
```

## 📦 Installation

### As a Library

Add to your `Cargo.toml`:

```toml
[dependencies]
libcps = "0.6.0"
```

#### Feature Flags

The library supports optional feature flags for flexible dependency management:

- **`cli`** - Enables the `cps` CLI binary with colored output and progress bars (enabled by default)

```toml
# Default: CLI feature enabled
[dependencies]
libcps = "0.6.0"

# Library only, without CLI dependencies
[dependencies]
libcps = { version = "0.6.0", default-features = false }
```

### CLI Tool from Crates.io

```bash
cargo install libcps
```

### Run CLI using Nix caches

```bash
nix run github:airalab/robonomics#libcps
```

### From Source

```bash
# Clone the repository
git clone https://github.com/airalab/robins
cd robonomics

# Build the library
cargo build --release --package libcps --lib

# Build the CLI tool
cargo build --release --package libcps --bin cps

# The binary will be at: target/release/cps
```

### Add CLI to PATH (optional)

```bash
sudo cp target/release/cps /usr/local/bin/
```

## 🚀 CLI Quick Start

### 1. Set up your environment

```bash
# Set blockchain endpoint
export ROBONOMICS_WS_URL=ws://localhost:9944

# Set your account (development account for testing)
export ROBONOMICS_SURI=//Alice
```

### 2. Create your first node

```bash
# Create a root node
cps create --meta '{"type":"building","name":"HQ"}' --payload '{"status":"online"}'

# Create a child node
cps create --parent 0 --meta '{"type":"room","name":"Server Room"}' --payload '{"temp":"22C"}'
```

### 3. View your CPS tree

```bash
cps show 0
```

Output:
```
[*] CPS Node ID: 0

|--  [O] Owner: 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY
|--  [M] Meta: {
  "type": "building",
  "name": "HQ"
}
`--  [P] Payload: {
  "status": "online"
}
```

## 📖 Commands

### `show <node_id>`

Display node information and its children in a beautiful tree format.

```bash
# Show node 0
cps show 0

# Show node with decryption attempt
cps show 5 --decrypt
```

### `create`

Create a new node (root or child).

```bash
# Create root node
cps create --meta '{"type":"sensor"}' --payload '22.5C'

# Create child node
cps create --parent 0 --payload 'operational data'

# Create with encryption (SR25519, default)
cps create --parent 0 --payload 'secret data' --receiver-public <RECEIVER_ADDRESS>

# Create with ED25519 encryption
cps create --parent 0 --payload 'secret data' --receiver-public <RECEIVER_ADDRESS> --scheme ed25519

# Create with specific cipher
cps create --parent 0 --payload 'secret data' --receiver-public <RECEIVER_ADDRESS> --cipher aesgcm256
```

**Options:**
- `--parent <id>`: Parent node ID (omit for root node)
- `--meta <data>`: Metadata (configuration data)
- `--payload <data>`: Payload (operational data)
- `--receiver-public <address>`: Receiver public key or SS58 address for encryption (required to encrypt data)
- `--cipher <algorithm>`: Encryption algorithm (xchacha20, aesgcm256, chacha20) [default: xchacha20]
- `--scheme <type>`: Cryptographic scheme (sr25519, ed25519) [default: sr25519]

### `set-meta <node_id> <data>`

Update node metadata.

```bash
# Update metadata
cps set-meta 5 '{"name":"Updated Sensor"}'

# Update with encryption
cps set-meta 5 'private config' --receiver-public <RECEIVER_ADDRESS>

# Update with ED25519 encryption
cps set-meta 5 'private config' --receiver-public <RECEIVER_ADDRESS> --scheme ed25519
```

### `set-payload <node_id> <data>`

Update node payload (operational data).

```bash
# Update temperature reading
cps set-payload 5 '23.1C'

# Update with encryption
cps set-payload 5 'encrypted telemetry' --receiver-public <RECEIVER_ADDRESS>

# Update with ED25519 and AES-GCM
cps set-payload 5 'encrypted telemetry' --receiver-public <RECEIVER_ADDRESS> --scheme ed25519 --cipher aesgcm256
```

### `move <node_id> <new_parent_id>`

Move a node to a new parent.

```bash
# Move node 5 under node 3
cps move 5 3
```

**Features:**
- Automatic cycle detection (prevents moving a node under its own descendant)
- Path validation

### `remove <node_id>`

Delete a node (must have no children).

```bash
# Remove node with confirmation
cps remove 5

# Remove without confirmation
cps remove 5 --force
```

## ⚙️ Configuration

### Environment Variables

```bash
# Blockchain connection
export ROBONOMICS_WS_URL=ws://localhost:9944

# Account credentials
export ROBONOMICS_SURI=//Alice
# Or use a seed phrase:
# export ROBONOMICS_SURI="your twelve word seed phrase here goes like this"
```

### CLI Arguments (override environment variables)

```bash
cps --ws-url ws://localhost:9944 \
    --suri //Alice \
    show 0
```

## 📚 Library Usage

### Quick Start

This example shows the core node-oriented operations: creating nodes, setting metadata and payload, and visualizing the tree structure.

```rust
use libcps::blockchain::{Client, Config, BoundedVec};
use libcps::node::Node;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Connect to blockchain
    let config = Config {
        ws_url: "ws://localhost:9944".to_string(),
        suri: Some("//Alice".to_string()),
    };
    let client = Client::new(&config).await?;
    
    // Create a root node with metadata and payload
    let meta = BoundedVec(r#"{"type":"building","name":"HQ"}"#.as_bytes().to_vec());
    let payload = BoundedVec(r#"{"status":"online"}"#.as_bytes().to_vec());
    let root_node = Node::create(&client, None, Some(meta), Some(payload)).await?;
    println!("Created root node: {}", root_node.id());
    
    // Create a child node
    let child_meta = BoundedVec(r#"{"type":"room","name":"Server Room"}"#.as_bytes().to_vec());
    let child_payload = BoundedVec(r#"{"temp":"22C"}"#.as_bytes().to_vec());
    let child_node = Node::create(&client, Some(root_node.id()), Some(child_meta), Some(child_payload)).await?;
    println!("Created child node: {}", child_node.id());
    
    // Update node metadata
    let new_meta = BoundedVec(r#"{"type":"room","name":"Server Room","updated":true}"#.as_bytes().to_vec());
    child_node.set_meta(Some(new_meta)).await?;
    
    // Update node payload
    let new_payload = BoundedVec(r#"{"temp":"23.5C"}"#.as_bytes().to_vec());
    child_node.set_payload(Some(new_payload)).await?;
    
    // Query and display node information
    let info = root_node.query().await?;
    println!("Node {} has {} children", info.id, info.children.len());
    
    Ok(())
}
```

### Data Types

```rust
use libcps::blockchain::BoundedVec;
use libcps::node::NodeId;
use libcps::crypto::EncryptionAlgorithm;

// Create plain data (unencrypted)
let meta = BoundedVec("sensor config".as_bytes().to_vec());
let meta_bytes = BoundedVec(vec![1, 2, 3]);

// Create encrypted data from cipher output
let encrypted_msg = cipher.encrypt(plaintext, &receiver_public, EncryptionAlgorithm::XChaCha20Poly1305)?;
let encrypted_bytes = encrypted_msg.encode();
let payload = BoundedVec(encrypted_bytes);
```

## 🔐 Encryption

The library supports multiple cryptographic schemes and AEAD encryption algorithms with robust key derivation and self-describing message format.

### Cryptographic Schemes

Two cryptographic schemes are supported for ECDH key agreement:

| Feature | SR25519 | ED25519 |
|---------|---------|---------|
| **Curve** | Ristretto255 | Curve25519 (via X25519) |
| **ECDH** | Ristretto255 scalar multiplication | ED25519 → X25519 |
| **Best For** | Substrate blockchain operations | IoT devices, Home Assistant |
| **Compatibility** | Native to Polkadot ecosystem | Standard ED25519 implementations |
| **Key Agreement** | `scalar * point` on Ristretto255 | ED25519 → Curve25519 → X25519 |

#### **SR25519** (Default - Substrate Native)

- Uses Ristretto255 curve for ECDH
- Native to Substrate/Polkadot ecosystem
- Best for: Substrate blockchain operations
- Key agreement: Ristretto255 scalar multiplication

#### **ED25519** (IoT Compatible)

- Uses X25519 ECDH (ED25519 → Curve25519 conversion)
- Compatible with standard ED25519 implementations
- Best for: IoT devices, Home Assistant integration, standard cryptography
- Key agreement: ED25519 → Curve25519 → X25519

### Encryption Algorithms

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

### How it works

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


### Key Derivation (HKDF-SHA256)

The encryption scheme uses HKDF (RFC 5869) for deriving encryption keys from shared secrets:

#### Process:

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
     - "robonomics-cps-xchacha20poly1305"
     - "robonomics-cps-aesgcm256"
     - "robonomics-cps-chacha20poly1305"
   
   OKM = HMAC-SHA256(PRK, info)[0..32]
   ```

#### Security Properties:

- **Domain Separation**: Keys bound to Robonomics network context
- **Algorithm Binding**: Different algorithms produce independent keys
- **Key Independence**: Each (shared_secret, algorithm) pair → unique key
- **Security Enhancement**: Constant salt strengthens key derivation even with low-entropy secrets

## 🎯 Use Cases

### 1. IoT Sensor Network

```bash
# Create building structure
cps create --meta '{"type":"building"}'
cps create --parent 0 --meta '{"type":"floor","number":1}'
cps create --parent 1 --meta '{"type":"room","name":"Server Room"}'

# Bridge sensor data with the mqtt-bridge crate
mqtt-bridge subscribe "sensors/room1/temp" 2
mqtt-bridge subscribe "sensors/room1/humidity" 2
```

### 2. Smart Home Automation

```bash
# Create home hierarchy
cps create --meta '{"type":"home"}'
cps create --parent 0 --meta '{"type":"room","name":"Kitchen"}'
cps create --parent 1 --meta '{"type":"device","name":"Smart Light"}'

# Control devices with the mqtt-bridge crate
mqtt-bridge publish "devices/kitchen/light/state" 2
```

### 3. Industrial Monitoring

```bash
# Create factory structure
cps create --meta '{"type":"factory"}'
cps create --parent 0 --meta '{"type":"line","name":"Assembly Line 1"}'
cps create --parent 1 --meta '{"type":"machine","id":"CNC-001"}'

# Monitor machine data with encryption via the mqtt-bridge crate
mqtt-bridge subscribe "machines/cnc001/telemetry" 2 --receiver-public <RECEIVER_ADDRESS>
```

## 🛠️ Development

### Project Structure

```
tools/libcps/
├── Cargo.toml            # Dependencies and features (uses robonomics-runtime-subxt-api)
├── README.md             # This file
├── DEVELOPMENT.md        # Developer guide
└── src/
    ├── lib.rs            # Library entry point with module exports
    ├── main.rs           # CLI entry point
    ├── node.rs           # Node-oriented API with CPS type definitions
    ├── blockchain/       # Blockchain client and connection
    │   ├── mod.rs
    │   └── client.rs
    ├── commands/         # CLI command implementations
    │   ├── mod.rs
    │   ├── show.rs
    │   ├── create.rs
    │   ├── set_meta.rs
    │   ├── set_payload.rs
    │   ├── move_node.rs
    │   └── remove.rs
    ├── crypto/           # Encryption utilities
    │   ├── mod.rs        # Documentation and re-exports
    │   ├── types.rs      # CryptoScheme, EncryptionAlgorithm, EncryptedMessage
    │   └── cipher.rs     # Cipher implementation
    └── display/          # Pretty CLI output
        ├── mod.rs
        └── tree.rs
```

### Building

```bash
cargo build --package libcps
```

### Testing

```bash
cargo test --package libcps
```

### Generating Blockchain Types

Type-safe blockchain interactions are **automatically generated** using the 
`robonomics-runtime-subxt-api` crate. This crate:

1. Extracts metadata from the robonomics runtime at build time
2. Saves metadata to `$OUT_DIR/metadata.scale`  
3. subxt macro reads the metadata and generates type-safe APIs at compile time

**No external tools required!** Just build the project:

```bash
cargo build -p libcps
```

The generated types are always in sync with the runtime dependency version.
For more details, see the [subxt-api documentation](../../runtime/robonomics/subxt-api/README.md).

## 🤝 Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## 📄 License

Apache-2.0

## 🔗 Links

- [Robonomics Network](https://robonomics.network)
- [Documentation](https://wiki.robonomics.network)
- [GitHub](https://github.com/airalab/robonomics)

## 💡 Tips

- Use `//Alice`, `//Bob`, etc. for development accounts
- Always backup your seed phrase in production
- Test encryption with development keys first
- Use `--help` on any command for more details

## 🐛 Troubleshooting

### Connection Failed

```bash
# Check if node is running
curl -H "Content-Type: application/json" -d '{"id":1, "jsonrpc":"2.0", "method": "system_health"}' http://localhost:9944

# Try default WebSocket URL
cps --ws-url ws://127.0.0.1:9944 show 0
```

### Account Not Found

```bash
# Make sure SURI is set
export ROBONOMICS_SURI=//Alice

# Or pass it directly
cps --suri //Alice create --meta '{"test":true}'
```

---

Made with ❤️ by the Robonomics Team
