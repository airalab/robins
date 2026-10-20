# Installation

The `cps` binary is the command-line face of `libcps`. Pick whichever way suits you.

## From crates.io

```bash
cargo install libcps
```

## With Nix

Run it without installing, straight from the flake:

```bash
nix run github:airalab/robins#libcps -- --help
```

The flake's binary caches (`robonomics.cachix.org`) are declared in the repository's `flake.nix`.

## From source

```bash
git clone https://github.com/airalab/robins
cd robins
nix develop                       # optional, provides the toolchain
cargo build --release -p libcps --bin cps
# the binary is target/release/cps
sudo cp target/release/cps /usr/local/bin/   # optional
```

Check the installation:

```bash
cps --version
cps --help
```

Every command accepts `--help`; the pages of this part only show what you need for the story.

## Library only

If you only build applications, depend on the crate without the CLI dependencies:

```toml
[dependencies]
libcps = { version = "0.8", default-features = false }
# embedded light client:
# libcps = { version = "0.8", default-features = false, features = ["light-client"] }
```

| Feature | Default | Provides |
| --- | --- | --- |
| `cli` | yes | the `cps` binary (includes `light-client`) |
| `light-client` | via `cli` | embedded smoldot light node, `libcps::light` |
