# Migrating to assay-vault 0.5.0 — KEK sealed at rest (#113)

**Breaking change for anyone running the vault module.** Before 0.5.0 the
vault's master key-encryption-key (KEK) was persisted in **plaintext** in
`vault.kek_metadata.sealed_blob` and auto-seeded on first boot. A single DB
read decrypted every secret. 0.5.0 closes that: the KEK is **sealed at rest**
under operator-supplied unseal material, and the engine **fails closed** rather
than boot with an unsealed KEK.

Read this if `engine.modules.vault` is enabled (it is by default when the
`vault` Cargo feature is compiled in).

## What changed

- `vault.kek_metadata` rows now use `sealing_method = 'sealed-v1'`. The
  `sealed_blob` is the KEK encrypted with AES-256-GCM-SIV under a wrapping key
  derived from your unseal material. Format:
  `version(1) | kdf(1) | salt_len(1) | salt | nonce(12) | ciphertext(48)`.
- Boot **requires** unseal material. No silent plaintext fallback.
- The runtime `seal-status` now reports `method = "sealed-v1"` (was
  `"plaintext"`). The vault is still unsealed-and-operational at boot — the KEK
  is unsealed into memory; it is NOT gated on an unseal ceremony like Shamir.

## What you must do

Provide unseal material via the new `[vault]` config section. Pick one:

```toml
[vault]
# Recommended: a base64-encoded 32-byte key from an env var.
unseal_key_source = "env:ASSAY_VAULT_UNSEAL_KEY"
```

```sh
export ASSAY_VAULT_UNSEAL_KEY=$(openssl rand -base64 32)
```

Other sources:

| `unseal_key_source`              | Meaning                                                |
| -------------------------------- | ------------------------------------------------------ |
| `env:NAME`                       | base64 32-byte key (or `passphrase:<text>`) from `$NAME` |
| `file:/path`                     | same, read from a file that **must be `0600`**         |
| `base64:BBBB`                    | inline base64 raw key (dev/test)                       |
| `passphrase:TEXT`                | inline passphrase (Argon2id m=64MiB/t=3/p=4)           |

**Store the unseal key safely and durably.** Losing it means losing the KEK,
which means every wrapped secret is unrecoverable.

## Migrating an existing plaintext KEK

If `vault.kek_metadata` already holds a `plaintext` KEK from a pre-0.5.0
deployment:

1. **Back up the database first.** The migration is one-way and irreversible —
   the plaintext blob is overwritten with the sealed blob and cannot be undone.
2. Set `unseal_key_source` and export the key (as above).
3. Set `allow_plaintext_migration = true` to explicitly authorise the one-way
   rewrite:

```toml
[vault]
unseal_key_source = "env:ASSAY_VAULT_UNSEAL_KEY"
allow_plaintext_migration = true   # remove after migration completes
```

4. Reboot. The engine **re-seals the existing KEK in place**: the row flips to
   `sealing_method = 'sealed-v1'`, the plaintext blob is overwritten with the
   sealed blob, and a `MIGRATED ...` warning is logged. Your existing wrapped
   DEKs keep working — the KEK identity (`kid`) is preserved.
5. **Remove `allow_plaintext_migration = true`** from `engine.toml` once you
   have confirmed the migration logged successfully. The flag is not needed
   after a fresh `sealed-v1` row exists.

If you reboot with `unseal_key_source` set but **without** `allow_plaintext_migration`,
the engine refuses to boot (fail-closed) and prints instructions. No data is
touched.

If you reboot **without** `unseal_key_source` at all, the engine also refuses to
boot (fail-closed). No data is touched.

## Dev / demo escape hatch

For local demos where sealing is overkill, you can keep the old plaintext
behavior explicitly:

```toml
[vault]
dev_plaintext_kek = true
```

This logs a **CRITICAL** warning on every boot and persists the KEK in
plaintext. Never use it for real secrets.

## Future-proofing

The sealed blob carries a version tag (`sealed-v1`). The planned Shamir / cloud
KMS / HSM phases ship as new `sealing_method` values + new blob versions; an old
binary refuses a row it can't interpret instead of mis-parsing it.
