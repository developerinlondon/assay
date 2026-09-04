# Moving the engine store from SQLite to Postgres

`assay-engine migrate` copies a stopped SQLite store into an empty Postgres one. Use it when a
deployment outgrows a single volume: Postgres is what lets more than one engine serve the same
store, and this is the way across without starting over.

> **Give the engine its own database.** It owns the `engine`, `workflow`, `auth` and `vault`
> schemas, and its upgrade path moves tables named `workflows` and `namespaces` out of `public`. It
> will not touch tables it did not create — it checks provenance first and logs what it leaves alone
> — but a database shared with an application is a name collision waiting to be confusing.
> `CREATE DATABASE assay_engine` costs nothing.

## What is in the store

The SQLite backend is not one file. It is one database per module in the data directory, ATTACHed
into a router connection at boot:

```text
<data_dir>/engine.db     engine.modules, engine.migrations, engine.audit,
                         engine.instances, engine.events, engine.lock
<data_dir>/workflow.db   workflows, events, activities, timers, signals,
                         schedules, workers, snapshots, namespaces
<data_dir>/auth.db       users, sessions, passkeys, jwks_keys, biscuit_root_keys,
                         zanzibar_*, oidc_*, user_upstream, password_recovery_tokens
<data_dir>/vault.db      kv, kv_meta, transit_*, collections, items, folders, leases,
                         kek_metadata, unseal_shares, biscuit_root_keys, ...
```

Only the first two hold workflow state. The other two hold every credential the deployment has:
users and their password hashes, sessions, WebAuthn credentials, JWT signing keys, OIDC clients and
their secrets, the Zanzibar tuples that carry authorisation, and the vault's secrets.

**The vault's master key is a row, not an environment variable.** `vault.kek_metadata.sealed_blob`
holds the KEK itself, in the clear under the default `plaintext` sealing method. Migration copies
that row with everything else, which is what lets the ciphertext in `vault.kv` decrypt on the other
side. There is no key to carry separately, and equally: whoever can read the target database can
read the vault, exactly as before the move.

## The move

Run it with the engine stopped. The command reads the source directly, and a running engine holds
uncommitted state.

The same database name has to appear in every step below. Migrating into one database and then
pointing the engine at another leaves the engine on an empty store, which reads exactly like the
migration having lost everything.

```sh
# 0. An empty database of the engine's own. `migrate` creates the schemas
#    and tables inside it; there is no engine boot needed beforehand.
createdb assay_engine        # or: psql -c 'CREATE DATABASE assay_engine'

# 1. Stop every engine on this store.
kubectl scale deploy/assay-engine --replicas=0     # or: systemctl stop assay-engine

# 2. Copy the store off the node while it is stopped. This is the rollback.
kubectl cp assay-engine-0:/var/lib/assay/data ./assay-store-backup   # or: cp -a

# 3. See what will move. Writes nothing.
assay-engine migrate \
  --from sqlite:/var/lib/assay/data \
  --to   postgres://assay:$PASSWORD@db:5432/assay_engine \
  --dry-run

# 4. Do it.
assay-engine migrate \
  --from sqlite:/var/lib/assay/data \
  --to   postgres://assay:$PASSWORD@db:5432/assay_engine

# 5. Point the engine at the SAME database and start it.
#    engine.toml:
#      [backend]
#      type = "postgres"
#      url  = "postgres://assay:...@db:5432/assay_engine"
kubectl scale deploy/assay-engine --replicas=1
```

The vault's master key travels with the rest of the store, so migrated secrets decrypt on the other
side with nothing carried by hand. It also arrives in the clear, which is what it was on SQLite —
and a Postgres store gets dumped nightly where a volume did not. Set `ASSAY_VAULT_SEAL_KEY` on that
first Postgres boot and the key is encrypted at rest instead. See
[`docs/vault-sealing.md`](vault-sealing.md).

`--from` takes the data directory, as `sqlite:<dir>`, `sqlite://<dir>`, or a bare path — whichever
form the config or the volume gives you. `--to` takes the Postgres URL the engine will use.

The command prints a row count per table and a total:

```text
table                              copied
-----------------------------  ----------
auth.users                              4
...
workflow.workflows                     25
-----------------------------  ----------
total                                 138

skipped engine.lock (0 rows): no Postgres counterpart.

5 sequences re-pointed past the copied ids.
```

## Verify before you keep it

```sh
# The run you knew about, with its status and its history.
curl -sH "Authorization: Bearer $ADMIN_KEY" \
  https://engine.example.com/api/v1/engine/workflow/workflows/<id> | jq .
curl -sH "Authorization: Bearer $ADMIN_KEY" \
  https://engine.example.com/api/v1/engine/workflow/workflows/<id>/events | jq 'length'

# A secret you know the value of.
curl -sH "Authorization: Bearer $ADMIN_KEY" \
  https://engine.example.com/api/v1/vault/kv/<path> | jq -r .data

# Someone logs in.
```

The copy from step 2 is the rollback, and it has to be a copy held somewhere else. Putting
`[backend]` back to `type = "sqlite"` returns the engine to the store untouched, because migration
only reads — but only if the store still exists. **Do not rely on the volume.** A PVC with a
`Delete` reclaim policy, which is the default for local-path storage, is removed by the same sync
that switches the backend, and the four `.db` files go with it. Copy `engine.db`, `workflow.db`,
`auth.db` and `vault.db` off the node before you change anything, and keep them until the Postgres
store has served real traffic.

## What it guarantees, and what it refuses

- **Ids and timestamps are preserved.** A workflow keeps its id, its run id, its history sequence
  numbers and its float-seconds timestamps, so anything holding a reference to a run still resolves.
- **Sequences are re-pointed.** `workflow.events`, `workflow.activities`, `workflow.timers`,
  `workflow.signals` and `engine.events` have generated ids. After copying rows that already own ids
  1..N, each sequence is set past N, so the first write after the move does not collide.
- **The target must be empty.** A target holding engine rows is refused by name before anything is
  written. Merging two stores would have to reconcile ids that were only ever unique within one of
  them, and there is no correct answer to that.
- **A column the source has and the target does not is an error**, not a silent drop. The reverse is
  fine: a column added since the source was written takes its default.
- **Re-running against an empty target is safe.** Against a target it already filled, it refuses.
- **`engine.lock` is skipped**, and said so in the report. SQLite serialises single-instance access
  through that table; Postgres uses an advisory lock and has no counterpart. Any other source table
  without a Postgres counterpart is skipped the same way and reported with its row count.

## If the engine is already sharing a database

A database holding both an application's tables and an older engine's relocates **partly**, by
design: the marker-gated tables move, the ambiguously named ones stay where they are, and each one
left behind is named in a `WARN`. That is the safe outcome, not a finished one — the engine is
running in a database it half-owns, and the next release that adds a table can collide with a name
the application already uses.

Treat those warnings as work: migrate the engine into a database of its own. `migrate` reads the
SQLite store, so from a Postgres-on-Postgres position the move is a `pg_dump` of the four schemas
into a fresh database, then repointing `[backend] url` at it.

## Running more than one engine afterwards

That is the point of the move, and it works from a cold start: every Postgres schema migration takes
one transaction-scoped advisory lock, so engines that boot together on an empty database serialise
their DDL rather than racing it. Before that lock existed, `CREATE ... IF NOT EXISTS` could let two
engines both pass the existence check and one lose the catalog insert with
`duplicate key value violates unique constraint "pg_type_typname_nsp_index"`.
