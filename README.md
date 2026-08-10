[<img align="right" src="https://github.com/airalab/robonomics/blob/master/web3_foundation_grants_badge_black.jpg">](https://medium.com/web3foundation/web3-foundation-grants-wave-two-recipients-16d9b996501d)

# Robonomics Binaries 

[![License](https://img.shields.io/github/license/airalab/robonomics-bin)](https://github.com/airalab/robonomics-bin/blob/master/LICENSE)
[![Release](https://img.shields.io/github/release/airalab/robonomics-bin.svg)](https://github.com/airalab/robonomics-bin/releases)
[![Nightly](https://github.com/airalab/robonomics-bin/workflows/Nightly/badge.svg)](https://github.com/airalab/robonomics-bin/actions/workflows/nightly.yml)
[![Downloads](https://img.shields.io/github/downloads/airalab/robonomics-bin/total.svg)](https://github.com/airalab/robonomics-bin/releases)

> Robonomics node and tools build in Rust based on the [Polkadot SDK](https://polkadot.com/platform/sdk/). For more specific guides see the [Robonomics Wiki](https://wiki.robonomics.network).

Robonomics platform includes a set of open-source packages and infrastructure for Robotics, Smart Cities and Industry 4.0 developers.

## Repository Structure

This repository is organized as a Cargo workspace with the following structure:

### Node

- **`src`** - Robonomics Blockchain node 
  - Built using `polkadot-omni-node-lib` for maximum Polkadot compatibility

### Chain Specifications

- **`chains/`** - Chain specification files for different networks

### Tools

- **`tools/libcps/`** - Robonomics CPS (Cyber-Physical Systems) library and CLI
  - Comprehensive Rust library for managing hierarchical CPS nodes on-chain
  - Beautiful CLI interface with colored output and tree visualization
  - Multi-algorithm AEAD encryption support (XChaCha20-Poly1305, AES-256-GCM, ChaCha20-Poly1305)
  - MQTT bridge for IoT device integration
  - See [libcps/README.md](tools/libcps/README.md) for detailed documentation

- **`tools/robonet/`** - Local network spawner and integration test framework
  - CLI tool for spawning multi-node test networks using ZombieNet SDK
  - Built-in integration tests for XCM, CPS, and network functionality
  - Multiple network topologies (simple parachain, with AssetHub for XCM testing)
  - Developer-friendly interface with progress indicators and detailed logging
  - See [robonet/README.md](tools/robonet/README.md) for detailed documentation

### Development Infrastructure

- **`nix/`** - Nix flake modules and build configurations
- **`scripts/`** - Build, deployment, and testing scripts
  - `build-deb.sh` - Debian package builder
  - `build-static.sh` - Build statically linked binaries (for docker and portability)
  - `docker/` - Docker runner image and healthcheck scripts
  - `docker/builder` - Docker builder image 

## Contributing

We welcome contributions! Please see our [Contributing Guidelines](https://github.com/airalab/robonomics-bin/blob/master/CONTRIBUTING.md).

## Support

- **Wiki**: https://wiki.robonomics.network
- **Issues**: https://github.com/airalab/robonomics/issues
- **Website**: https://robonomics.network

## License

Robonomics is licensed under the Apache License 2.0. See [LICENSE](./LICENSE) for details.
