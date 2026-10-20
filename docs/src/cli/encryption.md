# Encryption

> **Library twin:** [Encrypt your data](../library/08-encryption.md)

Anything written to the chain is public. Encrypt data for a receiver's public key when writing, and
decrypt with the receiver's key when reading. A key agreement (ECDH) derives the shared key, so no
secret needs to be exchanged.

| Option | Applies to | Meaning |
| --- | --- | --- |
| `-r, --receiver-public <ACCOUNT>` | `create`, `meta set`, `payload set` | encrypt for this receiver (SS58 or hex public key); needs `--suri` |
| `--cipher <NAME>` | same | `xchacha20` (default), `aesgcm256` or `chacha20` |
| `--scheme <sr25519\|ed25519>` | same, and reads | scheme of the `--suri` keypair used for the key agreement |
| `-d, --decrypt` | `show`, `meta get`, `payload get` | decrypt with the `--suri` keypair |

```bash
RECEIVER=$(cps identity inspect //Bob --output json | sed -E 's/.*"address":"([^"]+)".*/\1/')

# Alice writes a secret for Bob
cps --suri //Alice payload set 1 'secret reading' --receiver-public "$RECEIVER"

# Anyone sees only ciphertext
cps payload get 1 --hex

# Bob reads it
cps --suri //Bob payload get 1 --decrypt
```

Choose the scheme to match the key type: SR25519 is Substrate-native and the default; ED25519 suits IoT
devices and Home Assistant.

```bash
cps --suri //Alice create \
    --payload 'home assistant data' \
    --receiver-public "$RECEIVER" \
    --cipher aesgcm256 \
    --scheme ed25519
```

Data encrypted for you can be read with `--decrypt` only with the key it was encrypted for; reading
with another key fails instead of printing garbage. Algorithms, key derivation and the message format
are described in [Encryption internals](../reference/encryption-internals.md).
