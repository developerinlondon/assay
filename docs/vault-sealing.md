# Sealing the vault's master key

The vault encrypts every secret under a master key, the KEK, and the KEK lives in the engine's own
store in `vault.kek_metadata`. By default it is stored as raw bytes: the row that protects the
secrets sits next to the secrets it protects. A database dump is then a plaintext copy of the vault,
and so is every backup of that dump.

Set `ASSAY_VAULT_SEAL_KEY` and the KEK is encrypted at rest instead. The dump carries ciphertext,
and the key to read it lives wherever the deployment already keeps its environment.

## Turning it on

`ASSAY_VAULT_SEAL_KEY` is any string of at least 32 characters. The engine derives the 32-byte
AES-256 key from it with SHA-256 over a fixed label, so the encoding does not matter: base64, hex
and a passphrase all work, and a value that happens to be valid base64 is not treated as one.
Surrounding whitespace is trimmed, because a secret delivered as a file arrives with a newline.

```sh
# Any of these are fine. Keep the value where the deployment keeps its
# other secrets; it is not recoverable and the vault cannot be read
# without exactly the same string.
openssl rand -base64 32
openssl rand -hex 32
```

A value shorter than 32 characters is refused at boot:

```text
ASSAY_VAULT_SEAL_KEY must be at least 32 characters, got 12
```

Give it to the engine as `ASSAY_VAULT_SEAL_KEY` and restart. Nothing else changes:

```yaml
env:
  - name: ASSAY_VAULT_SEAL_KEY
    valueFrom:
      secretKeyRef: { name: assay-engine-seal, key: seal-key }
```

A store already holding a plaintext KEK is re-sealed on that first boot, in place, and logs:

```text
WARN vault KEK was stored in plaintext and has been re-sealed with ASSAY_VAULT_SEAL_KEY;
     database backups taken before now still contain the unsealed key
```

That last clause is the part to act on. **Re-sealing does not protect backups you already have.**
Rotate the vault's contents, or destroy the old dumps, if the plaintext copies matter.

Re-running with the same key is a no-op. Confirm it took:

```sh
curl -sH "Authorization: Bearer $ADMIN_KEY" \
  https://engine.example.com/api/v1/vault/sys/seal-status | jq .method
# "env-aes-gcm"
```

## What it does and does not protect

The KEK is sealed with AES-256-GCM-SIV under the derived key. The stored blob is a version byte, a
12-byte nonce, and the encrypted key with its tag, 61 bytes in all. The key id is the additional
authenticated data, so a blob copied onto another row does not open.

The derivation is `SHA-256("assay-vault/env-seal/v1" || value)`. It is deliberately not a slow KDF:
the input is a machine-generated secret held in the environment, not a human password being defended
against offline guessing, and the 32-character floor is what rules out a guessable one.

- **Protects**: database dumps, backups, replicas, anyone with read access to the store. They see
  ciphertext.
- **Does not protect**: the running process. The unsealed KEK is in memory while the engine serves,
  which is what lets it answer requests at all.
- **Does not protect** anyone who has both the dump and the environment. Keep the seal key somewhere
  the database backups are not.

## Losing the key

There is no recovery. The KEK cannot be reconstructed, and every secret in `vault.kv`, every transit
key and every collection item is unreadable without it. The engine refuses to start rather than mint
a fresh KEK and silently orphan them:

```text
vault KEK kid=kek-… is sealed with env-aes-gcm but ASSAY_VAULT_SEAL_KEY is not set;
set it to the key this store was sealed with
```

A wrong value fails the same way, naming the variable — and one character of difference is a wrong
value, since the whole string is hashed. Back the key up the way you back up anything whose loss is
unrecoverable.

## Turning it off

Unsetting the variable does not unseal the store — the engine will refuse to boot, because it cannot
tell "the operator turned it off" from "the key went missing". To go back to plaintext at rest,
rotate the KEK with the seal key absent.

## Without the variable

Behaviour is unchanged from before this existed, and the engine says so on first boot:

```text
WARN first-boot plaintext KEK persisted. Phase 1 placeholder
```
