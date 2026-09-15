# Sealing the vault's master key

The vault encrypts every secret under a master key, the KEK, and the KEK lives in the engine's own
store in `vault.kek_metadata`. Something has to encrypt that row, or the key protecting the secrets
sits next to the secrets it protects: a database dump is then a plaintext copy of the vault, and so
is every backup of that dump.

So the engine needs unseal material, and **a vault-enabled engine that has none refuses to start.**
It names what to set:

```text
the vault module is enabled but no unseal material is configured, so its master key would be
written to vault.kek_metadata in the clear and a database dump would be a copy of every secret.
Set ASSAY_VAULT_SEAL_KEY to a string of at least 32 characters, or point [vault.sealing] at a
file, an inline value or a passphrase. For local development only, set
[vault.sealing] allow_plaintext_kek = true in engine.toml — there is deliberately no environment
variable for that.
```

## The quickest way

Set `ASSAY_VAULT_SEAL_KEY` to any string of at least 32 characters and change nothing else. That is
the default source, so `engine.toml` needs no `[vault.sealing]` section at all.

```sh
# Any of these are fine. Keep the value where the deployment keeps its
# other secrets; it is not recoverable and the vault cannot be read
# without exactly the same string.
openssl rand -base64 32
openssl rand -hex 32
```

```yaml
env:
  - name: ASSAY_VAULT_SEAL_KEY
    valueFrom:
      secretKeyRef: { name: assay-engine-seal, key: seal-key }
```

A value shorter than 32 characters is refused at boot:

```text
ASSAY_VAULT_SEAL_KEY must be at least 32 characters, got 12
```

## Where the key can come from

The environment is convenient and it is also the worst place to keep a secret on a Unix box: it is
readable from `/proc/<pid>/environ`, it is carried into core dumps, and it shows up in
`systemctl show -p Environment`. `[vault.sealing]` lets a deployment put the key somewhere else.

```toml
[vault.sealing]
source = "env" # the default
var = "ASSAY_VAULT_SEAL_KEY" # name your own if your injector has conventions
```

```toml
[vault.sealing]
source = "file"
path = "/run/secrets/vault-seal-key" # must be mode 0600
```

```toml
[vault.sealing]
source = "value"
value = "${VAULT_SEAL_KEY}" # ${VAR} is expanded before the TOML is parsed
```

```toml
[vault.sealing]
source = "passphrase"
passphrase = "${VAULT_PASSPHRASE}"
salt = "an-example-public-salt" # 16–64 characters, required, not secret
```

**The same string is the same key.** Every source except `passphrase` is hashed with SHA-256 over a
fixed label — exactly what the environment variable has always done — so an operator can move a
secret from the variable into a file, or inline, without re-sealing the store. In particular `value`
does **not** base64-decode what you give it, even though an inline key is usually base64: decoding
would derive a _different_ key from the same string, and moving `ASSAY_VAULT_SEAL_KEY` into `value`
would tell you your correct secret "does not decrypt".

`file` is checked before it is read. The engine `fstat`s the open handle rather than the path, so a
symlink swapped underneath it cannot win the race, and refuses a file group or other can read:

```text
the seal key file /run/secrets/vault-seal-key is mode 0644, which lets group or other read it;
chmod 600 it and restart
```

Trailing whitespace is trimmed everywhere, because a secret delivered as a file arrives with a
newline.

`${VAR}` is expanded before the TOML is parsed, so with `value` or `passphrase` the engine holds the
real secret in its config rather than a reference to one. Both fields are redacted wherever that
config is rendered — `GET /api/v1/engine/core/config` returns `[REDACTED]`, and so does any log line
that prints it. `env` and `file` keep the secret out of the config entirely, which is one fewer
place for it to be.

`passphrase` is the one source that is not a plain hash: it runs through Argon2id (19 MiB, 2 passes,
1 lane) because a passphrase a human can remember has far less entropy than 32 random bytes, and the
KDF is what makes guessing it expensive. It needs the `vault-sealing-passphrase` build feature,
which the default build has. Its `salt` is **required and public** — the derived key must survive a
restart, so the salt cannot be random per boot; its job is separation between deployments, not
secrecy. Lose the salt and you have lost the key.

## Turning it on for a store that already has secrets

