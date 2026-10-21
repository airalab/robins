# Installation

The `cps` binary is the command-line face of `libcps`. Pick whichever way suits you.

## Nightly release (recommended)

Prebuilt binaries are published as pre-releases on the
[Releases page](https://github.com/airalab/robins/releases) under tags named
`nightly-<YYMM>.<N>`. Every archive contains the `cps`, `mqtt-bridge` and `edge` binaries for
Linux (static, `x86_64`/`aarch64`) or macOS (`aarch64`/`x86_64`).

```bash
# pick one of: linux-x86_64, linux-aarch64, macos-aarch64, macos-x86_64
PLATFORM=linux-x86_64
# the newest nightly tag, e.g. nightly-2610.1
TAG=$(gh release list --repo airalab/robins --limit 1 --json tagName --jq '.[0].tagName')

curl -fsSL https://github.com/airalab/robins/releases/download/$TAG/robins-$TAG-$PLATFORM.tar.gz | tar -xz
sudo install -m 755 cps /usr/local/bin/
```

Without the `gh` CLI, copy the tag from the Releases page into `TAG`. To verify a download, fetch
`SHA256SUMS` from the same release and run `sha256sum --ignore-missing -c SHA256SUMS`
(macOS: `shasum -a 256 --ignore-missing -c SHA256SUMS`).

> Nightly builds are not audited and may change without notice.

## From crates.io

```bash
cargo install libcps
```

## With Nix

Run it without installing, straight from the flake:

```bash
nix run github:airalab/robins#libcps -- --help
# pin to a specific nightly
nix run github:airalab/robins/nightly-2610.1#libcps -- --help
```

The flake's binary caches (`robonomics.cachix.org`) are declared in the repository's `flake.nix`.


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