A store written before sealing holds its KEK in the clear. The first boot with unseal material can
re-seal that row in place — but the rewrite is **one-way**. Afterwards the key exists only under
your material, and losing the material loses every secret the vault wraps, with no plaintext copy
left to fall back on. So it waits to be asked:

```text
vault KEK kid=kek-… is stored in the clear and unseal material is now configured, but re-sealing
it is one-way: afterwards the key exists only under that material, and losing the material loses
every secret the vault wraps, with no plaintext copy left to fall back on. Back up
vault.kek_metadata first, then set [vault.sealing] allow_plaintext_migration = true to let this
boot re-seal it.
```

Take the backup, then:

```toml
[vault.sealing]
allow_plaintext_migration = true
```

Boot once, confirm, and you can drop the flag again — a store that is already sealed never hits this
path, so leaving it set does nothing, and removing it means a future downgrade is caught.

```sh
curl -sH "Authorization: Bearer $ADMIN_KEY" \
  https://engine.example.com/api/v1/vault/sys/seal-status | jq .method
# "env-aes-gcm"
```

That method name describes the blob layout, not the source: a store sealed from a file or a
passphrase records `env-aes-gcm` too.

The re-seal logs what it cannot fix:

```text
WARN vault KEK was stored in plaintext and has been re-sealed;
     database backups taken before now still contain the unsealed key
```

**Re-sealing does not protect backups you already have.** Rotate the vault's contents, or destroy
the old dumps, if the plaintext copies matter.

## Local development

There is one way to boot without unseal material, and it is a config field:

```toml
[vault.sealing]
allow_plaintext_kek = true
```

Every boot that uses it logs at ERROR, because the deployment is running without the protection this
whole page is about:

```text
ERROR the vault master key is held in vault.kek_metadata in the clear, because
      [vault.sealing] allow_plaintext_kek is set. Anyone who can read a database dump, a backup
      or a replica can read every secret in the vault. This is a local development setting.
```

It has no environment variable, on purpose: a variable is the kind of thing that gets copied into a
deployment template and quietly turns sealing off in production. (`${VAR}` substitution is a textual
pass over the whole config file, so `allow_plaintext_kek = "${X}"` would still substitute — the
guarantee is that there is no shortcut of its own, not that the config loader is env-proof.)

It permits **minting** a key in the clear. It never opens a store that is already sealed — that
would mint a second KEK and orphan every secret the first one wraps, so it fails the same way a
missing key does.

## What it does and does not protect

The KEK is sealed with AES-256-GCM-SIV under the derived key. The stored blob is a version byte, a
12-byte nonce, and the encrypted key with its tag, 61 bytes in all. The key id is the additional
authenticated data, so a blob copied onto another row does not open.

The derivation is `SHA-256("assay-vault/env-seal/v1" || value)`. For everything but `passphrase` it
is deliberately not a slow KDF: the input is a machine-generated secret, not a human password being
defended against offline guessing, and the 32-character floor is what rules out a guessable one.

- **Protects**: database dumps, backups, replicas, anyone with read access to the store. They see
  ciphertext.
- **Does not protect**: the running process. The unsealed KEK is in memory while the engine serves,
  which is what lets it answer requests at all. Key material is scrubbed when it goes out of scope,
  which narrows that window but does not close it — the cipher keeps its own expanded round keys,
  and nothing here stops the process being paged to swap or written to a core dump.
- **Does not protect** anyone who has both the store and the material. Keep the seal key somewhere
  the database backups are not — which is the argument for `source = "file"` over the environment on
  a host where both end up in the same crash report.

## Losing the key

There is no recovery. The KEK cannot be reconstructed, and every secret in `vault.kv`, every transit
key and every collection item is unreadable without it. The engine refuses to start rather than mint
a fresh KEK and silently orphan them:

```text
vault KEK kid=kek-… is sealed with env-aes-gcm but no unseal material is configured —
ASSAY_VAULT_SEAL_KEY is not set and [vault.sealing] names no other source that produced a key.
Set it to the key this store was sealed with; [vault.sealing] allow_plaintext_kek does not open a
sealed store.
```

A wrong value fails the same way, naming the source it came from — and one character of difference
is a wrong value, since the whole string is hashed. Back the key up the way you back up anything
whose loss is unrecoverable.

## Turning it off

Removing the material does not unseal the store — the engine refuses to boot, because it cannot tell
"the operator turned it off" from "the key went missing". To go back to plaintext at rest, rotate
the KEK with the seal material absent.
