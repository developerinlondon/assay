# Changelog

All notable changes to Assay are documented here.

## assay-lua 0.20.5 — 2026-09-04

### Added

- **`assay.salesforge` can connect a mailbox and switch its warm-up on.** The module could read
  what a workspace holds and what its warm-up is doing, and change neither. Wiring a fleet into
  the sequencer therefore stayed a thing done by hand in the web app, seventeen boxes at a time.

  `c:connect_smtp(address, password, opts)` posts the transport blocks the public API asks for —
  one password carried into both, the address as the username on each, `smtp.gmail.com:587` and
  `imap.gmail.com:993` unless `opts.smtp`/`opts.imap` say otherwise. The vendor verifies the
  credentials afterwards, so what comes back is `pending` and `connected` is true only where the
  vendor already said `active`: a caller that needs the verdict reads the box again rather than
  being told a verification that has not happened yet succeeded. A 2xx carrying the vendor's own
  refusal in the body — "failed to verify mailbox credentials" — is a typed `refused` error and
  never a mailbox, because read as one it is a box nothing can send from, reported as connected.

  `c:set_warmup(id_or_address, on)` sets the switch on the web app's own API and then **reads the
  box back**. That is the point of it: a box created through the public API arrives with warm-up
  off despite the vendor documenting that a connected box warms automatically, and a PUT that
  answers 200 while the flag stays false is the failure this exists to catch. What comes back is
  what the vendor now holds, not what it was asked for. An address is resolved to the vendor's id
  on the internal listing, so an operator passes the address they actually have; anything without
  an `@` is already an id, because the vendor's prefix has changed before.

  `c:mailbox_internal(id)` and `c:mailbox_id(id_or_address)` are the two reads underneath, exposed
  because a caller connecting a whole fleet needs both.

## assay-lua 0.20.4 — 2026-09-04

### Added

- **`assay.email_triage` reads what a message says about itself.** `M.categorize` buckets by
  matching a fixed keyword list against the subject and nothing else. Probed on 0.20.2 it misses a
  keyword anywhere but the subject, and "unsubscribe please" lands in `needs_reply`, which is the
  opposite of what it is. A reply-reading lane needs a pass that costs nothing and is not a guess,
  so a message it can read never reaches a model at all.

  `M.signals(msg)` takes `{headers, subject, text, html?}` and answers five independent readings —
  `auto_reply`, `bounce`, `out_of_office`, `unsubscribe`, `referral` — each `{present, evidence}`
  where the evidence is the header or the sentence that decided it. None of the five ranks the
  others: a message can be an away notice that also asks to be left alone, and which of those wins
  is the caller's policy rather than this module's. Every signal is in the answer whether or not it
  fired, so `s.bounce.present` reads without a nil check first.

  **Only the sender's own words are searched.** Cold outreach carries an unsubscribe line on every
  send, and a reply quotes it underneath. Matched there, every reply anyone ever sends reads as
  somebody asking to be left alone. `M.own_words` cuts at the first quote marker in four languages,
  and at forty lines regardless. A client that quotes with no marker leaves a header block instead,
  written in whatever language it runs in; that counts as a quote only where the sender line carries
  an address and a second header follows it within four lines, so a reply opening "From: our end,
  this looks fine" keeps everything it went on to say. The two word lists behind that are exported
  as `M.HEADER_FROM_WORDS` and `M.HEADER_NEXT_WORDS`, and hold what Neutron's own reader holds. A
  cut that would leave nothing found the whole message rather than a quote — a forward typed out by
  hand — and keeps it. A bounce is the one exception and reads the whole body, because a delivery
  report has no quoted reply to cut at.

  **A return date with no year is the next occurrence of it.** Read as this year, a December message
  naming January lands in the past and the follow-up goes out the same day, into an inbox nobody is
  reading. A day that is today is today. A date the calendar does not have is no date at all, so "31
  February" reads as nothing rather than rolling forward to the first of March.

  **`List-Unsubscribe` is noted and never counted.** It is a header put on outbound mail, so a reply
  quoting the letter carries it back; counted, every reply to a compliant campaign would read as an
  opt-out. `Precedence: bulk` is reported the same way, beside the verdict rather than as one,
  because bulk is what a mailing list sets and a person can write from a list.

  English, German, French and Spanish, through `M.fold`: `string.lower` is byte-wise ASCII, so
  "BÜRO" lowercases to something that never matches "buro". Both cases of every accented letter the
  phrase lists use are folded, along with the curly apostrophe mail clients substitute silently. The
  phrase lists are exported, so a caller adding a language extends them rather than forking the
  reader.

  `M.categorize` and `M.categorize_llm` are untouched and still exported. This is a function beside
  them, not a replacement.

## assay-lua 0.20.3 — 2026-09-04

### Added

- **The vendor modules report what the fleet costs, where the vendor will say.** A caller pricing a
  mailbox fleet had to keep a price list beside these modules and trust it still matched what the
  vendor was charging. Three of the four surfaces now answer for themselves; the fourth says
  outright that it will not.

  `assay.clayinbox` gains `c:costs()`. The price rides on the mailbox row — `cost` as a decimal
  string, beside the cycle it repeats on — because every invoice, order and price path the vendor
  might have put it behind answers 404. Rows sharing a price and a cycle collapse into one item, so
  `quantity` means something, and the cycle is part of the grouping key: a yearly box never lands in
  a monthly line at the same number. Only a live mailbox is billed: a box the vendor stopped
  charging for inflates the bill with spend nobody is making, so a cancelled or suspended one is
  counted in `meta.inactive`. A row whose status the vendor did not state at all is counted in
  `meta.status_unknown` instead, because calling it inactive would report a cancellation nobody
  made. Between them and `meta.unpriced`, every row that went unbilled says which of the three
  reasons it was.

  `assay.salesforge` gains `c:costs()`, off the web app's own `/me` — the only surface that carries
  the plan at all. Every plan, billing, usage and limits path under the public workspace is a flat
  404, and the internal subscription route answers "growth subscription not found" for an account
  that never bought one. The vendor names no money anywhere on it, so the plan item carries no price
  and `meta.priced` is false outright rather than letting an absent price read as free. `meta`
  carries the plan's monthly ceilings and the credit pools beside them.

  `assay.forge` gains `p:domain_price(domain)`, the only price either forge product answers. It
  quotes registration for a year — a bought domain's `expiresAt` lands a year after its `createdAt`
  — and says nothing about what the workspace is charged today. Neither product has a billing
  endpoint, and the module description says so.

- **`assay.vendor_cost`, the contract those three answer with.** One item shape and one money
  conversion, so the three modules cannot drift apart on either.

  An item is `{kind, unit, ref, quantity, unit_price_cents, period, source}`. `unit` is the unit of
  measure — `"mailbox"`, `"domain"`, `"plan"` — and `ref` names the instance a line applies to,
  which a line covering a group of them does not have. A price or a period the vendor never stated
  is an absent key, never a zero: `meta.priced` says whether any line carries money at all, and
  `meta.currency_known` is false on all three, because not one of these vendors states a currency
  anywhere.

  Money is whole cents, converted once. A fleet priced in floats accumulates a rounding error across
  every row, and `19.99` landing at 1998 rather than 1999 is pinned by a test. `tonumber` reads
  `"0x10"` as 16 and `"1e2"` as 100, so a price is digits with at most one decimal point and
  anything else is counted as unpriced rather than billed at sixteen hundred cents.

  A refused key reads as a typed error rather than as an empty item list. A costing that answers
  "nothing" when the credential is wrong is the most expensive way this can fail, so 401, 402, 429
  and 5xx each read as themselves, and a `/me` that is not an account object is a read error rather
  than a workspace entitled to nothing.

## assay-lua 0.20.2 — 2026-09-04

### Added

- **`assay.salesforge` can set a sequence's mailbox rotation and its status.** The two write calls
  the sequencer seam needs and 0.20.1 left out. Without them the caller has to keep the vendor's
  host name and its own HTTP client, which is the coupling these modules exist to remove.

  `c:set_rotation(sequence_id, mailbox_ids)` replaces which mailboxes a sequence sends from, so a
  caller taking one domain out of the rotation sends back the ids it means to keep.
  `c:set_sequence_status(sequence_id, status)` takes `"paused"` or `"active"`. Both answer `(true)`
  or `(nil, err)` like `c:enrol` and `c:dnc`, and `c:sequence(id)` already reads both back.

  An empty rotation is a real instruction rather than an error. Pulling a paused domain's mailboxes
  can leave a sequence with none, and that is the truthful state — it then cannot send, which is
  what `c:set_sequence_status` is beside it for. Forcing the caller to keep one stale mailbox in the
  rotation to express "none" would be worse. The ids are copied onto a table marked
  `__jsontype = "array"` so an empty list reaches the vendor as `[]`; a bare Lua table would encode
  as `{}` and be read as a malformed object. The copy keeps the marker off the caller's own table.

  Two things are still refused before a request is made rather than after the vendor rejects one: a
  blank sequence id, which would address the workspace itself, and any status outside the two the
  vendor accepts, so a typo reads as a config error the caller can act on instead of a 400 it has to
  interpret. A `mailbox_ids` that is not a table is refused for the same reason — it is not a list.

## assay-engine 0.5.18 — 2026-09-04

### Added

- **`assay-engine migrate` moves a store from SQLite to Postgres.** The engine could be pointed at
  either backend but never carried one to the other, so a deployment that outgrew its volume had to
  start over. What it would have thrown away is not only workflow history: the same store holds the
  auth module — users, password hashes, sessions, passkeys, JWT signing keys, OIDC clients, Zanzibar
  tuples — and the vault, whose master KEK is a row in `vault.kek_metadata` rather than an
  environment variable. Copying that row is what lets the ciphertext in `vault.kv` decrypt on the
  other side, and a migration that moved the secrets without it would have produced a store that
  looked intact and could not be read.

  `--from sqlite:<data-dir> --to postgres://…` copies every table of every schema the engine owns,
  preserving ids and timestamps, so a reference to a run still resolves afterwards. The table list
  comes from `sqlite_master` and the Postgres catalog rather than from a list in the code, so a
  module that adds a table is carried without an edit here. Generated-id sequences are re-pointed
  past the copied rows, because rows that already own ids 1..N leave a sequence at 1 handing the
  next write a collision.

  A target that already holds engine rows is refused by name before anything is written: merging two
  stores would have to reconcile ids that were only ever unique within one of them. A column the
  source has and the target does not is an error rather than a silent drop; the reverse takes its
  default. `--dry-run` prints the plan and the source's row counts and writes nothing. Both modes
  print a row count per table. `engine.lock` is skipped and said so — SQLite serialises
  single-instance access through that table where Postgres uses an advisory lock — as is any other
  source table with no Postgres counterpart, reported with the rows it held.

  Order of operations, and how to verify the result before keeping it, are in
  [`docs/engine-store-migration.md`](docs/engine-store-migration.md).


- **`ASSAY_VAULT_SEAL_KEY` encrypts the vault's master key at rest.** The KEK was stored as raw
  bytes in `vault.kek_metadata`, so the row protecting every secret sat beside the secrets it
  protects. On a volume that was one exposure; with the store in Postgres the nightly dump becomes
  a plaintext copy of the whole vault, and so does every backup of it.

  Set the variable to any string of at least 32 characters and the KEK is sealed with
  AES-256-GCM-SIV instead — a version byte, a nonce, and the encrypted key, with the key id as
  additional authenticated data so a blob copied onto another row does not open. The cipher key is
  derived from the value with SHA-256 over a fixed label rather than decoded from it, so base64,
  hex and a passphrase all work and nothing depends on the encoding a chart happens to emit.
  A store already holding a plaintext KEK is
  re-sealed in place on the first boot that has the key, which makes turning it on a restart rather
  than a migration, and logs that backups taken before then still hold the unsealed key. Re-running
  is a no-op. Rotation keeps the sealing rather than writing the next KEK in the clear.

  Losing the key is not recoverable, so a sealed store with the wrong key or none at all refuses to
  start rather than minting a fresh KEK and orphaning every secret the old one wraps. Without the
  variable behaviour is exactly as before, warning as it always did.
  [`docs/vault-sealing.md`](docs/vault-sealing.md) covers the trade and the failure modes.

### Fixed

- **The engine no longer takes tables it did not create.** Its v0.13.1 upgrade step ran on every
  boot and moved `public.workflows` and `public.namespaces` into the `workflow` schema, and dropped
  `public.api_keys` with `CASCADE`, on nothing more than the names matching. Pointed at a database
  a host application also uses, it took that application's tables: 30 rows and their own columns
  relocated under the engine, the application's reads failing with `relation "public.workflows"
  does not exist`, and `public.api_keys` gone for good. The engine broke too, since the tables it
  had adopted were not the shape it expected. `ALTER TABLE ... SET SCHEMA` is not undone by
  reverting a deploy.

  The move now requires proof the tables are the engine's own. `public.workflow_events` is the
  marker — every v0.13.1 store has it, no application is holding a table by that name, and it moves
  in the same transaction as the rest. Without it nothing moves. With it, the two ambiguously named
  tables must also carry the columns the engine's own versions always had. Anything failing either
  check is left alone and named in the log, and `public.api_keys` is now reported rather than
  dropped, because an orphaned table costs nothing and an unrecoverable drop does not.

  Run the engine in its own database regardless. It owns four schemas, and the default config
  example says so.


- **Engines starting together on an empty Postgres no longer kill each other.**
  `CREATE ... IF NOT
  EXISTS` is not atomic: Postgres runs the existence check before the catalog
  insert, so two engines booting at the same instant both passed the check and one lost with
  `duplicate key value violates
  unique constraint "pg_type_typname_nsp_index"` — or
  `pg_namespace_nspname_index` for a schema — and exited instead of serving. Ten engines started
  together on a fresh database lost nine.

  The engine-core schema had already been serialised behind `pg_advisory_xact_lock`, and its doc
  comment named this exact behaviour, but the auth, vault and workflow migrations and the engine's
  own `CREATE SCHEMA` loop each ran their DDL outside it. All four now take the same
  transaction-scoped advisory lock, so concurrent boots serialise rather than race. Transaction
  scope means commit, rollback and a dropped connection all release it, so a migration that dies
  cannot strand the lock. A schema setup that still loses a catalog race — a caller reaching
  Postgres without the lock — retries instead of failing, on the three codes the catalog raises for
  it. A test boots ten engines at once on a fresh database, five times over.

## assay-workflow 0.4.7 — 2026-09-04

### Fixed

- Postgres schema migration runs inside one advisory-locked transaction, so concurrent first boots
  serialise their DDL instead of racing `CREATE TABLE IF NOT EXISTS` and losing one caller to a
  catalog unique violation, and retries when it loses one anyway.
- The v0.13.1 relocation moves a table only once the database proves it is the engine's own, and
  reports rather than drops the retired `public.api_keys`.

## assay-auth 0.6.3 — 2026-09-04

### Fixed

- Same advisory lock around `schema::migrate_postgres`. The `backend-postgres` feature now also
  enables `assay-domain/backend-postgres`, which it had always needed transitively.

## assay-vault 0.4.4 — 2026-09-04

### Added

- `crypto::env_seal` seals the master KEK under a key derived by SHA-256 from an environment
  string of at least 32 characters, and
  `kek_store::load_or_init_*_sealed` load, re-seal and refuse accordingly. `SealingMethod::EnvKey`
  and `VaultCtx::with_kek_method` carry the method through to `/sys/seal-status`, which reported
  every store as plaintext before.

### Fixed

- Same advisory lock around `schema::migrate_postgres`, and the same `backend-postgres` feature fix.

## assay-domain 0.2.5 — 2026-09-04

### Added

- `engine::SCHEMA_MIGRATION_LOCK` and `engine::acquire_schema_lock`, the one advisory-lock id every
  Postgres schema migration in the workspace holds while it runs DDL. Modules share one id rather
  than taking their own because they share the `CREATE SCHEMA` statements even where their tables
  are disjoint. `engine::retry_ddl` re-runs a migration that lost a catalog race regardless.

## assay-lua 0.20.1 — 2026-09-04

### Added

- **`assay.clayinbox`, `assay.forge` and `assay.salesforge` — the cold-email vendor stack as stdlib
  modules.** The logic had been living twice: once in an ops script that reads OpenBao itself, and
  once in a TypeScript adapter inside one product. Neither is reachable from an agent that bakes
  assay modules as tools. All three take their credentials from the caller (`opts`, or a documented
  environment variable) and read no secret store, so the same module serves a script, a service and
  an agent.

  `assay.clayinbox` lists the domains a workspace holds and the mailboxes on them, paged to the
  last row. `assay.forge` speaks the shared forge MCP endpoint for both Primeforge and Warmforge —
  domains, mailboxes, warm-up position, placement tests and the DNS health report. `assay.salesforge`
  covers the public REST API (workspaces, mailboxes, sequences, contacts, do-not-contact, replies)
  and the web app's own Firebase-authenticated API, which is the only place the warm-up state
  appears.

  Six vendor behaviours are pinned by tests because each one has already cost someone a wrong
  answer. A Primeforge domain arrives as `sld` and `tld` and never as a whole name, so a reader
  looking for one finds no domains at all. A Warmforge health check the report omits is `unknown`
  and never `invalid`, because reading an omitted check as a failure tells an operator a record
  they published is missing. Warm-up length is the sum of the days done and the days left rather
  than a constant kept in step with the vendor's. A placement row carrying no folder counts is a
  test nobody ran, not a placement of zero. The Salesforge public key rides bare in `Authorization`
  — an apiKey scheme, not a bearer one — and a workspace with no sequences answers with a JSON
  object where a list belongs. Auth, rate limiting, the Growth-plan gate and a Cloudflare block page
  served under an HTTP 200 all read as themselves rather than as an empty fleet.

  Errors are returned, not thrown: every vendor call answers `(result)` or `(nil, err)` where `err`
  carries `code`, `status` and `message` and prints as its message. The constructors are the
  exception and throw, matching the rest of the stdlib — a client built without a key or a
  workspace is a programming error rather than a vendor answer. `raw` on a mapped row is the
  vendor's own record with credentials removed, since both Clayinbox and Primeforge put a mailbox
  password on a list row.

  Every list call also answers a second value, `meta = {truncated, cap, seen}`. `truncated` means a
  cap stopped the walk rather than the vendor running out of rows, so no list can come back short in
  silence. It matters most on Primeforge, where a domain filter is applied to a ten-row window: an
  empty result there means either that the domain has no mailboxes or that its mailboxes fall
  outside the window, and only `meta` tells the two apart.

  One limit is the vendor's rather than the module's: `primeforge_list_mailboxes` accepts
  `workspaceId` and nothing else, and answers ten rows whatever `limit` and `offset` say — offset 10
  and offset 20 return the same ten ids. A workspace holding more than ten mailboxes cannot be
  listed in full through that tool, so `p:mailboxes(domain_id)` filters what the vendor gives rather
  than sending a filter the tool would ignore.

## assay-engine 0.5.17 — 2026-09-03

### Fixed

- **0.5.16 exited at startup on every store created by an earlier engine.** The baseline schema
  created the `events.activity_id` index before the migration added the column, so an existing
  database failed on `no such column: activity_id` (SQLite) or `column "activity_id" does not
  exist` (Postgres) and the engine never came up. Fresh databases were unaffected, which is why the
  release tests passed. The index is now created after the column, and a schema-upgrade test opens
  both stores on a pre-0.5.16 database.

## assay-engine 0.5.16 — 2026-09-03

### Fixed

- **Activity completion is one transaction: the row, the history event, and the workflow task.** The
  engine wrote those three separately, so a slow disk could land the first and lose the rest —
  `POST /tasks/{id}/complete` returned 200, the activity read `COMPLETED` with its result, and the
  workflow's history showed the activity still scheduled. A deterministic workflow replays that
  history, waits on an activity that already finished, and stays `RUNNING` forever. In the reported
  case the only recovery was to re-post the identical completion by hand.

  `WorkflowStore::settle_activity` now applies all three in one transaction, so `COMPLETED` without
  the matching event is unreachable rather than unlikely. It is idempotent, which makes re-posting a
  completion the documented repair path: an activity whose event is already durable re-arms the
  dispatch flag and appends nothing, recovering a workflow task lost after the event landed — the
  second shape the same disk produced. Signal delivery closes the same way: the signal row, its
  `SignalReceived` event and the dispatch arming commit together, since a stored signal the workflow
  cannot see is a run that waits on nothing.

  Rows already half-settled by an earlier engine still needed a way out, so the health monitor now
  re-settles them on its next pass. `workflow.events` carries an `activity_id` column to make "did
  this activity's terminal event land" an indexed question rather than a scan of payload JSON;
  existing terminal events are backfilled from their payloads at startup.

- **A worker whose registration was reaped now registers again instead of polling a dead id.**
  Heartbeating an id the reaper had already removed answered `200`, so the worker kept polling under
  a registration no queue dispatched to until someone restarted the process — observed as a worker
  that went missing for thirty minutes and came back only on a pod restart. The endpoint answers
  `404` when the row is gone, and the Lua worker re-registers on it.

## assay-lua 0.20.0 — 2026-09-02

### Added

- **`dns` builtin — the record types `getaddrinfo` cannot ask for.** `dns.lookup(name, type, opts?)`
  answers `A`, `AAAA`, `CNAME`, `MX`, `NS` and `TXT`, with `opts.{server, timeout_ms, tries}`;
  `dns.dnsbl(domain, list, opts?)` asks a blacklist about a domain. Domain health was the case that
  forced it — MX, SPF, DKIM, DMARC and blacklist hits are all questions the stub resolver has no way
  to put — and it had been living in a bash script around `dig`, which cannot run inside an agent
  that bakes assay modules. `examples/domain-health.lua` is that script, ported.

  Three decisions in it are load-bearing rather than incidental. A `TXT` record's 255-byte chunks
  are rejoined with nothing between them, because they are one value the wire format had to cut up
  and a separator corrupts every DKIM key long enough to need two of them. `NXDOMAIN` is an empty
  list while `SERVFAIL`, `REFUSED` and timeouts raise, because "nothing lists this domain" and
  "nobody answered" look identical once failures collapse into an empty result, and they mean
  opposite things to whoever is about to send mail. And `127.255.255.0/24` does **not** count as a
  DNSBL listing: Spamhaus and SURBL return it to resolvers they do not serve, so reading it as a hit
  marks every domain checked as blacklisted. It is still reported in `codes`, so a caller can tell
  "not listed" from "not allowed to ask".

  The protocol is spoken directly over UDP with TCP fallback rather than through a resolver crate —
  no new dependency, and the wire format's awkward parts (chunk joining, MX ordering, RCODE meaning,
  compression-pointer loops) are unit-testable as pure functions. Compression pointers must point
  strictly backwards, so a self-referential message errors instead of hanging. Queries carry an
  EDNS0 buffer of 1232 bytes, since the classic 512-byte limit truncates ordinary DKIM keys.

  Nameservers come from `/etc/resolv.conf` in the order it lists them, with no public fallback: a
  script that believes it is asking the corporate resolver should not silently ask someone else's.
  `opts.server` overrides that, and is refused outright when a policy is installed — a caller-chosen
  resolver is a directed egress channel, since a restricted script could carry data out in the names
  it looks up. Resolution through the system resolver stays unbounded, though: a policy has no
  allowlist of names, so a policed script can still reach an authoritative server of its choosing by
  looking up a name under it. `docs/policy.md` says so under **DNS** rather than leaving it to be
  discovered.

## assay-lua 0.19.2 — 2026-08-28

### Added

- **`assay.companies_house` — the UK register, and the first free source that still needs a key.**
  Company search, full profiles, and the officers a profile does not name — which is the reason to
  reach a registry for outreach at all, since the profile names the company and the officer list
  names the person to write to. The key is free but issued per caller, so the client refuses to
  construct without one rather than failing later with a `401` that reads like an outage; the key is
  the Basic username with an empty password, and the trailing colon is load-bearing. Search hits and
  company profiles describe the same entity under different names — `title`/`company_name`,
  `company_type`/`type`, `address`/`registered_office_address` — and both are read, because reading
  one set yields a record with a nil name from the other endpoint. Twelve registry statuses bucket
  into the three the other registry modules answer in; a status the registry adds later is
  upper-cased rather than mapped to `ACTIVE`, which would be the one wrong answer.
- **`assay.mails_so` — the paid verify_email rung.** One GET per address through the budget gate.
  The vendor's "deliverable" lands as PROBABLE, never VERIFIED — only our own evidence verifies —
  and a domain that accepts anything is CATCH_ALL whatever the vendor concluded. Raw verdict, score,
  MX and reason ride along on the record. Gated live smoke arms itself when `MAILS_SO_KEY` is
  present.

### Changed

- **`assay.gleif` and `assay.edgar` now answer in the shared company shape.** They predated
  `lead_provider.company` and carried a hand-rolled provenance with no `retrieved_at`, so a fact
  from the two oldest registry modules was not interchangeable with one from `brreg`, `cvr` or
  `companies_house` — which is the whole claim of NEP-0007 §10. Both now normalize through the
  shared constructor. `edgar:find` returns company records rather than raw index rows; `tickers()`
  stays a lightweight index, since stamping ten thousand rows individually buys nothing.

### Fixed

- **Three field mappings that only a live response could disprove**, all found by running the
  modules against the real APIs rather than against their own fixtures:
  - `assay.gleif` cited Equinor's Norwegian number as `923 609 016` where Brreg holds `923609016`.
    Unstripped, the two registries never joined on the company they both describe — which was the
    entire point of putting a national `registry_id` on a GLEIF record.
  - `assay.edgar` read a domestic filer's `stateOrCountry` as the country, turning Apple's
    California into `CA` — indistinguishable from Canada. EDGAR states no country for domestic
    filers and inverts the fields for foreign ones, so the two cases are now read separately.
  - `assay.edgar` reported CIK `320193` from the ticker index and `"0000320193"` from submissions,
    handing out two identities for one company and breaking any join between its own two surfaces.
    It also passed EDGAR's empty-string `website` straight through as a blank domain claim.

- **`assay.brreg` and `assay.cvr` — worldwide reach starts with the registries that are actually
  open.** Norway's Enhetsregisteret and Denmark's CVR are keyless, and Brreg publishes two things
  most national registries do not: the company website and a live employee count. That makes
  `by_website` possible — _which legal entity owns this domain_ — which is the join a prospect list
  actually needs, since the list holds domains and the registry holds companies. Normalisation
  carries the weight: Norway's three distress booleans collapse to one status, an unreported
  headcount stays absent rather than becoming zero, Denmark's `04/12 - 2013` becomes an ISO date,
  and a website registered as `https://WWW.X.no/` reduces to a joinable host. `lead_provider` grows
  a shared `company` shape, so a registry fact and a bought fact differ only by provenance (NEP-0007
  §10).

- **Live smoke tests for the paid lead providers.** Gated on a key being present, so they skip
  everywhere a key is absent and the suite stays green. They check the half a fixture cannot: that
  the vendor still sends the fields the adapter reads. That mattered once already — BetterContact's
  verdict field was wrong, and no fixture could catch it because the fixture carried the same
  mistake. One of them proves a declined budget stops a call against the real API rather than a
  mock.

- **`assay.bettercontact` read the wrong field for the email verdict.** The vendor's field is
  `contact_email_address_status`, not `contact_email_status`, and its deliverable value is
  `deliverable`, not `valid` — `valid` is a counter in the summary object, not a per-contact
  verdict. Reading the wrong name yielded nil, which mapped to UNKNOWN, and UNKNOWN never schedules
  under NEP-0007 §2 — so every enriched address came back silently unusable rather than visibly
  broken. The full documented enum is now covered, `catch_all_safe` stays CATCH_ALL rather than
  being promoted on the vendor's say-so, and the raw verdict rides along as `vendor_status` so the
  nuance survives without laundering.

- **`assay.lead_provider`, `assay.contactout`, `assay.bettercontact` — paid lookups get a gate they
  cannot walk around.** The contract module carries the uniform person/email shapes with provenance,
  so free registry facts and bought ones read identically downstream, and it owns the budget gate:
  the spend ledger lives in the caller's database, so the context is injected, and a client cannot
  be constructed without one. A declined budget means the provider is never called, and a call that
  raised is never metered — the ledger answers what things cost, and a failed call bought nothing.
  ContactOut wants the bare `token` header (not Authorization, no Bearer). BetterContact is
  asynchronous and answers 202 with no data while a run is still going, so only its own `terminated`
  status counts as finished. Neither adapter can promote a vendor's claim to VERIFIED; `valid` means
  PROBABLE, because an assertion is not a delivery.

- **`smtp_probe` — email verification finishes inside the binary.** The rung above DNS is now a
  compiled builtin rather than a service to run or a vendor to pay: connect, greeting, EHLO (falling
  back to HELO), MAIL FROM, a random-address RCPT that exposes catch-alls, the target RCPT, QUIT.
  DATA is never issued, so a probe cannot deliver anything. `email_verify` grows a `probe()` that
  turns those replies into NEP-0007's vocabulary — PROBABLE for an accepted recipient, CATCH_ALL for
  a host that accepts anyone, INVALID only when a server names the mailbox as absent — plus
  disposable, role and typo signals carried as flags, never as a status, because INVALID is
  permanent and a shortlist can be wrong. An accepted RCPT stops at PROBABLE on purpose: servers
  accept at RCPT and bounce afterwards. The cost is tens of KB on networking already in the binary,
  and the one-binary story is intact.

- **`assay.email_verify` — the waterfall's free rung.** Syntax that refuses what could never
  deliver, MX lookups over DNS-over-HTTPS (keyless, deterministic, one mock in tests), and the
  pattern candidates an executive address usually takes. Its vocabulary is deliberately capped at
  INVALID and UNKNOWN — the statuses that let a pipeline reject cheaply without ever inflating free
  evidence into a send-safe verdict.

- **`assay.gleif` and `assay.edgar` — the first registry modules.** Company discovery kept paying
  (or scraping) for facts that sit in open registries. GLEIF answers "does this legal entity exist,
  where, under what status" for every jurisdiction with no key at all; EDGAR answers the US
  public-company half — tickers, SIC, addresses, filings, full-text search — behind nothing but an
  identifying User-Agent, which the client refuses to run without. Both normalize to one flat
  registry shape and stamp provenance on every record, so a fact fetched here stays auditable
  wherever it flows.

## assay-engine 0.5.15 — 2026-08-20

### Changed

- Ships assay-dashboard 0.6.0 (sign-in composition below). No engine behaviour changes.

## assay-dashboard 0.6.0 — 2026-08-20

### Changed

- **The sign-in composition now fills the screen it is given.** The two-area layout shipped at a
  phone's measure and kept it on a desktop: a 40px headline and a 400px card marooned in the middle
  of a 1920px window, with the story and the form each reading as an isolated block. Column widths,
  type sizes, card padding and control heights are `clamp()`ed against the viewport, so the page
  scales as one composition instead of stepping between two fixed sizes.

  The ground carries two accent blooms — behind the brand mark and under the composition — mixed
  from `ASSAY_WHITELABEL_ACCENT`, so a deployment re-colours the whole page from the one variable it
  already sets. The headline's second line takes the accent rather than receding into muted grey,
  the brand mark blooms at logo size, and the trust note gets a shield of its own instead of the
  rule it shared with block quotes.

  Form work: both fields carry placeholders, the primary submit is a gradient off the accent and
  sits a notch taller than the inputs it follows, and the provider buttons centre their label. The
  password-reset page picks up the same placeholders.

### Added

- **`ASSAY_WHITELABEL_LOGIN_BRAND`** — a sign-in wordmark for product names that read as two parts.
  Text after `|` takes the accent colour, so `Neutron|Core` renders `Core` in the brand colour.
  Unset renders `ASSAY_WHITELABEL_NAME` in one colour, unchanged.

### Breaking

- `WhitelabelConfig` gains a `login_brand` field. Callers constructing it as a struct literal need
  the extra field; `WhitelabelConfig::from_env` is unaffected.

## assay-dashboard 0.5.0 — 2026-08-19

### Added

- **A sign-in page that can say what the product is.** The hosted login was a centred card on a flat
  rectangle: a badge, two fields and a button. Every operator running assay as the front door of
  their own product got a page that said nothing about that product, and assay's own orange read as
  their brand because changing it meant hosting a stylesheet.

  Sign-in is now a two-area composition — the operator's story beside the credential panel —
  assembled entirely from whitelabel config. `ASSAY_WHITELABEL_LOGIN_HEADLINE` turns it on;
  `_SUBHEAD`, `_ROSTER_TITLE`, `_ROSTER` and `_NOTE` fill it, and `ASSAY_WHITELABEL_ACCENT`
  re-colours the page from one variable. With no headline configured there is no story element and
  the page keeps the centred card every existing deployment has today.

  The roster rows are an illustration the operator writes (`Label:tone:Status`, capped at five),
  never account data — assay styles three tones and reads no state to fill them, and the panel is
  labelled as an illustration for assistive tech. An unknown tone degrades to `pending` and an
  incomplete row is dropped, so a typo in env cannot take sign-in down.

  The brand badge is a token now, so an operator with a real mark gets it rendered instead of an
  initial in a coloured square.

### Fixed

- **The password-reset page showed both of its forms at once.** `recovery.js` swaps its request and
  completion forms by toggling `hidden`, but an author `display` beats the UA's `[hidden]` rule and
  `.password-login` declares one. Restored with an explicit rule.

- Sign-in form details: fields gain hover and focus treatment, the error region reserves its space
  so a failed attempt cannot shove the button down, the submit button reports that it is working,
  and a reveal control flips the password field and re-masks it on failure. The recovery link moved
  after the input in the DOM so a keyboard reaches the password field before the way around it.

## assay-engine 0.5.14 — 2026-08-19

### Changed

- Ships assay-dashboard 0.5.0 (sign-in redesign above). No engine behaviour changes.

## assay-vault 0.4.3 — 2026-08-16

### Added

- **A HashiCorp Vault / OpenBao read facade, so an estate adopts assay-vault by repointing a URL.**
  Estates that already hold their secrets in Vault or OpenBao consume them through one dialect:
  `X-Vault-Token` for auth, `/v1/{mount}/data/{path}` for a KV2 read, and a fixed response envelope.
  External Secrets Operator's vault provider speaks it, ansible's `community.hashi_vault` speaks it,
  `vault kv get` and curl speak it. Until now none of them could point at assay-vault, so adopting
  it meant rewriting every `ExternalSecret` and every inventory lookup. The new `hashicorp_compat`
  module serves that dialect on top of the existing KV store.

  Read-only by construction: `GET /v1/{mount}/data/{path}` (with `?version=N`), `GET` and `LIST` on
  `/v1/{mount}/metadata/{path}`, and `GET /v1/sys/health`. Writes, rotation, sealing, and token
  issuance stay on the native `/api/v1/vault/*` surface where the policy and the audit trail already
  live; any other method on a facade route answers `405`, and there is no `sys/mounts`, no auth
  mount, and no token endpoint to find.

  A Vault token IS an assay token — the facade presents `X-Vault-Token` to the embedder's existing
  admin-bearer / trusted-JWT gate as the bearer it already checks, so there is no second token store
  to keep in sync and one enforcement point rather than two. Answers speak Vault's vocabulary:
  `403 {"errors":["permission denied"]}` for a rejected or absent token (Vault answers 403 for
  both), `404 {"errors":[]}` for a missing path, `503` and a sealed `sys/health` when the engine is
  sealed.

  The mount is a label, not a namespace: it names the one logical KV2 mount an engine exposes
  (`secrets` by default) and is stripped before the lookup, leaving the assay KV path verbatim, so
  `secrets/data/platform/postgres` and `/api/v1/vault/kv/platform/postgres` are the same secret. Any
  other mount name is a 404 rather than a quiet read of a different path. assay KV stores an opaque
  UTF-8 payload per version where KV2 hands back an object, so a payload that parses as a JSON
  object is served field-by-field and anything else is served as `{"value": …}` — an
  `ExternalSecret` naming `property: password` works, and a single-string secret stays reachable.

  Gated behind the `vault-hashicorp-compat` Cargo feature (in the default `vault` umbrella) and, in
  the engine, behind config. Documented in `docs/vault-hashicorp-compat.md`.

## assay-engine 0.5.13 — 2026-08-16

### Added

- **`[vault.hashicorp_compat]` mounts the Vault / OpenBao KV2 read facade at `/v1/*`.** Off by
  default: serving a second dialect of the secret store at the engine root is a deliberate act, not
  something an upgrade should switch on. `mount` (default `secrets`) names the logical KV2 mount, so
  consumers keep the paths their old OpenBao used. The routes sit at the server root because Vault
  clients hardcode `/v1/…` and cannot be told to use a prefix — point `VAULT_ADDR` or an ESO
  `server:` at the engine's base URL. `GET /v1/sys/health` is unauthenticated, as Vault's is;
  everything else carries the same admin-bearer gate as every other engine module surface.

## assay-lua 0.19.1 — 2026-08-15

### Added

- **`assay.authz` — in-process authorization engine.** Scripts that gate an action on who is asking
  had two options, both bad: hand-rolled `if` ladders that drift between scripts, or a network hop
  to a policy service that a host-management script cannot depend on being up. The new module
  decides in-process, with no I/O, no storage and no expression language to sandbox — the policy is
  data, and the vocabulary is closed by construction, which is what makes it safe to let an agent
  author one.

  `authz.engine(opts)` takes a grant universe plus a declared vocabulary (condition keys, scope
  kinds, an action registry) and returns an engine; `e:check(subjects, action, resource, opts?)`
  answers one question. The semantics are the interesting part, and all of them fail toward less
  access: deny wins over any allow in the resolved scope chain; a condition that cannot be evaluated
  withdraws an allow but leaves a deny standing; a grant's `bounds` narrow the allow statements of
  the policy it confers and never its denies; a malformed or undeclared scope entry denies outright
  rather than quietly evaluating a different chain; and an action names a base that covers a closed,
  enumerable family of derived actions rather than a wildcard. `e:validate` refuses a statement or
  condition the evaluator could never evaluate, `e:describe` serves the whole vocabulary as data for
  an administration surface, and `e:grants_for` lists what applies over a chain for a "why" view.

- **`assay-authz` crate.** The engine lives in its own workspace crate, dependency-light (serde,
  serde_json, chrono) and usable from Rust without the Lua runtime. It decides every case in the
  [agentauthz](https://github.com/developerinlondon/agentauthz) conformance fixture set identically
  to that reference library: all 149 language-neutral golden fixtures are vendored verbatim under
  `crates/assay-authz/conformance/cases` and run on every build, through both the pure evaluator and
  the composed engine. Conformance is the claim the fixtures support — behaviour outside their reach
  is covered by this crate's own tests, not by that suite. The four fixtures marked
  `storable: false` name a conditions shape a storage layer must refuse at rest; with no storage
  layer here, the suite asserts `validate` refuses them and the evaluator still fails closed on
  them.

  Behaviour the fixtures do not reach is pinned against the reference implementation directly: a
  context value of any shape yields a decision rather than aborting the check (an empty list, a
  boolean, a number under a string key, and a value the reference could only stringify all follow
  its verdicts); numbers render through JavaScript's `String(n)`, exponential form and all, because
  that is what a policy value was written to match; and a grant or statement missing a required
  field degrades to a dropped grant or a skipped statement rather than taking the engine down at
  construction.

  Grant bounds have no write boundary in-process, so the evaluator applies the one the reference
  applies at its own: a bound `validate` would refuse is unmatchable, and can never narrow an allow
  into existence. `e:validate_bounds(...)` exposes the same check to a host that owns storage.

## assay-lua 0.19.0 — 2026-08-14

### Changed

- **`ModuleMetadata`, `QuickRef` and `DiscoveredModule` are now `#[non_exhaustive]`.** Adding `icon`
  and `category` to `ModuleMetadata` is a breaking change — `cargo-semver-checks` classes any field
  added to an externally-constructible public struct that way — which is why this is a minor bump
  rather than the patch the rest of the change would have warranted. Rather than pay that again on
  the next tag, all three types are marked `#[non_exhaustive]` in the same release: assay parses or
  discovers every one of them and hands it out, nothing outside constructs one, and closing
  struct-literal construction makes a future field addition a patch. Both breaks are covered by this
  one minor. Code outside the crate that built one by literal starts from `Default::default()`
  instead, or reads the value assay returns; `modules --json` output is unchanged.

### Added

- **`@icon` and `@category` module metadata.** `modules --json` told a consumer what every module
  does but nothing about how to present it, so a catalogue UI had one shape for all 89 entries.
  Modules now carry an optional `--- @icon <simple-icons slug>` and a `--- @category <name>`, both
  parsed alongside the existing `@keywords`/`@env` tags and both emitted by `modules --json`; `icon`
  is `null` rather than absent when no brand fits, so a consumer can branch on one shape. Categories
  are `kubernetes`, `gitops`, `secrets`, `identity`, `observability`, `cloud`, `saas`, `devtools`,
  `comms`, `host`, `data`, `core`, and every shipped module has one — a test walks discovery and
  fails on a module that ships without one or invents a thirteenth bucket.

  Every embedded stdlib module is backfilled, 32 of them with a brand mark. Slugs were checked
  against the published `simple-icons` 16.28.0 package rather than derived from the brand name,
  which is not a formality: `argocd`, `fluxcd` and `sonarqube` are all wrong (the real slugs are
  `argo`, `flux` and `sonarqubeserver`), and simple-icons carries no Amazon or AWS mark at all, so
  the `assay.aws.*` modules and `assay.s3` intentionally have no icon.

## assay-lua 0.18.12 — 2026-08-14

### Added

- **`assay modules --json`.** The table prints three columns, so a tool that wants the module
  catalogue had to scrape fixed-width text and still lost most of what discovery already parsed.
  `--json` emits the full record per module — name, source, description, keywords, env vars,
  quickref signatures with their return hints, and the auto-extracted function names — under a
  `{ version, modules }` envelope whose `version` is the binary's, so a consumer can cache the
  catalogue and invalidate it on upgrade. Output is unchanged without the flag, and the JSON path
  survives a closed pipe (`| head`) instead of panicking.

### Changed

- **The `assay_context` MCP tool no longer repeats the builtins reference.** Every response
  previously ended with the same ~1,300-character built-in function list, spending roughly 325
  tokens per call on text an agent's harness already carries. Over MCP the block is now opt-in via
  `include_builtins: true`; the `assay context` CLI is unchanged for a human reader and gains
  `--no-builtins` for the same trim. `context::format_context` keeps its behaviour and signature;
  the trimmed rendering is the new `context::format_context_without_builtins`.

### Added

- **`assay.excalidash` — ExcaliDash REST API.** Excalidraw drawings, collections, version-history
  snapshots, and user and link sharing on a self-hosted dashboard. The credential decides the reach:
  an `exd_` API key is exempt from CSRF but the server's scope gate admits it to only four route
  shapes, while a session token reaches everything and pays a one-off `/csrf-token` handshake for
  writes. The client picks per route when it holds both, and refuses a history or sharing call made
  with only an API key rather than relaying the bare 401 or 403 the server would answer. Scene
  writes carry the version they were read at, so a stale edit is refused instead of clobbering;
  `history:restore` guards the same way. A wrong `api_path` is caught too — a dashboard origin
  answers unknown paths with the SPA's HTML and a 200, which would otherwise read as an empty
  dashboard.

## assay-lua 0.18.10 — 2026-08-13

### Added

- **`assay.clickup` rich comments and member resolution.** ClickUp renders comments as Quill rich
  text, so a markdown string posted through `comment_text` arrives with its asterisks and pipes
  intact and a plain `@Name` tags nobody. `clickup.rich()` builds the delta instead — bold, italic,
  code, links, bullet and numbered lines, headings, and `type: tag` mentions — and
  `c.comments:create` takes the builder directly, with an `extra` table for options like
  `notify_all`. `clickup.resolve_member` looks a person up in the workspace roster by username or
  email, since a mention notifies on the numeric id and the visible `@Name` is only a label. The
  bare-string path is unchanged for plain notes.

## assay-lua 0.18.9 — 2026-08-13

### Added

- **`assay.huly` — Huly transactor REST API.** Huly exposes no per-resource endpoints: every read is
  a class-parameterised query and every write is a transaction document, so the client is shaped
  around those two calls plus tracker helpers for projects, issues, milestones, and components.
  Reads unwrap the `TotalArray` envelope and re-inject what the transactor drops — `_class` on
  class-scoped reads, and any attribute the query already pinned to a scalar — because a document
  queried by `identifier` comes back without one. `create_issue` numbers an issue by atomically
  incrementing the project's `sequence`, since an issue written without a `number` and `PREFIX-N`
  identifier is invisible in the UI. Requests ask for identity encoding: Huly's own client asks for
  `snappy, gzip`, and assay can decode neither.

## assay-lua 0.18.8 — 2026-08-13

### Added

- **`assay.plane` — Plane REST API.** Projects, work items, Cycles (Plane models a sprint as a
  Cycle), Modules, states, labels, members, comments, and links, against self-hosted or cloud Plane.
  The workspace slug binds at client construction because it appears in every path, and a blank slug
  errors on first use rather than building `/workspaces//`. `all_items` follows the `next_cursor`
  envelope under a bounded page count, `ensure_item` is idempotent by name, and `resolve_project`
  refuses to guess when a key sees several projects. Comments and links are addressed under
  `/issues/` while work items sit under `/work-items/`, matching Plane's own split.

## assay-lua 0.18.7 — 2026-08-12

### Fixed

- **`assay.clickup` Goals returned the response envelope instead of the goal.** ClickUp wraps single
  goals in a `goal` key, unlike every other v2 resource, so `goals:get`, `goals:create`, and
  `goals:update` handed back `{goal = ...}` and every field read as nil — a `create` looked like it
  had failed while having silently created the goal. The mocked responses asserted the unwrapped
  shape, so the suite stayed green against a shape the API never returns; they now carry the real
  envelope, and `create`/`update` are covered.

## assay-lua 0.18.6 — 2026-08-11

### Added

- **`assay.clickup` — ClickUp REST API.** Tasks, Lists (ClickUp models a sprint as a List), folders,
  spaces, comments, Goals, custom fields, time tracking, and Docs. `all_tasks` follows the
  zero-based `page` pagination to `last_page` under a bounded page count, `ensure_task` is
  idempotent by name, and `resolve_team` refuses to guess when a token sees several workspaces.
- The token travels in `Authorization` with no `Bearer` prefix, which ClickUp rejects on personal
  `pk_` tokens. Docs are the one resource on API v3; everything else is v2.
- Driving ClickUp through this module rather than its hosted MCP server matters for throughput: the
  REST API allows 100 requests per minute, while that MCP endpoint permits 50 calls per 24 hours on
  the Free plan and 300 on paid tiers.

## assay-lua 0.18.5 — 2026-08-11

### Fixed

- **`assay.openstack` image reads failed with HTTP 300 on clouds that publish an unversioned Glance
  endpoint.** The module was inconsistent about which side carried the API version: network methods
  hardcoded `/v2.0` in the request path while image methods hardcoded nothing, so a catalog entry of
  `…/glance` composed `…/glance/images` and Glance answered with its version document instead of a
  listing. The mirror-image bug was latent for network — a versioned `…/neutron/v2.0` catalog entry
  composed `/v2.0/v2.0/networks`. Both services now resolve the endpoint and append the expected
  version only when the URL does not already end in one, so either catalog convention works.
- The shared test catalog published image as already-versioned, which is why no test caught this. It
  now publishes both services unversioned, as real clouds do, and both conventions are covered.
- **Caller-supplied headers replaced runtime-set ones instead of duplicating them.** `opts.headers`
  was applied with `RequestBuilder::header`, which appends, so a caller naming a header the runtime
  also sets — `Content-Type`, for a table body — sent it twice. Strict servers reject that; Keystone
  answers `400 Expecting to find application/json in Content-Type header`. Naming the header
  explicitly is the obvious thing for a module author to do, and it was exactly what broke. Caller
  headers now replace per name, and an invalid header name or value fails loudly at the call rather
  than surfacing as an opaque send error. This repaired eight stdlib modules that pass a table body
  alongside an explicit `Content-Type` — `argocd`, `grafana`, `harbor`, `infoblox`, `neutron`,
  `openclaw`, `github` and `alertmanager` — all of which were sending it twice against every server
  lenient enough not to complain.

### Changed

- `lua/builtins/http.rs` is split into `http/mod.rs` (client) and `http/server.rs` (the
  `server`-gated `http.serve` half), and the two oversized functions inside it are decomposed. The
  file had passed the repo's 1000-line cap, which is what blocked the header fix above.

## assay-lua 0.18.4 — 2026-08-11

### Fixed

- **`assay.openstack` could never authenticate against a real Keystone.** The module named
  `Content-Type: application/json` in its auth request headers, and the runtime already sets that
  header itself when the body is a table. `http.post` appends headers rather than replacing them, so
  the request carried the header twice and Keystone rejected it with
  `400 Expecting to find application/json in Content-Type header`. Every read that needed a fresh
  token failed; only callers supplying a pre-issued token were unaffected. The module now sends
  `Accept` alone and lets the runtime set `Content-Type`.
- The shared Keystone test mock now matches `content-type` by exact value, so a duplicated header
  fails the suite rather than passing a looser match.

## assay-lua 0.18.3 — 2026-08-11

### Added

- **`assay api-serve` runs gated Lua over authenticated HTTP.** `POST /v1/run` and `POST /v1/resume`
  behind a bearer token from `ASSAY_API_TOKENS`, plus an unauthenticated `GET /healthz`. Both return
  the same tool-mode envelope the CLI prints. `mcp-serve` speaks stdio, so a host wanting the
  runtime in a separate trust domain had to invent a protocol over the CLI to get there; this is
  that protocol, in the runtime.
- The server refuses to start when no tokens are configured, rather than serving an ungated runtime.
  Token comparison is constant-time and does not short-circuit across the configured list.
  `unrestricted` mode is refused unless the server opts in, matching `mcp-serve`'s default, and
  `timeout_secs` is clamped rather than trusted.
- Each run executes on its own thread with its own current-thread runtime, because the Lua VM is
  `!Send`; concurrent requests share no VM state.

## assay-lua 0.18.2 — 2026-08-11

### Added

- **An approval grant is now bound to the exact call it was issued for.** Approving an operation
  used to approve its _name_ at a sequence index, so a replay that reached the same index with a
  different URL or body still spent the grant. Each grant now carries a SHA-256 digest over the
  operation and its arguments, and a replay whose request differs fails terminally with
  `approval: ... changed since approval` instead of executing what nobody approved.
- The approval descriptor reports that `digest` plus the **header names** in play, so an approver
  can see which credential a request carries without the value ever entering the descriptor or the
  persisted resume state.

### Changed

- A grant with no digest — resume state written by an earlier version — is refused rather than
  falling back to name-only matching. The check fails closed, so an in-flight resume token issued
  before this release must be re-approved.

## assay-lua 0.18.1 — 2026-08-11

### Added

- **Credential handles let a script authenticate without being able to read the secret.** A policy
  can declare named credentials whose fields resolve from environment keys. `credential.get("name")`
  returns a handle of opaque placeholders; the real values are substituted into the outgoing request
  body and headers by the HTTP layer, after the policy has already decided the target is allowed.
  Printing, concatenating, or encoding a handle yields the placeholder, so a script composes an
  authenticated request without ever holding the secret, and modules that accept
  `username`/`password` need no changes. Pair with an `env.allow` list that excludes the backing
  keys.
- A handle used in a URL is refused rather than substituted — a secret in a request line ends up in
  every access log along the path. Requesting an undeclared credential is an error rather than an
  empty handle.

Substitution is positional, not semantic: a handle placed in an unexpected field is still sent to
whatever host the rules allow, so the `http.rules` allowlist is what bounds the exposure. The
tradeoff is documented in `docs/policy.md`.

## assay-lua 0.18.0 — 2026-08-11

### Added

- **`assay.n8n` — n8n public REST API client.** Covers the whole `/api/v1` surface of n8n 2.x:
  workflows (CRUD, activate/deactivate, publish/unpublish, archive/unarchive, transfer, tags,
  version history), test runs, executions (list/get/delete/retry/stop/stop-all, tags), credentials
  (CRUD, test, type schema, transfer), tags, variables, projects and project members, folders,
  users, source-control pull, security audit, data tables (rows and columns), community packages,
  instance settings (security policy, OTel, SAML), log-streaming destinations, package
  export/import, insights, and discover. Authenticates with `X-N8N-API-KEY`, falling back to the
  `N8N_API_KEY` environment variable.
- Idempotent reconcilers on top of that surface, so a script can be re-run without creating
  duplicates: `ensure_workflow` (identity is the workflow name), `ensure_tag`,
  `ensure_workflow_tags`, `ensure_variable`, `ensure_project`, and `set_active`. Plus `all()` to
  walk every cursor page, `find_workflow_by_name()` for exact-name lookup — the server-side `name`
  filter matches substrings — and `wait()` to block until `/healthz` answers.

## assay-lua 0.17.9 — 2026-08-11

### Added

- **A capability policy confines a run to a declared set of modules, environment keys, and HTTP
  targets.** `ASSAY_POLICY_FILE` (or `create_vm_with_policy` for embedders) loads a YAML file
  declaring which `assay.*` modules may be required, which environment keys `env.get` and `env.list`
  can see, and which hosts, methods, and paths the HTTP builtins may reach. Enforcement lives in the
  runtime, so a script cannot reach past it by building a name at runtime the way it can past a
  caller that inspects source text before running. Policy is orthogonal to execution mode: the mode
  governs writes, the policy governs reach, and both apply together.
- **`classify: read` lets an authentication POST run under read-only mode.** Classifying by verb
  alone marks OpenStack Keystone token issue and Kubernetes bearer-token presign as writes, so
  read-only automation could not reach those services at all and approval mode demanded a human
  decision for what is only a login. A rule may now declare an exact host, method, and path a read;
  everything else on the same host is unaffected.
- Responses can be capped with `http.max_response_bytes` (oversize raises rather than truncating, so
  a clipped body is never mistaken for a complete one) and filtered with `http.redact`, which
  replaces matching JSON keys at any depth and matching response header names with `[redacted]`.
- Unknown keys in a policy file are rejected rather than ignored — a typo in an allowlist that
  silently widened the policy is the failure worth being loud about. Every section is optional and
  absent means unrestricted, so adding a section can only tighten a file.

With no policy loaded every check passes and behaviour is unchanged.

## assay-engine 0.5.12 — 2026-08-08

### Fixed

- A newly created cron schedule no longer starts one run the moment it is registered. Scheduling a
  workflow for a date months away now waits for that date.

## assay-workflow 0.4.4 — 2026-08-08

### Fixed

- `create_schedule` seeds `next_run_at` from the cron expression on both backends. The scheduler
  reads a NULL `next_run_at` as due-now, so an unseeded schedule fired one run at registration
  regardless of its cron. Schedules whose fire time has already elapsed still catch up, and an
  explicitly supplied `next_run_at` is preserved.
- `POST /schedules` returns the stored record, so its `next_run_at` reflects the seeded value
  instead of `null`.

### Added

- `scheduler::evaluate_schedules_at(store, now)` — one deterministic scheduler pass at an injected
  `now`, for tests that would otherwise wait on the 15s poll interval.

## assay-engine 0.5.11 — 2026-08-04

### Added

- **Operators can resume a run after correcting an external activity blocker.** The authenticated
  workflow API, embedded dashboard, CLI, and Lua management client can atomically requeue the
  terminal failed activity with a required requester and reason. Replay preserves earlier results,
  invalidates activity results produced by the failure-handling branch, and records the retry in
  immutable workflow history.
- Retry requests reject non-failed, archived, and child workflows and cannot create duplicate work
  when submitted concurrently. Automatic activity retries remain bounded by the activity policy;
  terminal recovery remains an explicit operator decision.

## assay-workflow 0.4.3 — 2026-08-04

### Added

- Added backend-neutral terminal failed-activity retry, native atomic PostgreSQL and SQLite
  implementations, REST/OpenAPI support, replay-boundary handling, and retry-request bus events.

## assay-domain 0.2.3 — 2026-08-04

### Added

- Added the source-compatible `WorkflowStore::retry_failed_activity` extension point and its typed
  outcomes. Third-party stores default to an unsupported result until they implement the operation.

## assay-dashboard 0.4.5 — 2026-08-04

### Added

- Failed workflow rows now offer an audited Retry action that collects the operator identity and
  reason before requeuing the failed activity.

## assay-lua 0.17.8 — 2026-08-04

### Added

- Added `assay workflow retry` and `workflow:retry_failed_activity(...)` management clients.
- Durable replay now clears cached activity outcomes at the recorded retry boundary.

## assay-engine 0.5.10 — 2026-08-02

### Added

- **Verified users can recover a forgotten password by email.** Deployments can enable an SMTP
  recovery flow with a 15-minute, single-use token. Only the SHA-256 token digest is stored, the raw
  token remains in the browser URL fragment, successful completion revokes existing sessions, and
  public request responses do not expose account existence or SMTP latency.
- The embedded login page links to a dedicated recovery page that requests an email or accepts a
  reset token without placing that token in request URLs or browser history.
- Public deployments can independently mount browser authentication assets and operator consoles,
  allowlist accepted hostnames, and keep health checks reachable for infrastructure probes. The
  first-party service uses this boundary to show a clean auth landing page while withholding every
  operator console and rejecting ordinary requests to its Fly hostname.

## assay-auth 0.6.2 — 2026-08-02

### Added

- Added backend-neutral password-recovery contracts, SMTP delivery, and native PostgreSQL and SQLite
  stores for hashed, expiring, single-use recovery tokens.

## assay-dashboard 0.4.4 — 2026-08-02

### Added

- Added the public password-recovery request and completion interface to the embedded auth assets.
- Split public sign-in/recovery assets from operator auth-console assets and added a minimal public
  Assay Auth landing page.

## assay-engine 0.5.9 — 2026-08-01

### Fixed

- **OIDC consumers can finish sign-in through UserInfo.** Provider access tokens now verify with
  their signed consumer client audience instead of the engine's static API audience. The provider
  still requires the configured issuer, signature, expiry, access-token purpose, and matching `aud`
  / `client_id`. JWT access-token introspection uses the same rules.

## assay-auth 0.6.1 — 2026-08-01

### Fixed

- Added provider-scoped JWT verification for dynamic OIDC client audiences while preserving the
  existing static-audience verifier for every other auth surface.

## assay-engine 0.5.8 — 2026-08-01

### Fixed

- **Fresh installations can sign in without an upstream identity provider.** The embedded auth
  landing now always offers first-party email/password login and treats configured OIDC providers as
  optional alternatives. Successful login resumes only a same-origin authorization URL, while
  invalid credentials leave the form usable and clear the submitted password.

## assay-engine 0.5.7 — 2026-08-01

### Added

- **One engine can expose a dedicated authentication origin.** The optional `auth.public_url`
  setting controls the default OIDC issuer, federation callback base, and passkey origin while
  `server.public_url` remains the canonical workflow, vault, and dashboard URL. Deployments can
  therefore present stable auth and engine hostnames from the same process and database without
  adding a gateway or duplicating the identity plane.
- **The first-party Assay service is deployable from this repository.** The new flat `service/`
  package runs the complete released engine on Fly.io against externally managed PostgreSQL, scales
  to zero while idle, and verifies the public engine health and OIDC discovery surfaces after each
  successful engine release.

## assay-engine 0.5.6 — 2026-07-28

### Added

- **Workflow event history supports bounded cursor pages.** Existing unqueried
  `GET /api/v1/engine/workflow/workflows/{id}/events` calls retain the full ascending array, while
  operators can request `limit`, an exclusive sequence `cursor`, and `order=asc|desc`. Native
  PostgreSQL and SQLite stores apply the bound in SQL, with page sizes capped at 1,000 events. This
  lets operator consoles expose recent durable execution history without loading an unbounded run
  into an API response or browser.

### Fixed

- **Standalone builds now include S3 workflow archival.** The engine documented the
  `ASSAY_ARCHIVE_*` runtime settings, but its published default binary did not forward the workflow
  crate's build feature, so those settings could never start the archiver. Default `assay-engine`
  builds now include archival support while keeping it runtime-disabled until
  `ASSAY_ARCHIVE_S3_BUCKET` is configured. Custom embedders can still omit the AWS dependencies by
  disabling default features and selecting their required features explicitly.

## assay-workflow 0.4.2 — 2026-07-28

### Added

- Added bounded, cursor-based workflow event pages to the REST API and both native stores. The
  original full-history store and HTTP methods remain compatible for replay and existing clients.

## assay-domain 0.2.2 — 2026-07-28

### Added

- Added the provided `WorkflowStore::list_events_page` method. Third-party stores retain source
  compatibility through the default bounded implementation; PostgreSQL and SQLite override it with
  native queries.

## assay 0.17.7 — 2026-07-16

### Added

- **`assay.openstack` — Keystone-authenticated OpenStack inventory.** A GET-only client for identity
  projects/users/regions, detailed compute servers and quotas, images, networks, subnets, ports,
  routers, security groups, and network quotas. It supports Keystone v3 project-scoped password
  authentication, existing tokens, service-catalog endpoint selection by region/interface, and
  explicit endpoint overrides. Password authentication continues to pass through the existing
  `http.post` gate, while pre-issued-token inventory runs entirely in readonly mode.

## assay-workflow 0.4.1 — 2026-07-10

### Fixed

- **Heartbeat-timeout reaper re-queues instead of wedging the run.** When a RUNNING activity's
  heartbeat expired with attempts remaining, the health monitor terminally FAILED it — no requeue,
  no `ActivityFailed` event, no dispatch wake — so the parent workflow sat RUNNING forever (and with
  `overlap_policy: "skip"`, every subsequent fire of that schedule was skipped). The reaper now
  re-queues with the same exponential backoff `/fail` uses; the exhausted path still terminally
  fails the activity and the workflow.
- **Claims that die before their first heartbeat now time out.** `get_timed_out_activities` required
  `last_heartbeat IS NOT NULL`, so a worker that crashed between claim and first beat was invisible
  to the reaper. The claim time (`started_at`) is now the baseline until a heartbeat arrives.
- New `health::check_health_at(store, now)` seam so the reaper is integration-testable
  deterministically; regression tests pin all three paths on both backends.

## assay-engine 0.5.5 — 2026-07-10

### Fixed

- Rebuild on assay-workflow 0.4.1 — heartbeat-timeout reaper re-queues retryable activities and
  catches never-heartbeated claims (see above).

## assay 0.17.6 — 2026-07-08

### Added

- **`assay.neutron` — Neutron instance admin client.** The Neutron agent platform's full admin REST
  API from Lua: agents (personas, tool policies, guardrails, baked assay modules), secrets and
  git-host connections (both with per-agent scoping), workspace/guide resources, roles, instance
  settings, API tokens, and usage. Env-driven auth (`NEUTRON_URL` / `NEUTRON_TOKEN`, optional
  Cloudflare Access service token), with a first-boot bootstrap-token takeover flow for freshly
  provisioned instances. One client per instance; manage a fleet by creating several.

## assay 0.17.5 — 2026-07-07

### Fixed

- **`assay_resume` requires an explicit `approve` decision.** The MCP tool previously defaulted an
  omitted `approve` to `true` — the authorization step for a suspended mutating operation could be
  taken by accident. The input schema has always declared the field required; the lenient handler
  was the bug, and omitting `approve` is now an invalid-arguments error, never an approval. (#195)
- **Approval grants are bound to the operation they approved.** A resume grant used to be matched by
  sequence index alone: because a resume re-runs the script from the top, a run whose control flow
  shifted between suspend and replay (e.g. a read that returned a different value the second time)
  could spend the grant on a _different_ mutating operation that landed on the same index. Each
  grant now records the op it was issued for and the gate refuses terminally —
  `approval:
  operation at index 0 changed since approval (approved 'http.post', got 'http.put')` —
  instead of executing an operation nobody approved. Grants travel to the re-run via the new
  `ASSAY_APPROVED_OPS` environment variable (a JSON array of `{index, op, approver?}` records) and
  accumulate across the approval chain in the resume state. The gate is fail-closed: an approved
  index with no op binding is refused outright, so index-only grants — including resume state
  written by earlier versions — are never honored. (#196)

### Added

- **`approver` audit identity on resumes.** `assay_resume` (MCP) and `assay resume` (CLI,
  `--approver`) accept an optional approver identity. It is recorded in the resume state's grant
  records, logged with the resume decision, and echoed as a top-level `approver` field in the result
  envelope. Intended for orchestrators that interpose a human decision between suspend and resume
  and want the authorizer on the record. (#195)

## assay 0.17.4 — 2026-07-07

### Added

- **`assay.k8s` is multi-cluster: kubeconfig context support.** Any k8s call takes `opts.context`
  (or set a default with `k8s.use_context(name)` / `ASSAY_K8S_CONTEXT`), resolved from
  `opts.kubeconfig` / `$KUBECONFIG` / `~/.kube/config`: the cluster's server URL and CA
  (`certificate-authority-data` honored via a per-context HTTP client), and the user's auth.
  Supported user auth: a static `token`, or the aws `eks get-token` exec plugin — which is
  **recognized and minted in-process** (with `--role-arn`/`--profile`/exec-`env` respected) rather
  than executed, so contexts work in readonly mode where subprocesses are blocked. `k8s.contexts()`
  lists what's available. Fully backward compatible: with no context configured, calls target the
  in-cluster ServiceAccount exactly as before.
- **`assay.aws.sts` — the AWS credential chain, in-process.** `sts.credentials(opts)` resolves like
  the AWS CLI: explicit keys → `opts.profile`/`AWS_PROFILE` (parsing `~/.aws/config` +
  `~/.aws/credentials`, following `role_arn` + `source_profile` chains and
  `web_identity_token_file`) → env keys → IRSA (`AWS_ROLE_ARN`+`AWS_WEB_IDENTITY_TOKEN_FILE`). Plus
  `sts.assume_role(role_arn, opts)` (SigV4-signed) and
  `sts.assume_role_with_web_identity(role_arn, token, opts)`. Temporary credentials are cached until
  shortly before expiry. STS is asked for JSON (Accept header), so no XML parsing is involved.
- **`assay.aws.eks` — EKS bearer tokens without the aws CLI.** `eks.get_token(cluster)` mints the
  `k8s-aws-v1.` presigned-STS token in-process (what `aws eks get-token` produces), honoring
  profile/role/region options. This is what powers the k8s exec-plugin contexts above.
- **`sigv4.presign(opts)`** — query-string SigV4 signing (presigned URLs), alongside the existing
  header signing; both now accept `opts.time` for deterministic signatures in tests.
- **`aws.ec2` / `aws.s3` / `aws.ecr` clients resolve credentials automatically.** `client(opts)` no
  longer hard-requires literal keys: omit them and the standard chain runs (honoring `opts.profile`
  and `opts.role_arn`); `region` falls back to `AWS_REGION`/`AWS_DEFAULT_REGION`. Explicit keys keep
  working unchanged.

## assay 0.17.3 — 2026-07-07

### Added

- **`assay_resume` MCP tool — the suspend→approve→resume loop now lives entirely in the MCP API.**
  `assay_run(mode: "approval")` already returns a `needs_approval` envelope with a `resumeToken`
  when a mutating operation suspends; the new `assay_resume` tool takes that token plus
  `approve: true|false` and returns the next envelope (`ok` on completion, `needs_approval` again
  with a fresh token if the run suspends on the next operation, or `error`). Previously resuming was
  only possible via the `assay resume` CLI subcommand, so an MCP host had to shell out; now a host
  can drive the whole gated flow over the API and interpose its own approver between run and resume.
  Internally, `resume_tool_execution` is refactored to a shared `resume_tool_outcome` that returns
  the envelope (mirroring `execute_tool_mode`), with the CLI now a thin wrapper that prints it.
- **`ASSAY_MCP_UNRESTRICTED` — opt-in exposure of the third execution mode over MCP.** By default
  `assay mcp-serve` still advertises and accepts only `readonly` + `approval`, so every MCP client
  stays safe by default and can never fall through to unrestricted execution. When the server is
  started with `ASSAY_MCP_UNRESTRICTED=1` (or `true`), the `assay_run` tool additionally offers and
  accepts `mode: "unrestricted"` (the always-existing `ExecMode::Unrestricted`, previously reachable
  only from the CLI). Intended for a host that gates access itself — resolving the caller's allowed
  mode from its own policy before ever passing `unrestricted` — rather than trusting the model. The
  advertised `mode` enum and the tool description track the flag, so a client only ever sees a mode
  it can actually request.

## assay 0.17.2 — 2026-07-06

### Added

- `k8s.pods:exec(namespace, pod, command, opts?)` — run a command inside a pod over the Kubernetes
  streaming exec endpoint and collect the result as `{stdout, stderr, exit_code}`. It opens a
  WebSocket with the `v4.channel.k8s.io` subprotocol and a bearer token, demultiplexes stdout
  (channel 1) and stderr (channel 2), and reads the exit code from the `v1.Status` the API server
  sends on the error channel (channel 3) at process exit. `opts`:
  `{container, stdin, tty, timeout_secs, token, base_url, insecure}`. Because it opens its stream
  via the already-gated `ws.connect`, read-only mode blocks it and approval mode suspends it with no
  change to the shared mutation catalog.
- `ws.connect(url, opts?)` gains an optional second argument for the handshake, keeping
  `ws.connect(url)` unchanged:
  - `opts.subprotocols` — array of strings offered via `Sec-WebSocket-Protocol`; read the negotiated
    protocol back with the new `ws.protocol(conn)`.
  - `opts.headers` — extra request headers on the upgrade (e.g. `Authorization: Bearer …`).
  - `opts.insecure` — skip TLS certificate verification for the `wss://` handshake.
- `ws.send_binary(conn, bytes)` — send a binary frame from a binary-safe Lua string of arbitrary
  bytes. `ws.recv` now returns binary frames as raw bytes instead of erroring on non-UTF-8 payloads,
  so channel-prefixed protocols round-trip losslessly.

## assay 0.17.1 — 2026-07-05

### Added

- plugin: ship a Claude Code plugin (`plugin/`) + marketplace (`.claude-plugin/marketplace.json`)
  that wrap the MCP server — `claude plugin marketplace add developerinlondon/assay` then
  `claude plugin install assay` gives the gated `assay_run` + `assay_context` tools and a usage
  skill. Requires the `assay` binary on PATH.
- build: track the embedded `stdlib/` tree via `build.rs` `rerun-if-changed` so a stdlib-only change
  is never served from a stale build cache (persistent CI caches previously missed newly added
  modules).
- Five API-client stdlib modules over the existing `http` and `aws.sigv4` builtins, extending the
  batteries-included set to more infrastructure and cloud HTTP APIs:
  - `assay.sonarqube` — SonarQube web API reads: quality-gate project status, issue and hotspot
    search, component measures, project search. Bearer or basic auth. Read-only.
  - `assay.servicenow` — ServiceNow Table API (`list` / `get` / `create` / `update`) plus a
    `cmdb:query` helper. Basic or bearer auth. `create` / `update` mutate; `cmdb:query` is a lookup
    that uses `POST` by API contract.
  - `assay.infoblox` — Infoblox WAPI: DNS record / network / range reads and grid status, plus
    record create / update / delete. Basic auth. Documents that the Grid CA must be trusted by the
    runtime because the `http` builtin exposes no TLS-skip option.
  - `assay.aws.ec2` — EC2 Query API reads over Signature V4: `describe_instances`,
    `describe_volumes`, `describe_security_groups`. Read-only.
  - `assay.aws.s3` — S3 reads over Signature V4: `list_buckets`, `list_objects`, `head_object`.
    Read-only.

  Every mutating method routes exclusively through the already-gated `http.post` / `put` / `patch` /
  `delete` verbs, so read-only and approval modes classify them as mutations with no change to the
  shared mutation catalog. Because that gate is verb-based, `servicenow.cmdb:query` (a `POST` that
  reads) is also suspended in gated modes.

## assay 0.17.0 — 2026-07-05

### Added

- `assay mcp-serve` — a Model Context Protocol server over stdio so AI coding agents (Claude Code,
  Cursor, Windsurf, Cline, and any MCP client) can drive the runtime directly. It speaks JSON-RPC
  2.0 over stdin/stdout with newline-delimited framing per the MCP stdio transport, implementing
  `initialize`, `tools/list`, `tools/call`, `ping`, and clean shutdown on EOF. Rather than exposing
  one tool per module, it presents exactly two tools and lets Lua compose the modules — the
  advertised schema stays tiny no matter how many modules ship:
  - `assay_run` — execute a Lua script and return the tool-mode JSON envelope (`status` ok /
    needs_approval / error, `output`, `requiresApproval` + resume token, `truncated`, `readonly`).
    Reuses the existing tool-mode execution path, so approval gates suspend and return a resume
    token exactly as `assay run --mode tool` does. Execution is **always gated**: the `mode`
    argument accepts only `readonly` (default) or `approval`; an unrestricted mode is intentionally
    not offered and any other value is rejected, so a caller can never opt out of the gate. A
    blocked write in read-only mode surfaces as an `error` envelope rather than crashing the server.
  - `assay_context` — search the embedded modules and return prompt-ready Markdown (method
    signatures, env vars, builtins), the same output as the `assay context` CLI, so an agent can
    discover module APIs before composing a script.
  - Tool-execution failures return an MCP result with `isError: true` (`needs_approval` is not an
    error); malformed JSON-RPC returns a transport-level error response. Transport is hand-rolled on
    `serde_json` + `tokio` (both already dependencies) to keep the static binary size flat — no MCP
    SDK is pulled in.

## assay 0.16.10 — 2026-07-05

### Added

- Enforced approval mode for supervised script contexts (agent-generated remediation, self-service
  automation), where each mutating operation is authorized individually by an operator or calling
  system rather than trusted to the script. Activate with the global `--approval-mode` CLI flag
  (works for `run`, `exec`, YAML check mode, and tool mode) or `ASSAY_APPROVAL=1`/`true`. When both
  read-only and approval mode are requested, approval mode wins (ask, don't hard-block). Instead of
  executing, a mutating builtin suspends the run and raises the existing tool-mode approval flow
  (`status:"needs_approval"` + resume token) carrying a descriptor of the operation — `op` (the
  dotted builtin name), `summary` (the salient argument: url for HTTP, path for filesystem, command
  for shell, SQL for `db.execute`), and `index` (a per-run sequence number assigned to every gated
  operation). The supervisor decides with `assay resume --token … --approve yes|no`:
  - `yes` re-runs the script from the top with that one operation additionally permitted;
    previously-approved operations execute and the run re-suspends at the next unapproved one, so a
    two-write script takes two approval cycles before it completes. Grants are single-shot and
    per-operation, never "unlock the rest of the run".
  - `no` fails that operation terminally with `approval: <op> denied`, a clean error the script's
    own error handling can observe. The gated surface is the same catalog read-only mode uses (now
    shared between the two gates): HTTP write verbs (including `http.client(...)` wrappers),
    `ws.connect`, all of `shell.*` / `process.*` / `machinectl.*`, `fs` write ops, `env.set`,
    `db.execute`, `oci`/`systemd`/`apt`/`tar`/`compress` mutators, and the Lua stdlib write paths
    (`io.popen`, `io.open` write modes, `io.output(target)`). Read paths (`http.get`, `fs.read`,
    `env.get`, `db.query`, status/list helpers) run freely without prompting. `assay modules` notes
    when the mode is active. Because approval matching is by sequence index, a read that changes
    control flow between operations across re-runs can shift indices (the same class of limitation
    as workflow replay); acceptable for supervised single-writer scripts.

## assay 0.16.9 — 2026-07-05

### Added

- Read-only execution mode for semi-trusted script contexts (agent-generated scripts, review
  pipelines, dry-run diagnostics). Activate with the global `--readonly` CLI flag (works for `run`,
  `exec`, YAML check mode, and tool mode) or `ASSAY_READONLY=1`/`true`. Mutating builtins stay
  registered but raise `readonly: <name> blocked` errors (suffixed with "write operations are
  disabled in read-only mode") instead of executing: HTTP write verbs
  (`http.post/put/patch/delete/serve/download`, including `http.client(...)` wrappers),
  `ws.connect`, all of `shell.*`, `process.*`, and `machinectl.*`, `fs` write ops (`write`,
  `write_bytes`, `remove`, `rename`, `copy`, `chmod`, `mkdir`, `tempdir`, `sub_in_file`), `env.set`,
  `db.execute`, `oci` mutators, `systemd` unit and machine lifecycle actions, `apt` mutators,
  `tar.create`/`tar.extract`/`compress.untar`, and the Lua stdlib write paths (`io.popen`, `io.open`
  write modes, `io.output(target)`). Read paths (`http.get`, `fs.read`, `env.get`, `db.query`,
  `systemd.list_units`, journal/status/list helpers) work unchanged. `assay modules` notes when the
  mode is active, and tool-mode JSON envelopes carry `"readonly": true` (omitted when off).

## assay-vault 0.4.2 — 2026-06-28

### Changed

- Rebuild against `assay-auth 0.6` (no code change). assay-auth's 0.6.0 major bump means the
  previously-published assay-vault 0.4.1 (which pinned `assay-auth 0.5`) would pull a second
  `assay-auth` into the crates.io dependency graph when `assay-engine` is published, breaking the
  `cargo publish --verify` build (`FromRef` resolves across two `assay_auth` versions). Bumping
  assay-vault so it republishes with the `assay-auth 0.6` requirement keeps a single `assay-auth` in
  the graph.

## assay-engine 0.5.3 — 2026-06-28

### Fixed

- Upstream-OIDC ("Continue with Google") logins no longer fail intermittently with
  `id_token verify: Signature verification failed` after the upstream rotates its signing keys. The
  federation client cached the upstream JWKS once at discovery and pinned it forever, so once Google
  rotated keys, id_tokens signed by a new `kid` could no longer be verified. The client now
  refreshes the JWKS — proactively when the certs endpoint's `Cache-Control: max-age` lapses, and
  reactively (rate-limited) when a token arrives with a `kid` not in the cache. (Carries
  `assay-auth 0.6.0`.)

## assay-auth 0.6.0 — 2026-06-28

### Fixed

- `oidc::OidcClient` keeps a refreshable cache of each upstream's signing keys instead of pinning
  the set fetched at discovery. The id_token verifier is rebuilt from the live cache, which
  re-fetches from `jwks_uri` on TTL expiry (honoring `Cache-Control: max-age`, clamped) and on an
  unknown `kid` (`SignatureVerificationError::NoMatchingKey`), with a minimum interval between
  reactive refetches.

### Changed

- **Breaking (auto-trait):** `OidcClient` no longer implements `UnwindSafe` / `RefUnwindSafe` — it
  now holds an `Arc<RwLock<…>>` JWKS cache and a `reqwest::Client` for refreshes. `Send` + `Sync`
  are unaffected. Bumped to 0.6.0 per SemVer; dependents (`assay-engine`, `assay-vault`) updated to
  require `assay-auth 0.6`.

## assay-engine 0.5.2 — 2026-06-03

### Fixed

- Zanzibar permission resolution no longer mis-flags a "diamond" — a permission reaching the same
  relation via two union branches (e.g. `view = up + down` where both include `account`) — as a
  schema cycle. Such permissions previously resolved to `CycleDetected` and silently denied; they
  now evaluate correctly. (Carries `assay-auth 0.5.1`.)
- Biscuit share-link verify no longer intermittently times out under load — the datalog `query` now
  uses the same 10s `max_time` backstop as `authorize`. (Carries `assay-vault 0.4.1`.)

## assay-auth 0.5.1 — 2026-06-03

### Fixed

- `zanzibar::resolve` tracks the current resolution _path_ for cycle detection instead of a
  cumulative visited-set, so a relation reachable via two union branches is no longer a false
  positive. Genuine self-referential schema cycles are still rejected.

## assay-vault 0.4.1 — 2026-06-03

### Fixed

- Share-link `verify` bounds the biscuit datalog `query` with a 10s `max_time` (was biscuit's 1ms
  default), matching the `authorize` backstop — a loaded host no longer hits `RunLimit(Timeout)`.

## assay-engine 0.5.1 — 2026-06-03

### Added

- Zanzibar `/check` resolves computed-userset arrows (`relation->permission`) plus `intersect` /
  `exclude` operators via a recursive evaluator (MAX_DEPTH 50, cycle-safe). Previously stubbed.

### Fixed

- Biscuit authorizer `max_time` 1 ms → 10 s — token checks no longer abort with "Reached Datalog
  execution limits" under load (a spurious `Forbidden`).

## assay 0.16.8 — 2026-06-03

### Added

- `assay.postgres.client_url(url)` creates a stdlib Postgres client from a full connection URL.

### Fixed

- `db.query()` now decodes Postgres `numeric`/`decimal` values as exact Lua strings instead of
  failing through sqlx's `Any` driver.

## assay 0.16.7 — 2026-05-26

### Added

- `yaml.parse_all(str)` parses multi-document YAML streams into a Lua array, skipping empty
  documents. This removes per-script YAML stream splitting for Helm/Kubernetes render checks.

## sysops 0.2.0 — 2026-05-20

### Added

- **Auth gateway** — sysops can now front the engine as a BFF that terminates OIDC, holds the admin
  bearer server-side, and injects it into proxied requests. Opt in via four new `mount` opts: `oidc`
  (issuer, client_id, redirect_uri, scopes), `session` (signing_key, ttl_seconds, cookie_name),
  `gateway` (engine_upstream, admin_bearer), and `authz` (require_zanzibar_admin,
  bootstrap_first_admin). Consumers that don't pass `oidc` are bit-for-bit unchanged.
- New page handlers: `/auth/login`, `/auth/callback`, `/auth/logout` (OIDC Authorization Code +
  PKCE; HttpOnly session cookie; ephemeral in-process store for refresh tokens).
- `gateway.whoami` intercepts `GET /api/v1/engine/auth/whoami` and answers from the session cookie —
  defuses the assay-dashboard auth/engine SPA token banners without modifying SPA code.
- `gateway.proxy` is a dual-mode reverse proxy on `/api/v1/engine/*` + dashboard SPA asset paths.
  Caller's `Authorization: Bearer` is passed through unchanged (preserves SSH+curl, CI scripts,
  customer-IdP JWT direct calls); session-only callers get the configured admin bearer + X-User-Id
  injected. Hop-by-hop headers (Connection, Transfer-Encoding, Cookie, …) stripped both directions.
- First-user-wins admin bootstrap: the first OIDC login on a fresh deployment auto-grants the
  `engine:core#admin` Zanzibar tuple if no admins exist. Opt out via
  `authz.bootstrap_first_admin = false`.
- `require_session` middleware for gating sysops's own `/auth/*` and `/vault/*` pages on the OIDC
  session cookie. No-op pass-through when the auth gateway isn't wired.
- `libs/sysops/codec.lua` — shared `b64url`/`hex_to_bytes`/`must`/`consteq` helpers.
- `libs/sysops/session.lua` — HMAC-SHA256-signed cookies (compact JWT-like format, since
  `crypto.jwt_sign` is RSA-only) + in-memory session store with one-shot pending-state GC.

### Changed

- `ctx.lua` extended with `oidc_client`, `session_signer`, `session_store`, `gateway_admin_bearer`,
  `authz_require_admin`, `authz_bootstrap_first_admin`, `zanzibar_check`. All nil-defaulted;
  backward-compatible.

## assay 0.16.6 — 2026-05-22

### Fixed

- `assay.postgres.client` percent-encodes username + password in the DSN. Passwords with
  `?`/`/`/`#`/`@` used to break sqlx URL parsing.

## assay 0.16.5 — 2026-05-21

### Fixed

- Empty-array defaults in SDK payloads now serialise as `[]` (were `{}`, which upstreams reject as
  type mismatch). Wrapped in `json.array()`: `ory.hydra` `consent:accept`
  `grant_access_token_audience`, `tailscale` `mint_key` `tags`, `engine.auth` `passkey:start_auth`
  `passkeys`.

## assay 0.16.4 — 2026-05-19

### Fixed

- `assay.ory.keto` filter keys for `subject_set` now use dotted notation (`subject_set.namespace`,
  …) to match Keto's API. The previous underscored keys (`subject_set_namespace`, …) were silently
  ignored on GET and rejected with HTTP 400 on DELETE, breaking `tuples:upsert` for any subject_set
  tuples (e.g. parent edges in HRBAC seeds).

## assay 0.16.3 — 2026-05-19

### Added

- `c.tuples:upsert(tuple)` — idempotent ensure-exactly-one. Returns `"noop"` / `"created"` /
  `"repaired", N`. Use for seed scripts; `tuples:create` is non-idempotent.

### Changed

- `c.tuples:create` docstring flags non-idempotency, points at `upsert`.

## sysops 0.1.6 — 2026-05-18

### Added

- Host Services now shows a compact per-`.service` stats table with sortable memory and CPU-usage
  columns. Clicking a service expands a systemd detail panel with unit file state/path, main PID,
  exec command, restart policy, and related accounting fields.
- Host Services adds `.service`-only start, stop, and restart actions. POST handlers validate the
  unit name, call the allow-listed systemd lifecycle path, and redirect back to the current filtered
  services view with a success or error banner.
- `libs/sysops:smoke` now runs a focused service-unit helper contract test before the broader page
  smoke and vault tests.

## assay 0.16.2 — 2026-05-12

### Fixed

- `:<version>-sh` image: ship the full busybox applet set, not just `/bin/sh`; move `assay` to
  `/usr/local/bin/assay` so it resolves on `$PATH`.

## assay 0.16.1 — 2026-05-11

### Fixed

- `rauthy.client_presets.immich`: register `challenges = ["S256"]`. Current Immich web sends PKCE on
  every authorize; Rauthy was rejecting it as a spec mismatch.

### Container image

- New `ghcr.io/developerinlondon/assay:<version>-sh` tag — same scratch image as the plain
  `:<version>` tag, plus a 1 MB static busybox at `/bin/sh`. Use in environments that wrap the
  launch in `sh -c` (notably GitLab CI's docker executor). K8s consumers that already use
  `command: ["/assay", ...]` should keep the plain `:<version>` tag.

## assay 0.16.0 — 2026-05-11

### 🏗️ Container Registry (lite crane replacement)

- `oci.copy(src, dst, opts?)` — copy images between registries
- `oci.tag(src, tag)` — tag an existing image
- `oci.mutate(src, dst, files)` — add files as new OCI layers
- All use `oci-distribution` under the hood, no external binary needed

### 📦 Tar archive support

- `tar.create(output, files, opts?)` — create tar/tar.gz from `{path = content}`
- `tar.extract(archive, dest)` — extract to directory
- `tar.list(archive)` — list contents

### ☁️ AWS ECR support

- `assay.aws.ecr` — `client():get_authorization_token()` returns ECR auth tokens
- `assay.aws.sigv4` — reusable AWS Signature V4 signing for any service

### Internal

- Added `oci-distribution` crate dependency
- Binary size remains ~8.7MB (shared deps with existing reqwest/tokio)

## [assay-vault 0.3.0] — 2026-05-11

- Pin `assay-auth = "0.4"` (was `"0.3"`); republish to unblock `assay-engine` publish.
- `assay-engine` `assay-vault` dep `"0.2"` → `"0.3"` to match.
- Same pattern as `assay-vault 0.1.0 → 0.2.0` in #99.

## assay 0.15.11 — 2026-05-11

### Added

- `assay.rauthy` — Rauthy IdP admin client. `c.sys:health()`, `c.discovery:{config,jwks}`, full
  `c.clients` lifecycle (`list`/`get`/`create`/`put`/`delete`/`rebuild`/`rotate_secret`) plus
  idempotent `c.clients:reconcile(payload)`.
- `reconcile` rotates secrets only on real drift: 404 → create+rotate; `challenges` missing →
  rebuild+rotate (workaround for a Rauthy 0.35 cache quirk); other drift → put-only; no drift →
  noop.
- `rauthy.client_presets.openbao({host})` / `.argocd({host})` — ready-made client payloads with each
  consumer's OIDC verifier quirks baked in.

### Known limits

- Provider config (e.g. Google federation) not exposed — Rauthy 0.35's `/providers/*` is
  admin-session-only, no API-key path. Upstream feature request to follow.

## sysops 0.1.5 — 2026-05-08

### Added

- Opt-in `active_modules` mount option. Pass `active_modules = { "auth", "vault" }` to
  `sysops.mount(routes, opts)` to enable the new in-sysops auth + vault pages. Without the opt-in,
  sysops 0.1.5 is byte-identical in behaviour to 0.1.4 — same sidebar, same routes.
- `sysops.vault` SDK aggregator. `local v = require("sysops.vault").new(engine)` returns a table
  with sub-clients for KV, transit, sealing, dynamic credentials, share tokens, collections, and the
  bitwarden-compatible personal vault — pure-Lua HTTP wrappers around `/api/v1/vault/*`. Existing
  `vault.secret_store(opts)` keeps working for legacy 0.1.4 callers.
- `sysops.auth` SDK aggregator. `local a = require("sysops.auth").new(engine)` returns sub-clients
  for session (login / logout / whoami / passkey), users, sessions, OIDC clients, upstreams, JWKS,
  biscuit, audit, and zanzibar (check / expand / tuples).
- Auth + Zanzibar dashboard pages under `/auth/users`, `/auth/sessions`, `/auth/oidc-clients`,
  `/auth/upstreams`, `/auth/jwks`, `/auth/biscuit`, `/auth/audit`, `/zanzibar`, `/zanzibar/tuples`,
  `/zanzibar/check` — Lua-rendered, mirror the knowhere-pkg layout (page-header eyebrow + in-page
  tab strip + cards). Gated on `active_modules` containing `"auth"`.
- Vault dashboard pages under `/vault` (overview), `/vault/kv`, `/vault/transit`, `/vault/sealing`,
  `/vault/dynamic`, `/vault/share`, `/vault/collections`, `/vault/me`. Gated on `active_modules`
  containing `"vault"`.
- New optional mount opt: `engine_admin_key` (bearer token used by the SDK for admin-scoped engine
  calls).

### Changed

- `templates/layout.html` adds two conditional `<nav>` blocks for Auth and Vault. Existing Host /
  Networks / Engine / Admin sidebar entries are unchanged in text, position, and CSS.

### References

- Plan: `.claude/plans/25-v0.1.5-sysops-auth-vault-pages.md`
- Revisits plan 22's "Engine link only, no Lua port" decision — opt-in only, same default behaviour.

## sysops 0.1.4 — 2026-05-07

### Added

- `sysops.vault` shared Assay Engine Vault integration for host-manager apps. The
  `vault.secret_store(opts)` factory returns the `read/write/delete/available` service shape that
  sysops backup flows consume, stores host/app operational secrets in engine KV v2, and preserves
  read-only fallback from existing rustic/local secret files.

### Changed

- `libs/sysops:smoke` now runs the vault adapter contract test alongside the host dashboard smoke
  test.

## 0.15.10 — 2026-05-05

- Fix: `json.encode({})` now returns `"{}"` (was `"[]"`). Same fix applies to every empty Lua table
  passed as a JSON body via the http builtin. Closes #129.
- Add: `json.array(t?)` / `json.object(t?)` to pin a table's encoded shape.
- Migration: callers that need `"[]"` for an empty table must use `json.array(t)`.

## hostops 0.1.3 — 2026-05-04

### Added

- `hostops.mount` `extra_sidebar_links` accepts a grouped entry shape in addition to the flat one. A
  grouped entry (`{label, children = { {href, label, nav_active}, ... }}`) renders as a
  `<details data-section><summary>label</summary>children</details>` block in the sidebar. The
  existing `app.js` disclosure script persists open state in localStorage; children sit one level
  indented and pick up the base `.nav a` typography.
- Mutex sidebar highlight: clicking a group summary moves the active treatment (background + accent
  left bar) onto the summary while clearing `.active` from any other sidebar link, so only one entry
  highlights at a time. Reload re-syncs to whatever the URL is.
- Smoke-test fixture in `tests-lua/smoke.test.lua` covers both shapes — flat link plus grouped entry
  render assertions.

### Changed

- `templates/audit.html` page-eyebrow now reads `<a href="/audit">Admin</a> · audit log` so the
  parent segment is a real link, matching how `Networks` is linked from `/tunnels` and the rest of
  hostops's eyebrow chrome.

## 0.15.9 — 2026-05-04

### Changed

- Bundles `hostops 0.1.3` (grouped sidebar entries + audit breadcrumb fix). No assay-lua source
  changes vs `0.15.8` — this release packages the merged hostops library updates as
  `assay-lib-hostops-0.1.3.tar.gz` alongside the runtime binary.

## hostops 0.1.2 — 2026-05-03

### Added

- `hostops.mount` accepts `extra_sidebar_links = { {href, label, nav_active}, ... }`. Each entry
  renders as a flat sidebar link below the lib's own nav. Plain pass-through — no plugin loader, no
  dispatch shim, no `plugin.toml`. Consumer apps register routes for the `href` themselves and pass
  `nav_active` to highlight the active link.

### Changed

- `templates/audit.html` no longer renders a tab strip pointing at `/inventory`, `/packages`,
  `/settings` — those routes never existed. Eyebrow link to `/inventory` swapped to plain text.

### Removed

- `pages/stubs.lua` and `templates/stub.html`. They rendered "coming soon" placeholders for features
  that aren't built. UI affordances now appear only when the underlying feature exists.

## 0.15.7 — 2026-05-03

### Added

- `assay.rustic` stdlib (`require("assay.rustic")`): rustic backup CLI wrapper — `snapshots`,
  `snapshot_detail`, `init`, `check`, `backup`, `restore`, `forget`. Repository URL + credentials
  travel as environment variables (`RUSTIC_REPOSITORY`, `RUSTIC_PASSWORD`, `AWS_*`) so secrets stay
  off `/proc/<pid>/cmdline`. The binary stays external — no `rustic_core` crate is linked. See
  [`docs/modules/rustic.md`](docs/modules/rustic.md).
- `assay.fs_snapshot` stdlib (`require("assay.fs_snapshot")`): btrfs / zfs subvolume snapshot
  wrapper for crash-consistent backup capture. `detect`, `take`, `release`, `with_snapshot`.
  Auto-selects the backend (`btrfs` / `zfs` / `none`) by inspecting `findmnt` output. See
  [`docs/modules/fs_snapshot.md`](docs/modules/fs_snapshot.md).
- `libs/hostops` library — host-visibility dashboard for nspawn containers, systemd services, cron
  timers, journal logs, networks, audit, host shell, and backups. Mounts on a consumer app's
  `routes` table via `require("hostops.mount")`; ships as a tarball published alongside the assay
  binary (`assay-lib-hostops-<version>.tar.gz`).
- `assay install` subcommand: reads a `Manifest.lua`, fetches declared extension binaries + libs
  over HTTPS, verifies sha256, installs into the configured bin/lib paths, and writes a
  `Manifest.lock` for reproducibility. Plan 21.

## 0.15.5 — UNRELEASED

### Added

- `apt` builtin: `apt.query`, `apt.list_installed`, `apt.list_upgradable`, `apt.add_source`,
  `apt.update`, `apt.install`, `apt.remove`. Wraps `apt-get` and `dpkg-query` for use by the package
  manager framework.
- `http.download(url, path, opts)`: streams a URL to disk via temp-file + atomic rename, with
  optional headers and timeout.
- `crypto.hash_file(path, algo)`: file-streamed hashing (sha2/sha3 family), avoids loading multi-MB
  binaries into Lua strings.
- `compress.untar(archive_path, dest_path, opts)`: extracts a single named member from a tar archive
  (auto-detects gz/xz/zst from extension).
- `systemd.machine_exec(name, cmd, opts)`: runs a command inside an nspawn machine via
  `systemd-run --machine=<name> --pipe --quiet --wait --collect`. Returns the same
  `{status, stdout, stderr, timed_out}` shape as `shell.exec`.
- `assay.pkg` Lua stdlib (`require("assay.pkg")`): catalog/template loaders with three-layer overlay
  (built-in / plugin / operator), strict-override on invalid entries, version comparator
  (semver/v-prefix/calver), host/machine target abstractions, deterministic plan generator.

## [assay-engine 0.4.1] - 2026-04-29

- **Re-publish so `assay_engine::embedded` is reachable from crates.io.** PR #104 added
  `pub mod embedded` (the `embedded::build()` API used to compose assay-engine into a parent
  binary's tokio runtime + axum router), but it landed _after_ `assay-engine 0.4.0` was tagged on
  commit `eae2296`. The published `0.4.0` therefore ships `lib.rs` with
  `config / engine_api /
  init / server / state` and no `embedded`, so any consumer wanting a
  registry version pin against the embedded API was forced onto a `git`/`rev` pin against `main`.
  Patch bump cuts a release that actually contains the module. No source changes vs. main HEAD.

## [assay-domain 0.2.1] - 2026-04-28

- **Add `EngineEventBus::prune_with(PruneOpts)` for namespace-scoped pruning.** The existing
  `prune(before_ts)` issues `DELETE WHERE ts < ?` with no namespace filter, which deletes events
  from every namespace in the shared table. That's correct for the global cluster-wide cleanup loop
  in `assay-workflow::events_cleanup`, but it made tests (and tenant-scoped callers) racy: one bus
  instance's `prune` would silently delete another instance's rows. The non-`#[serial]` test
  `append_then_read_round_trip` started losing its row when `prune_removes_older_than_cutoff` ran
  concurrently — `serial_test::serial` only synchronises tagged tests, so a non-tagged test in the
  same suite races freely. New `prune_with` takes a `PruneOpts` struct (`#[non_exhaustive]`, so
  future filter fields like `subsystem`, `kind`, or `dry_run` add non-breakingly):
  `namespace = Some(ns)` scopes the delete; `namespace = None` matches the global semantic. The
  trait method has a default impl that forwards to `prune` for the `None` case and errors otherwise
  — non-breaking for external implementors of `EngineEventBus`.

## [assay 0.15.4] - 2026-04-28

- **Rename `assay.hashicorp_vault` → `assay.hashicorp.vault`** (closes #92). Establishes a proper
  `hashicorp` namespace mirroring `assay.ory.*`, leaving room for future submodules (consul, nomad,
  boundary, terraform, packer, waypoint). New `assay.hashicorp` umbrella module re-exports `vault`.
  The `assay.openbao` alias now loads through the renamed path. **Breaking** (no back-compat shim):
  scripts requiring `assay.hashicorp_vault` must update to `assay.hashicorp.vault`.
- **Fix `M.ensure_credentials` and `M.assert_secret` mount handling.** Both helpers previously
  hardcoded the KV mount as `"secrets"`, making them unusable against any other mount. Signatures
  now take an explicit `mount` arg: `ensure_credentials(client, mount, path, check_key, generator)`
  and `assert_secret(client, mount, path, expected_keys)`. **Breaking** signature change for any
  existing callers (in practice none, since the hardcoded-mount limitation made the helpers unusable
  for non-`secrets` mounts).

## [assay 0.15.2] - 2026-04-27

- **`crypto.jwt_verify(token, key, opts?)`** — verify-side mirror of `jwt_sign`. PEM (RS256/384/512)
  or JWKS table (dispatched by `kid`). Validates `aud`/`iss`/`exp`/`nbf` with optional `leeway`.
  Lets pure-Lua services accept JWTs without an `assay-engine`.

## [assay-engine 0.4.0 / assay-auth 0.3.0 / assay-vault 0.2.0] - 2026-04-27

| Crate          | Bump          |
| -------------- | ------------- |
| `assay-engine` | 0.3.1 → 0.4.0 |
| `assay-auth`   | 0.2.2 → 0.3.0 |
| `assay-vault`  | 0.1.0 → 0.2.0 |
| (others)       | unchanged     |

**Breaking change (pre-1.0 minor bump).** Adding the `external_issuers` field to `AuthCtx` and
`AuthConfig` changes their shapes; per pre-1.0 semver convention the minor version is the
breaking-change bump. `assay-vault 0.2.0` rides along — its dep declaration on `assay-auth` had to
update from `"0.2"` to `"0.3"` (the published `assay-vault 0.1.0` couldn't be reused because its
baked-in manifest pinned the old assay-auth, and there's no way to mutate a published crate). While
doing it, every public config struct in all three crates is now marked `#[non_exhaustive]` so future
field additions are non-breaking — `AuthCtx`, `AuthConfig`, `EngineConfig`, `BackendConfig`,
`ServerConfig`, `WorkflowConfig`, `AuthSessionConfig`, `AuthPasskeyConfig`,
`AuthOidcProviderConfig`, `DashboardConfig`, `LoggingConfig`, `ExternalIssuerConfig`, plus all 51
public structs/enums in `assay-vault` are now `#[non_exhaustive]` and all require
`Default::default()` + field assignment for external construction. Pattern matches on
`BackendConfig` and on assay-vault's enums (`SealingMethod`, `VaultError`, `ShareTarget`,
`ActiveKek`, `Parent`) from outside the crate must include a wildcard arm.

**Headline:** **JWT pass-through validation.** The engine now accepts JWTs minted by an upstream
OIDC provider (Hydra, Keycloak, Auth0, …) on incoming `Authorization: Bearer ...` requests without
managing engine-side users. Each issuer's JWKS is discovered once, cached in memory, and refreshed
in the background — handles upstream key rotation transparently. Restores the v0.12.1
`--auth-issuer` / `--auth-audience` behavior in TOML config form, with multi-issuer support added.

This is the integration shape every operator who already runs an IdP wanted: keep your existing
identity stack, point assay-engine at it, accept JWTs forwarded by a trusted edge — no engine user
table, no engine sessions, no schema migrations on the auth side, no double-auth in front of the
engine. See `site/pages/auth-pass-through.html` for the architecture write-up.

### Added

- **`[[auth.external_issuers]]` config block** — list of trusted upstream OIDC issuers, each with
  `issuer_url`, `audience`, and `jwks_refresh_secs`. The engine discovers each issuer at boot via
  `<issuer_url>/.well-known/openid-configuration`, caches the JWKS, and verifies incoming JWTs
  against the matching key set. Tokens are routed by `iss` claim — no unnecessary cryptography on
  tokens for issuers we don't trust.
- **`assay_auth::external_jwt::ExternalJwtIssuer`** — public verifier type with full doc comments;
  usable directly by embedders who compose their own auth gate.
- **Boot-time exemption** — when `external_issuers` is non-empty the engine no longer requires
  operator users / `admin_api_keys` to be configured. The upstream IdP is the source of truth.
- **9 unit tests** covering happy path, wrong issuer, wrong audience, unknown kid, expired token,
  audience opt-out, multi-issuer routing by `iss`, unknown-issuer fall-through, and empty-list
  short-circuit.

### Changed

- The "no operator users" boot-error message now mentions `[[auth.external_issuers]]` as a third
  valid satisfying condition. Operators who configure pass-through don't need to also set
  `admin_api_keys`.

### Why this matters

Every comparable auth stack — Ory Hydra, Keycloak, Auth0 SDKs — assumes you bring your own edge that
validates JWTs and forwards. Few of them ship a single binary that _does_ the validation natively,
with JWKS caching + refresh, multi-issuer routing, and a sensible default config — without making
you stand up the IdP itself. That's what this release adds. Combined with the engine's existing OIDC
provider mode (assay-engine ALSO ships as an IdP), operators can mix and match: act as the IdP for
some traffic, accept upstream JWTs for the rest, all behind one binary.

## [assay-engine 0.3.1] - 2026-04-27

| Crate          | Bump          |
| -------------- | ------------- |
| `assay-engine` | 0.3.0 → 0.3.1 |
| (others)       | unchanged     |

**Headline:** `engine.toml` now expands `${VAR}` and `${VAR:-default}` env-var references at load
time, so credentials and per-environment URLs can stay out of config files. Operators wiring the
engine into Kubernetes Secret env vars, systemd `EnvironmentFile=`, or Compose `environment:` blocks
no longer need an external rendering step.

### Added

- **`engine.toml` env-var substitution** — `${VAR}` (required, errors if unset) and
  `${VAR:-default}` (optional, falls back to the inline default) work in any string field of the
  config: `[backend].url`, `[server].public_url`, `[auth].admin_api_keys`, `[auth].issuer`, etc.
  Bracket-less `$VAR` is left untouched so passwords / paths containing literal `$` are safe.
  Sequences whose contents aren't a valid identifier (e.g. `${1NOT_VALID}`, `${has space}`) pass
  through verbatim. README quick-start and `examples/postgres.toml` show the typical Kubernetes
  Secret-env pattern.

### Changed

- `crates/assay-engine/examples/postgres.toml` and `sqlite.toml` updated to demonstrate the new
  `${DATABASE_URL}` / `${DATA_DIR:-./data}` patterns.
- README quick-start now shows env-var-driven configuration.

### Internal

- 14 unit tests added covering set-var, unset-var-with-default, unset-var-no-default error, multiple
  substitutions per line, bracket-less `$VAR` pass-through, invalid identifier pass-through,
  unclosed `${`, plus a from-file integration test.

## [assay 0.15.1] - 2026-04-27

| Crate   | Bump            |
| ------- | --------------- |
| `assay` | 0.15.0 → 0.15.1 |

**Headline:** native Linux observability and systemd control for assay scripts, plus native
browser-shell capability. Three new Rust builtins (`linux`, `cgroup`, `systemd`), two new Lua stdlib
modules (`assay.cron`, `assay.system`), one new PTY primitive (`process.spawn_pty`), one new
`http.serve` response shape (`{ws = function(conn) ... end}` for server-side WebSocket upgrades),
and an `assay.shell` umbrella that bridges the two. Operator dashboards, health-check scripts,
host-introspection automation, and "Open Shell" buttons over xterm.js no longer fork a subprocess
per refresh cycle or sit behind a separate websocket sidecar.

All additions are purely additive — no breaking changes, no migration shim needed. Closes #88.

### Added — `linux` Rust builtin (`/proc` + `/sys/...` readers)

Linux-only. Backed by the `procfs` crate (0.17). Empty table on non-Linux.

```lua
linux.kernel()              -- {version, hostname, os_release, btime}
linux.uptime()              -- {uptime_secs, idle_secs}
linux.loadavg()             -- {one, five, fifteen, running, total, last_pid}
linux.cpu_stat()            -- /proc/stat aggregate row, jiffies
linux.cpu_stat_per_core()   -- per-CPU rows
linux.cpu_percent(prev, curr)
                            -- Lua-side delta math, no kernel call
linux.meminfo()             -- /proc/meminfo as bytes (procfs reports kB)
linux.netdev()              -- /proc/net/dev
linux.diskstats()           -- /proc/diskstats
linux.proc_stat(pid)        -- /proc/<pid>/stat
linux.proc_status(pid)      -- /proc/<pid>/status
```

### Added — `cgroup` Rust builtin (cgroup v2 unified hierarchy)

Linux-only. Pure `std::fs` + small parsers; no new crate dep. Path canonicalisation +
`/sys/fs/cgroup/` prefix check before every read.

```lua
cgroup.version()            -- "v2" | "v1" | "hybrid"
cgroup.list(slice_path)     -- child cgroup directories
cgroup.cpu_stat(path)       -- cpu.stat parsed
cgroup.memory(path)         -- memory.{current,max,swap.*,peak,low,high}
                            -- + memory.events (oom, oom_kill).
                            -- "max" sentinel maps to Lua nil.
cgroup.io(path)             -- io.stat per device
cgroup.pids(path)           -- pids.{current, max}
cgroup.procs(path)          -- cgroup.procs (pid list)
```

### Added — `systemd` Rust builtin (D-Bus + journal)

Linux-only. `zbus` 5 async client; one persistent system-bus connection per Lua VM. Stub table on
non-Linux returns "Linux only" runtime errors.

```lua
-- Units (org.freedesktop.systemd1)
systemd.list_units(filter?), unit_status(name), is_active(name)
systemd.list_timers()
systemd.start, stop, restart, reload   -- return job object path

-- Machines (org.freedesktop.machine1)
systemd.list_machines(), machine_status(name)
systemd.machine_start, machine_poweroff, machine_reboot, machine_terminate

-- Journal
systemd.journal({unit?, machine?, since?, until?, lines?, priority?})
                            -- one-shot read via `journalctl --output=json`
systemd.journal_follow(opts, fn) -> handle
                            -- streaming follow via sd_journal_wait
                            -- (libsystemd.so.0 dlopened at runtime via
                            --  libloading; no libsystemd-dev needed).
                            -- handle:close() stops the stream; worst-case
                            -- shutdown latency 500 ms.
```

`*UsecRealtime` D-Bus values are exposed as integer microseconds since the epoch under `*_realtime`
keys.

### Added — `assay.cron` Lua stdlib (scheduled-job inspector)

Pure Lua — file walks of `/etc/crontab`, `/etc/cron.d/*`,
`/etc/cron.{hourly,daily,weekly,monthly}/*`, `/var/spool/cron/crontabs/*`, plus a passthrough to
`systemd.list_timers()`. 5/6-field crontab parsing with `@reboot` / `@daily` / `@hourly` / `@yearly`
shorthand.

```lua
local cron = require("assay.cron")
cron.system_crontab()       -- /etc/crontab + /etc/cron.d/*
cron.user_crontabs()        -- per-user crontabs
cron.daily_dropins()        -- /etc/cron.{hourly,daily,weekly,monthly}/
cron.timers()               -- thin wrapper around systemd.list_timers()
cron.all()                  -- unified schedule list across every source
```

### Added — `assay.system` Lua umbrella stdlib

Single `require("assay.system")` re-export of `linux`, `cgroup`, `systemd`, and `assay.cron`, plus
convenience aggregates that span sub-modules:

```lua
local sys = require("assay.system")
sys.linux.cpu_stat()
sys.cgroup.memory(path)
sys.systemd.list_machines()
sys.cron.all()

sys.host_snapshot()         -- {cpu, mem, load, uptime, netdev, kernel}
sys.machine_snapshot(name)  -- {info, cgroup={cpu,memory,io,pids}, journal_tail}
sys.machines()              -- list_machines() with cgroup utilisation joined
```

### Tests

15 new unit tests on Linux (5 in `linux::tests`, 10 in `cgroup::tests`) + 3 #[ignore]-gated
journal_follow live-fire tests. Plus 5 D-Bus / journal tests in `systemd::tests` gated `#[ignore]`
(require a running system bus); pass on a typical Linux box with `--include-ignored`.

### Out of scope (reserved for v0.15.x follow-ups)

- macOS / Windows ports of these modules — `/proc` and the systemd D-Bus surface have no analogues,
  so the modules stay Linux-only by design.

## [assay 0.15.0 / assay-vault 0.1.0 / assay-engine 0.3.0 / assay-dashboard 0.3.0] - 2026-04-26

| Crate             | Bump            |
| ----------------- | --------------- |
| `assay`           | 0.14.2 → 0.15.0 |
| `assay-vault`     | NEW → 0.1.0     |
| `assay-engine`    | 0.2.2 → 0.3.0   |
| `assay-dashboard` | 0.2.1 → 0.3.0   |
| `assay-auth`      | unchanged       |
| `assay-workflow`  | unchanged       |

**Headline:** assay-engine adds the **vault module** — KV v2, transit, dynamic credentials,
Bitwarden-aligned vaults + collections + items, biscuit-attenuated share links, sealing (Shamir +
Cloud KMS shape), audit forwarding, and the foundation for a Bitwarden-protocol compatibility shim.
One static binary now covers Vault (HashiCorp / OpenBao), 1Password / Bitwarden self-host, Ory
Kratos / Hydra / Keto, and Temporal — at +1.7 MB on the existing `assay-engine` binary.

See `docs/migration-to-0.3.0.md` for the full migration guide.

### Added — `assay-vault` (new crate)

- KV v2 — versioned, server-decryptable secrets storage. AES-256-GCM-SIV per-record DEK wrapped by
  the master KEK; full lifecycle (PUT/GET/LIST/soft-delete/hard-destroy/undelete with version
  history); path-bound AAD rejects cross-row ciphertext substitution.
- Transit — encrypt/decrypt without exposing key material. `vault:vN:b64` envelope wire format
  (Vault-style); rotation appends a new version, old ciphertexts stay decryptable; AAD binds key
  name + version so cross-key-name decrypt fails.
- Personal vaults + shared collections + items + folders. E2E: collection key encrypted client-side
  via X25519 ECDH to each member's pubkey; server stores ciphertext + envelope blobs only.
- Biscuit-attenuated share links — mint, redeem (public), revoke. Per-block revocation IDs,
  time-bound caveats, content-addressed kid validation catches Shamir's silent-reconstruction attack
  on unseal too.
- Sealing — Shamir SSS init unseal, runtime SealState (sealed → every KV / transit / collection-key
  op fails closed with 503). POST `/sys/init` returns shares once for operator distribution; POST
  `/sys/unseal` accumulates threshold shares.
- Audit forwarding — webhook sink + SinkRegistry that fans events out to every matching glob filter.
  Syslog + S3 sinks reserved in trait shape (land in v0.3.x).
- Dynamic credentials — `DynamicCredsProvider` trait + Postgres provider. Operators register a
  role + grants; `issue` runs `CREATE ROLE … LOGIN PASSWORD …`, `revoke` drops the role. Lease
  tracking in `vault.leases` with a sweepable expiry. AWS / GCP / Kubernetes providers reserved in
  trait shape.

### Added — HTTP

All routes mounted under `/api/v1/vault/*`, admin-key gated except `GET /share/{token}` (public —
biscuit + revocation are the access controls). See `docs/migration-to-0.3.0.md` for the full route
table.

### Added — Lua stdlib

- `assay.vault` — full KV / transit client built on the engine's HTTP surface.
- The pre-existing HashiCorp Vault / OpenBao client moved to `assay.hashicorp_vault`. The
  `assay.openbao` alias still loads through the renamed module.

### Added — `assay-engine`

- New `vault` Cargo feature (default-on); composes `VaultCtx` into `EngineState` via
  `axum::extract::FromRef`. `engine.modules.vault.enabled` controls runtime activation.
- New schema namespace `vault.*` (PG) / attached `vault.db` (SQLite). Migration runs automatically
  on boot.

### Changed — HA failover (plan §S9)

`engine.instances` heartbeat tightened: 15s → 3s; stale cutoff: 60s → 10s. Worst-case failover
detection is now ~10s vs ~60s. No config changes required; takes effect on next boot.

### Migration

See `docs/migration-to-0.3.0.md`.

### Out of scope (reserved for v0.3.x follow-ups)

- Bitwarden-protocol compat shim — full BW client coverage. Phase 7 ships the BW HTTP shape
  (identity, profile, sync, ciphers, folders, discovery probes); end-to-end mobile/browser/CLI
  client coverage rides on a `bw` CLI in CI per plan §"Test plan".
- Cross-method KEK rotation (rotate plaintext → shamir or shamir → KMS in one op). Phase 2 ships
  in-method rewrap; cross-method needs a re-wrap-then-swap flow.
- Recovery delegate (offline admin envelope wrap for collection sharing) — plan §"Deferred" reserves
  this as a v0.4.x item.
- AWS IMDS-based credential fetch — currently the AWS provider + KMS unseal take explicit
  `AwsCredentials`; IMDS / IRSA / EC2-instance-role fetch lands in v0.3.x.

## [assay 0.14.2 / assay-auth 0.2.1 / assay-dashboard 0.2.1 / assay-engine 0.2.2] - 2026-04-26

| Crate             | Bump            |
| ----------------- | --------------- |
| `assay`           | 0.14.1 → 0.14.2 |
| `assay-auth`      | 0.2.0 → 0.2.1   |
| `assay-dashboard` | 0.2.0 → 0.2.1   |
| `assay-engine`    | 0.2.1 → 0.2.2   |

### Fixed

- `assay run <script> -- <args>` passes trailing positionals to Lua's `arg` global (`arg[0]` =
  script path, `arg[1..]` = user values).
- `dofile`, `load`, `loadfile` are usable again — old sandbox over-blocked them. `string.dump` stays
  blocked (bytecode escape).
- Zanzibar tuple writes no longer 500 (#82). `subject_rel` was implicitly NOT NULL via the PK but
  every code path treated it as nullable. Schema stores `''` for direct subjects, the relation name
  for usersets; queries use plain equality.
- Zanzibar namespace POST no longer rejects with `missing field 'wildcard'` (#81). Field is
  `#[serde(default)]` on `TypeRef`.
- Auth Console: removed leaked `phase 8b` planning shorthand (#83) from the Keys empty state and
  three header comments.

### Added

- `ASSAY_BLOCK_GLOBALS` env var: comma-separated names to nil at VM creation. Supports dotted paths
  (`os.execute`, `debug.getinfo`). Typos silently skip.

### Changed

- `Tuple` / `SubjectRef` `subject_rel` is now `String` (was `Option<String>`). Empty string = direct
  subject, non-empty = userset. JSON callers can omit the field; serde defaults to `""`.
- Schema migration is destructive — drop and recreate `auth.zanzibar_tuples` for any existing
  install. (No production assay 0.14.x deployment on file at this writing.)

## [assay 0.14.1 / assay-workflow 0.3.1 / assay-engine 0.2.1] - 2026-04-26

| Crate            | Bump            |
| ---------------- | --------------- |
| `assay`          | 0.14.0 → 0.14.1 |
| `assay-workflow` | 0.3.0 → 0.3.1   |
| `assay-engine`   | 0.2.0 → 0.2.1   |

### Fixed

- `workflow.cancel` no longer 400s on empty body (#66). Stdlib stops sending `[]`, and the handler
  tolerates `{}` / `[]` / no body / `{"reason":"..."}`.
- Pinned the lua coroutine ctx-resume contract with a regression test (#40, fixed in v0.13.0).

### Added — stdlib

- `assay.ansi` — SGR → HTML + strip (#67).
- `assay.url` — RFC 3986 percent encoding + form bodies (#72 prereq).
- `assay.tailscale` — OAuth2 client + auth keys + device management + ACL preview (#72).
- `assay.version` — compare across semver / debian / rpm / numeric (#71 §3).
- `assay.compress` — gunzip / unxz / unzstd Rust builtin (#71 §4).
- `assay.apt` — Debian Packages index reader, sorted via `assay.version` (#71 §2).
- `assay.github` — module-level Releases helpers (`latest_release`, `find_asset`,
  `release_checksum`, …) (#71 §1).

### Added — Lua builtins

- `template.render_with_loader(dir, name, vars)` — `{% extends %}` / `{% include %}` /
  `{% import %}` resolve sibling templates (#64).

### Changed

- `http` response bodies are now raw bytes, not `resp.text()` — round-trips gzip/xz/zst payloads
  without UTF-8 corruption.
- `cancel_workflow` handler reads raw bytes; see #66.

### Migration

No breaking changes. See `docs/migration-to-0.14.1.md`.

### Out of scope

- #75 (drop OpenSSL → RustCrypto for `webauthn-rs`).

## [assay-engine 0.2.0] - 2026-04-25

**Headline:** assay-engine becomes a full Ory replacement + IdP, on top of the Temporal-replacement
workflow engine that already shipped in v0.13.x. One static binary now covers Kratos (identity),
Hydra (OIDC provider), Keto (Zanzibar/ReBAC) and Temporal (workflows) — plus capability tokens
(biscuit) which Ory has nothing equivalent for. PostgreSQL 18 + SQLite, both first-class.

The umbrella v0.2.0 release rolls together the v0.1.2 engine-schemas refactor with the entire auth
stack (plan 12 phases 4-8) and the docs/site refresh. It supersedes the v0.1.2 work that was
in-flight on `feature/engine-0.1.2-schemas` — that PR was closed; this is the consolidated drop.

Active-development release — consumers roll with each bump, no dedicated migration guide. SQLite
deployments delete `./data/` and let the new per-module-file layout populate from scratch. PG
deployments get idempotent `ALTER TABLE … SET SCHEMA …` migrations applied automatically on boot.

Per-crate bumps:

| Crate             | Version           | Notes                                                                                                                           |
| ----------------- | ----------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `assay`           | 0.13.1 → 0.14.0   | New Lua stdlib `assay.auth.*` wrappers (login/passkey/oidc/biscuit/zanzibar/users/sessions/oidc_clients)                        |
| `assay-engine`    | 0.1.1 → **0.2.0** | Headline release. AuthCtx composition, `engine.modules`-driven boot, dashboard auth panes mounted, full `/auth/*` HTTP surface. |
| `assay-workflow`  | 0.2.1 → 0.3.0     | All store queries schema-qualified (`workflow.*`); tracks `assay-domain` 0.2                                                    |
| `assay-domain`    | 0.1.1 → 0.2.0     | New `engine` module hosts engine-core schema (events/lock/namespaces/modules/audit/instances/migrations)                        |
| `assay-auth`      | 0.1.0 → 0.2.0     | First real content release — full module set                                                                                    |
| `assay-dashboard` | 0.1.0 → 0.2.0     | Auth admin SPA (Users / Sessions / OIDC clients / Upstream / Zanzibar / Keys / Audit)                                           |

### Schema/attach storage model

- **PG**: one database, three schemas (`engine`, `workflow`, `auth`) per active module
- **SQLite**: one file per module (`./data/engine.db`, `./data/workflow.db`, `./data/auth.db`),
  attached to one connection on startup so query syntax matches PG (`auth.users`, etc.)
- Cross-schema (PG) and cross-attached-DB (SQLite) transactions stay atomic under the default
  journal mode, preserving the v0.13.1 atomic publish-on-commit guarantee
- Module enablement driven by `engine.modules` row at runtime; compile features control linking

### Added — engine core (was the v0.1.2 scope)

- `engine.modules` (boot manifest), `engine.audit` (ops log), `engine.instances` (multi-node
  visibility — 5 s heartbeat, 60 s stale TTL, graceful shutdown DELETE), `engine.migrations`
  (per-module schema-version tracker)
- `assay_engine::init::EngineBoot` — 8-step boot: open storage → migrate engine schema → read
  enabled modules → create/attach per-module → run module migrations → acquire leader → register
  instance → wire routes
- `[backend].data_dir` config field (default `./data/`); engine creates the dir on boot if missing
- `from_pool` / `from_attached_pool` factory methods on `PostgresStore` / `SqliteStore`
- `GET /healthz` returns engine version + leader status + attached modules
- `GET /api/v1/modules` for dashboard module-pane gating

### Added — auth primitives (Kratos-equivalent identity)

- **Sessions**: `auth.sessions`, opaque `sess_…` cookie, CSRF token, rotation on privilege change,
  HttpOnly + SameSite=Lax + Secure cookies, programmatic `assay_session` + JS-readable `assay_csrf`
- **Passwords**: Argon2id (m=64 MiB, t=3, p=4) with `hash`/`verify`/`needs_rehash`
- **JWT**: Ed25519 issue + verify with kid-based active+history lookup, JWKS rotation,
  `auth.jwks_keys` table
- **Passkey** (WebAuthn): registration + authentication via `webauthn-rs`, state round-trip via
  session payload
- **Stores**: PG + SQLite User/Session stores; `auth.users`, `auth.user_upstream`, `auth.passkeys`,
  `auth.audit`

### Added — biscuit (capability tokens — assay differentiator vs Ory)

- Ed25519 root keypair, persisted in `auth.biscuit_root_keys` with rotation support
- `BiscuitConfig::issue` / `verify` / `attenuate` (the last two work offline — no auth-server
  round-trip per check)
- Datalog policy expressions for time-bound TTLs and scope assertions
- AuthCtx carries `BiscuitConfig` as a non-optional field — same posture as session/JWT

### Added — Zanzibar (Keto-equivalent ReBAC)

- `ZanzibarStore` async trait + PG + SQLite recursive-CTE backends
- SpiceDB-compatible schema parser (definitions, relations, permission expressions with
  `+`/`&`/`-`/`->` arrows + wildcards)
- Operations: `define_namespace`, `write_tuple`, `write_tuples`, `check`, `expand`,
  `lookup_resources`, `lookup_subjects`
- Depth bound 50 + cycle guard via path array
- `auth.zanzibar_namespaces`, `auth.zanzibar_tuples` (with reverse index for lookup)
- `Consistency` enum: `Minimum` / `Exact(zookie)` / `AtLeastAsFresh(zookie)`

### Added — OIDC (client + provider; Hydra-equivalent IdP)

- **OIDC client** (federated SSO): `OidcRegistry` + `OidcClient` per upstream provider; PKCE +
  nonce; userinfo fetch with graceful endpoint degradation
- **OIDC provider** (third-party apps authenticate against assay-engine):
  - Discovery doc at `/.well-known/openid-configuration`
  - JWKS at `/.well-known/jwks.json`
  - `/authorize` (code flow + PKCE), `/token` (auth-code + refresh grants), `/userinfo`, `/revoke`
    (RFC 7009), `/introspect` (RFC 7662)
  - Federation routes (`/oidc/upstream/{slug}/start`, `/callback`)
  - Consent screen (askama-rendered HTML)
  - Admin CRUD over `auth.oidc_clients` + `auth.upstream_providers`
  - Tables: `auth.oidc_clients`, `auth.upstream_providers`, `auth.oidc_authorization_codes`,
    `auth.oidc_refresh_tokens`, `auth.oidc_sessions`

### Added — Dashboard auth admin SPA

Mounted at `/auth/console` when auth module is enabled, served from `assay-dashboard` static assets:

- **Users** — list, search, view (with linked passkeys + sessions + upstream links), enable/disable,
  delete, password reset
- **Sessions** — global + per-user, single + bulk revoke
- **OIDC clients** — CRUD + rotate-secret (display once)
- **OIDC upstream providers** — CRUD
- **Zanzibar** — namespace browser, tuple inspector, check evaluator, expand viewer
- **JWKS / Biscuit keys** — list active + history, rotation trigger
- **Audit log** — paginated `auth.audit` viewer with filters

Conditional render: SPA reads `/api/v1/modules` and renders auth panes only when auth is enabled.

### Added — Lua stdlib `assay.auth.*` (justifies `assay` 0.14.0 bump)

`crates/assay/stdlib/auth.lua` exposes the new auth surface to Lua scripts:

```lua
assay.auth.login(email, password) / logout() / whoami()
assay.auth.passkey.{start_register, finish_register, start_auth, finish_auth}
assay.auth.oidc.{start, complete}
assay.auth.biscuit.{issue, verify, attenuate}     -- verify + attenuate are local
assay.auth.zanzibar.{check, expand, write}
assay.auth.users.{list, get, create, update, delete}
assay.auth.sessions.{list_for_user, revoke, revoke_all_for_user}
assay.auth.oidc_clients.{list, create, rotate_secret}
```

### Schema rename (v0.13.1 → v0.2.0)

All v0.13.1 tables relocated into per-module schemas with the redundant prefix dropped:

| v0.13.1 (`public.*`)  | v0.2.0 (schema-qualified)                               |
| --------------------- | ------------------------------------------------------- |
| `engine_events`       | `engine.events`                                         |
| `engine_lock`         | `engine.lock` (SQLite path; PG uses `pg_advisory_lock`) |
| `namespaces`          | `workflow.namespaces`                                   |
| `api_keys`            | `workflow.api_keys`                                     |
| `workflows`           | `workflow.workflows`                                    |
| `workflow_events`     | `workflow.events`                                       |
| `workflow_activities` | `workflow.activities`                                   |
| `workflow_timers`     | `workflow.timers`                                       |
| `workflow_signals`    | `workflow.signals`                                      |
| `workflow_snapshots`  | `workflow.snapshots`                                    |
| `workflow_schedules`  | `workflow.schedules`                                    |
| `workflow_workers`    | `workflow.workers`                                      |

PG: idempotent migration block in `assay-workflow` runs `ALTER TABLE … SET SCHEMA …; RENAME TO …`
for any v0.13.1 tables found in `public`; safe on fresh installs and already-migrated databases.

SQLite: rebuild from scratch (per active-dev convention).

### Preserved (carried forward from v0.13.1)

- Atomic publish-on-commit — `INSERT INTO engine.events ... RETURNING id; pg_notify(channel, id);`
  in one transaction. Works on PG cross-schema and SQLite cross-attached-DB.
- Multi-node PG coordination via `pg_try_advisory_lock(1)` (leader election) +
  `FOR UPDATE SKIP LOCKED` (distributed task dequeue) + `LISTEN`/`NOTIFY` (cross-instance event
  propagation).
- LISTEN channel naming (`assay_engine_events_<ns>` per namespace) is configured in code, not
  derived from the table name — no rename needed despite `engine_events` → `engine.events`.

### Docs + website

- New `docs/migration-to-0.2.0.md` upgrade guide
- Repositioned `README.md` ("One static binary that replaces Temporal + Kratos + Hydra + Keto") +
  comparison table + auth quick-start
- 5 new `site/pages/auth-*.html` pages (overview / passkey / OIDC quickstart / Zanzibar / biscuit)
  - `compare-vs-ory.html`
- Site nav adds "Auth & IdP"; homepage banner highlights v0.2.0
- Crate-level rustdoc for `assay-auth` with the Ory-replacement narrative + getting-started doctest

### Binary sizes (measured)

- `assay` (Lua runtime + workflow + dashboard): 11 MB
- `assay-engine` (workflow + auth + IdP + dashboard): 8.9 MB

vs Ory: kratos + hydra + keto = ~30-45 MB combined, plus a separate dashboard you build yourself.

## [0.13.1] - 2026-04-24

Engine-events outbox. The PL/pgSQL LISTEN/NOTIFY triggers and the lossy in-memory SSE broadcast are
replaced by a Rust-managed CDC outbox (`engine_events`) that delivers durable realtime events to
dashboards and cross-node subscribers. All state-mutating workflow methods now emit typed
`WorkflowBusEvent` variants via the new `EngineEventBus` trait, which has PG + SQLite
implementations. Dashboards reconnecting after a laptop sleep replay up to 3 days of missed events
from a `Last-Event-ID` cursor; pre-retention gaps return HTTP 410 so the client can snapshot and
resync.

Active-development release — consumers roll with each bump, no dedicated migration guide.

Per-crate bumps:

| Crate            | Version | Notes                                                     |
| ---------------- | ------- | --------------------------------------------------------- |
| `assay`          | 0.13.1  | Dep bump (`assay-workflow 0.2.1`)                         |
| `assay-engine`   | 0.1.1   | Wires PG/SQLite bus + cleanup loop into `run_with_store`  |
| `assay-workflow` | 0.2.1   | Typed `WorkflowBusEvent` emits; SSE rewrite; no triggers  |
| `assay-domain`   | 0.1.1   | `EngineEventBus` trait + PG/SQLite impls + `events` table |

`assay-auth`, `assay-dashboard`, `assay-lua` unchanged.

### Added

- `assay_domain::events::EngineEventBus` trait + `PgEngineEventBus` + `SqliteEngineEventBus`
  implementations.
- `engine_events` table (PG + SQLite) as the durable event outbox.
- `WorkflowEventBus` + `WorkflowBusEvent` enum in `assay-workflow`.
- `EngineConfig.engine_events_ttl_secs` (default `259200` = 3 days) and an hourly cleanup task that
  prunes `engine_events` older than the configured TTL.
- SSE `/api/v1/events/stream` now supports `Last-Event-ID` replay, HTTP 410 Gone on pre-retention
  cursors, and `?ns=&subsystem=&workflow_id=&kind=` server-side filters.

### Changed

- SSE payload shape is now `{id, ts, namespace, subsystem, kind, payload}`.
- Scheduler wake-up is cross-node capable without per-subscription PgListener connections — one
  `assay_engine_events_<ns>` channel per namespace replaces the old per-workflow/per-queue channels.
- `dispatch_recovery` cadence bumped from 1s to 10min. Durable outbox + cursor replay is the
  correctness path; this loop is now a pure crash-safety net.
- `assay-workflow::api::serve_with_bus` is the preferred engine entry point; `serve` /
  `serve_with_version` still exist for bus-less embedders (tests, the `assay-lua` runtime harness).

### Removed

- PL/pgSQL triggers `assay_notify_runnable`, `assay_notify_task`. The migrate path drops them if
  they survive from a v0.13.0 database.
- `assay_runnable_<ns>` / `assay_task_<queue>` NOTIFY channels (replaced by one
  `assay_engine_events_<ns>` channel per namespace).
- `WorkflowStore::subscribe_runnable` and `subscribe_tasks` trait methods — consumers subscribe to
  the `EngineEventBus` instead.
- In-memory `sse_tx` / `engine_tx` broadcast channels + `EngineEvent` / `BroadcastEvent` types on
  `WorkflowCtx`.
- `crates/assay-workflow/tests/subscribe_trait_bounds.rs` + the two `push_*_fires_on_*` tests in
  `smoke_backends.rs` — they exercised the removed surface.

### Fixed

- Dashboard SSE clients no longer lose events when the laptop sleeps longer than the broadcast
  buffer; cursor-based replay refills the gap up to the retention window.

### Known gaps

- `PgConnectOptions` in sqlx 0.8 doesn't expose TCP keepalive knobs, so the listener uses OS-default
  keepalives (Linux: ~2h idle). sqlx auto-reconnect + cursor replay cover silently-dead TCP once the
  next `recv()` errors. A future sqlx bump will let us tune this directly.

## [0.13.0] - 2026-04-22

The monolithic `assay` binary is decomposed into six crates. `assay-lua` becomes a pure Lua runtime
and HTTP client; `assay-engine` becomes a standalone HTTP server that composes `assay-workflow`,
`assay-dashboard`, and (in v0.14.0) `assay-auth` behind one port. SurrealDB is dropped entirely in
favour of PostgreSQL 18 + SQLite, on the evidence of a measured 3× clean build time and 3× peak
compile RAM with no capability gain over PG18 + `pgvector` + recursive CTEs. Auth primitives, the
full OIDC provider, passkey, and Zanzibar ship in v0.14.0 — Phases 4–7 of
`.claude/plans/12-v0.13.0-execution.md` — so this release is deliberately narrower than the original
plan 12 scope. Full upgrade steps live in
[docs/migration-to-0.13.0.md](./docs/migration-to-0.13.0.md).

Six crates go out together:

| Crate             | Version | New / Bumped          |
| ----------------- | ------- | --------------------- |
| `assay-lua`       | 0.13.0  | Bumped (runtime only) |
| `assay-workflow`  | 0.2.0   | Bumped (breaking)     |
| `assay-domain`    | 0.1.0   | New                   |
| `assay-auth`      | 0.1.0   | New (scaffold)        |
| `assay-dashboard` | 0.1.0   | New                   |
| `assay-engine`    | 0.1.0   | New                   |

`assay-workflow` is the only breaking bump — the trait moved to `assay-domain`, `WorkflowCtx<S>`
replaces `Engine<S>`, and backends are now feature-gated additive flags rather than unconditional
compile. `assay-auth` is a scaffold only in this release; its real content ships in v0.14.0.

The root workspace no longer has a `[package]`; it's workspace-only. What used to be the top-level
`assay` binary moved to `crates/assay/` and publishes to crates.io as `assay-lua`. Every domain
concern lives under `crates/<name>/`. `assay-domain` holds the shared `WorkflowStore` trait and DTO
types so any crate can depend on the trait without pulling the whole workflow engine.
`assay-dashboard` holds the HTML/JS/CSS assets that used to live inside `assay-workflow`, exposed
through a thin axum router. The dashboard is now served only by `assay-engine`; the retired runtime
dashboard is gone for good.

The new `assay-engine` binary is the operational heart: `assay-engine serve --config engine.toml`
loads a TOML config, connects to PG18 or SQLite, runs migrations, and serves the workflow API plus
the dashboard on one port. Backend selection is runtime-configurable via
`[backend] type = "postgres" | "sqlite"`, not a build-time feature switch — both drivers compile
into the same binary by default. Example configs live in `crates/assay-engine/examples/` and the
`crates/assay-engine/tests/engine_smoke.rs` integration test proves the full pipeline end-to-end.

PostgreSQL 18 is the minimum supported version. Migrations use the PG18 `uuidv7()` built-in and the
schema is laid out to take advantage of PG18 skip-scan composite indexes (which the Zanzibar tuple
store in v0.14.0 will lean on). Consumers on older Postgres must upgrade before running 0.13.0
migrations.

The SurrealDB backend is removed everywhere — the `backend-surrealdb` Cargo feature, the `surrealdb`
crate dependency, roughly 2400 lines of `src/store/surrealdb/*` impls, and four SurrealQL migration
files are all gone. The measured cost was compile-time pain — 91 s → 281 s clean release build and
1.3 GB → 3.7 GB peak compile RAM — and no production value that PG18 plus `pgvector` plus recursive
CTEs doesn't cover. The full measurement and rationale live in plan 12's revision log. There is no
in-place data migration path from SurrealDB; move to PG18 or SQLite via a clean re-seed or a one-off
replay script.

Embedding the workflow engine inside `assay-lua` is also retired. The runtime binary no longer
depends on `assay-workflow` or `sqlx`, no longer accepts the `assay serve` command, and no longer
runs an internal scheduler. Scripts that need workflow functionality keep using the same HTTP
subcommands they used in 0.12 (`assay workflow start / list / describe / …`) — those commands were
always HTTP clients and they are unchanged. The only difference is that the HTTP endpoint now has to
be a separately deployed `assay-engine` instead of whatever `assay serve` was producing in-process.
Operators pick the engine URL via `$ASSAY_ENGINE_URL` or `--engine URL` as before.

Library consumers who embedded `assay-workflow` as a crate need to update their imports.
`WorkflowStore` lives in `assay_domain` now; DTOs like `WorkflowRecord` and `WorkflowEvent` live in
`assay_domain::types`. The `Engine<S>` generic is gone — its role is merged into `WorkflowCtx<S>`,
which is simultaneously the axum state and the orchestrator (Shape 2B from plan 12a Task 1.3
revised). Call sites go from `Engine::<PostgresStore>::new(store)` to
`WorkflowCtx::start(Arc::new(store))`. Features are now additive, not mutually exclusive:
`backend-postgres` and `backend-sqlite` can both compile into the same binary and the engine picks
one at startup.

The `WorkflowStore::subscribe_runnable` / `subscribe_tasks` methods are now `async` and return the
stream only after the underlying `LISTEN` has been registered on the server. The old shape returned
a lazy `async_stream` that issued `LISTEN` on first poll, which let a caller drop notifications by
calling `pg_notify` between constructing the stream and polling it. `PostgresStore::from_pool(pool)`
is a new constructor for when the engine owns the pool and hands a clone to the workflow module,
matching the plan-12 "shared pool" story.

The engine in 0.13.0 runs with `AuthMode::no_auth()` — there is no JWT or API-key gate on the
workflow API in this release. Do not expose a 0.13.0 engine on the public internet without a network
gatekeeper (Cloudflare Access, Tailscale, VPN, or similar). v0.14.0 wires the full IdP stack in and
flips the default AuthMode to JWT validation against the engine's own OIDC provider.

The runtime `workflow` feature and its embedded-sqlx surface are gone from `assay-lua`'s Cargo
manifest. The redundant `crates/assay/tests/workflow_store.rs` test was deleted — the new
`crates/assay-workflow/tests/smoke_backends.rs` covers the same ground against both backends and
runs in CI. No behaviour loss.

Plan 12's revision log, written inline in the plan file, documents the SurrealDB drop decision with
the measured evidence and documents the 0.13.0 → 0.14.0 scope split so future sessions don't
relitigate either. A backend-parity matrix in the same plan spells out, for every workflow and authz
capability, what differs between PG18 and SQLite so the trait-abstraction contract stays honest
across the two backends.

## [0.12.1] - 2026-04-19

Text-processing stdlib primitives + scratch-based image, so assay scripts don't need a shell in the
container and the published image goes from ~25 MB back to ~10 MB.

### Added

- **`fs.lines(path)`** — streaming line iterator. Designed for
  `for line in fs.lines(path) do … end`; reads from a buffered reader so multi-GB files don't land
  in memory. Each line is returned with the trailing `\n` (or `\r\n`) stripped. Equivalent to
  `while read line; do …; done < file` in bash.
- **`fs.sub_in_file(path, pattern, repl)`** — `sed -i` equivalent, but portable (no BSD-vs-GNU
  `sed -i` dance) and without the quoting traps of shell. Uses Lua patterns (same engine as
  `string.gsub`); `repl` accepts a replacement string with `%0`-`%9` backreferences OR a function.
  Writes only when the substitution count is > 0, so repeated calls on an already-substituted file
  are no-ops on disk.
- **`string.split(s, sep?)`** — awk-style field split, extending Lua's built-in `string` library.
  With no `sep`, splits on any run of whitespace and drops leading/trailing empty fields (matches
  awk default FS and Python `str.split()`). With a literal `sep`, splits on that substring (not a
  Lua pattern — use `string.gmatch` if you need pattern semantics). Pairs with `fs.lines` so
  `awk '{print $2}'`-style pipelines become three lines of Lua.

### Changed

- **Docker image runtime stage back to `FROM scratch`** (reverts the Feb-2026 regression to
  alpine:3.21). The published `ghcr.io/developerinlondon/assay` image is now the assay binary plus
  `/etc/ssl/certs/ca-certificates.crt`, nothing else. About 40% smaller (~10 MB vs ~25 MB with
  Alpine), zero Alpine CVE surface. Downstream images (anyone
  `FROM ghcr.io/developerinlondon/assay`) inherit the slimming automatically on next rebuild. The
  `command: ["/bin/sh", "-c", …]` wrapper that originally forced Alpine has been removed from every
  usage in the gitops repo; the new text-processing primitives above cover the shell-out cases that
  used to need sed/awk.
- **Regression guards on the Dockerfile** (`tests/dockerfile.rs`): asserts the runtime stage is
  `FROM scratch`, that the CA bundle is copied in, and that the ENTRYPOINT uses an absolute path
  (`/assay`, not bare `assay` — scratch has no `$PATH`). These fail CI loudly if anyone tries to
  flip the runtime back to Alpine without justification.

## [0.12.0] - 2026-04-18

This release combines a major dashboard upgrade (Steps tab + step- action signal protocol + the
AWE/consumer architectural-boundary documentation) with a substantial CI/CD overhaul (mise + moon +
a checked-in Playwright e2e suite + the Rust 1.95 toolchain bump) and a new stdlib surface for
orchestrating external processes.

### Added

- **`process.spawn(opts)` and `process.wait(pid, opts?)` Lua builtins.** Launch detached child
  processes from any assay script and reap them later, without dropping to bash. `process.spawn`
  accepts `cmd`, `args`, `cwd`, `env`, `stdout`, `stderr` and returns `{ pid }`; `process.wait`
  blocks (or polls until a `timeout`) and returns `{ status, exited, signaled, timed_out }`. Pairs
  with the existing `process.kill` for full lifecycle control. The dashboard e2e runner at
  `crates/assay-workflow/tests-e2e/run.lua` is the canonical example — boots engine + worker, polls
  `/version`, seeds a workflow, drives Playwright, cleans up. See `docs/modules/process.md` for the
  full surface.

- **Steps tab.** Any workflow that exposes a `pipeline_state` query with a `steps[]` array now gets
  an automatic "Pipeline" tab in the dashboard's detail view. Renders each step as a circle with one
  of five canonical statuses — `waiting ○`, `running ⟳`, `done ✓`, `failed ✕`, `cancelled —` — and
  the connector lines between circles fill state-aware so a glance tells you how far through the
  pipeline the run is. The tab is added at the front and default-selected when present, hidden
  entirely otherwise. See `docs/modules/workflow.md#pipeline-tab-convention` for the schema.

- **Live snapshot tail.** While the Steps tab is open and the workflow is `RUNNING`, the dashboard
  polls `GET /workflows/{id}/state/pipeline_state` every 1s and diff-applies changes onto the
  existing DOM — circles and connectors update in place, log entries append at the bottom, and
  animations on the running step keep their state. Polling stops when the user switches away from
  the tab, the panel closes, or the workflow reaches a terminal status. Includes a scroll-lock
  toggle so operators reading mid-log don't get yanked back to the bottom every second.

- **Per-step actions via signals.** Each step in `steps[]` may include an
  `actions = { "approve", "reject", ... }` array. Those render as buttons under the step's circle;
  clicking one POSTs a `step_action` signal to the engine with payload `{ step, action, user }`. The
  engine routes the signal; the workflow handler decides what each action means. AWE provides the
  plumbing, the consumer provides the semantics — same architectural boundary that keeps the engine
  domain-agnostic.

- **Step log filter.** Clicking a step circle filters the log below to just that step's entries
  (uses the optional `step` field on each log entry). Click again to clear the filter.

### Changed

- **Slim detail layout.** Dropped the left "identity card" column from the inline detail expansion —
  every field it carried (id, type, status, queue, created) is already on the workflow row above or
  in the namespace selector. The only field that wasn't redundant (`completed_at`) now renders as a
  single meta line above the action toolbar, only when the run is terminal. The action toolbar
  (Signal / Cancel / Terminate / Continue-as-new) sits full-width above the tabs, and the tabs
  themselves use the full horizontal width of the expansion. Net effect: tighter detail block, more
  room for the new Steps tab to breathe.

- **Dropped Run ID from detail meta** — it's a near-duplicate of the workflow id shown on the row
  directly above (run id is just the workflow id prefixed with `run-` and suffixed with a
  timestamp).

- **CI/CD overhaul: mise + moon + Playwright e2e + Rust 1.95.** `.mise.toml` now pins rust 1.95.0,
  node 25.9.0, and moon 2.2.1 — one source of truth for tool versions across local dev and CI. moon
  owns the workspace's project graph (`assay-lua`, `assay-workflow`, `dashboard-e2e`, `site`,
  `openclaw-extension`) and runs only the affected tasks on each PR via `moon ci`. Shared task
  templates in `.moon/tasks/tag-*.yml` keep per-project `moon.yml` minimal. New
  `crates/assay-workflow/tests-e2e/` directory holds the dashboard's Playwright suite, run
  automatically by CI whenever the workflow crate changes.

## [0.11.15] - 2026-04-18

### Changed

- **Inline row-expansion hides the detail-header entirely.** Previously the inline expansion still
  rendered the `.detail-header` block (with the h2 hidden but the `✕` close button visible on the
  right), leaving ~40px of whitespace above the actual content. The close button was redundant
  anyway — clicking the row itself toggles expand/collapse — so the whole header block is now
  `display: none` in inline mode. The right-hand side panel still renders its header (no row-above
  context there, no row-click toggle). Detail-body and detail-grid top padding / margin also zeroed
  for inline mode so content sits flush with the top of the expansion.

Cuts another ~40-60px of vertical space per expanded row on top of v0.11.14's id-header hide.

## [0.11.14] - 2026-04-18

### Fixed

- **Worker resilience — don't crash on transient HTTP errors.** The `workflow.listen()` poll loop
  now wraps each heartbeat + task-poll call in `pcall` and backs off exponentially (1s → 2s → 4s →
  8s → 16s, capped at 30s) on failure. Previously a single DNS blip, engine pod restart, or
  kube-proxy hiccup would propagate an error out of the loop, kill the worker, and leave the worker
  row stale until the consumer's pod was restarted — downstream effects included empty Queues view,
  empty Workers view (the registered worker had stopped heartbeating), and silently-dropped workflow
  tasks.

  First successful call after a failure resets the backoff to the baseline so recovery is instant
  once connectivity returns. Warn- level log on each failure includes the backoff duration and error
  message so operators can tell from logs whether a worker is cleanly weathering a blip vs.
  persistently broken.

### Changed

- **Two-column detail layout.** The workflow detail block is now a grid: left column is a
  fixed-width identity card (status badge, meta items stacked as `<dl>`, actions at the bottom) that
  stays visible regardless of which tab is selected; right column gets the rest of the horizontal
  space for tab content. Previously meta + actions ran horizontally above the tabs, which left tab
  content cramped on any single run with more than a few events. Stacks to a single column on
  viewports narrower than 720px.

- **Full workflow IDs in the list view.** Removed the 32-char truncation on the workflow id column;
  long ids wrap at column boundaries via `word-break: break-all`. Makes the id the first thing an
  operator can read and copy without opening the detail view. Since ids follow a consistent pattern
  (`promo-{ts}-{version}-to-{env}`), rows wrap to similar heights — the table doesn't become ragged.

- **Inline detail hides its id header.** When a row is expanded inline, the detail block no longer
  repeats the workflow id as an h2 — the row above already shows the full id, repeating it wastes
  vertical space. The right-hand side panel (used by child-workflow navigation) keeps the h2 because
  there's no row-above context there.

Together the two-column restructure and the header-hide cut ~60px of vertical noise per expanded row
while making the list view more scannable in the collapsed state.

## [0.11.13] - 2026-04-17

### Changed

- **Full workflow IDs in the detail view.** The detail-view header and Run ID meta field no longer
  truncate — the detail panel has the horizontal space for the full id, and operators consulting
  this panel are usually trying to read or copy the id anyway. Long ids wrap cleanly on column
  boundaries via `word-break: break-all`. List-view and children-table truncation retained (density
  matters there).

- **Smart truncate.** The `truncate(str, len)` helper now requires at least 4 chars of actual
  savings before it adds `"..."`. Previously a 34-char id in a 32-char column showed
  `"thirty-two-char-id-exactly-thi..."` — lossy for barely any column gain. Now it just shows the
  full string when trimming wouldn't materially help.

- **Row-click expansion.** Clicking _anywhere_ on a workflow row now toggles the inline detail — not
  just on the id link. Buttons (Signal / Cancel / Terminate) still have their own click behaviour
  and don't trigger expansion. Cursor pointer + hover feedback across the row so the affordance is
  obvious.

- **Modern link hover.** `.data-table .clickable:hover` no longer underlines — it shifts to the
  accent-hover colour instead. Cleaner under monospace id strings where typographic underlines on
  numbers and dashes can look jagged. Row-hover still provides a strong visual affordance.

- **Inline namespace switcher in the status bar.** Replaced the button that opened the sidebar's
  dropdown with its own native `<select>`, styled to look like plain text. The native dropdown now
  opens anchored at the status bar (where the user clicked), not at the top-left sidebar. Mirrors
  the sidebar select's options — switching either keeps both in sync.

- **Modern select trigger.** Both the sidebar namespace select and the new status-bar select got
  flat, OS-chrome-free styling with an inline SVG chevron, accent ring on focus, subtle border
  darken on hover. Native dropdown list still renders OS-default (no way to restyle that without a
  custom combobox).

### Tests

- 32 lib + 40 orchestration tests still pass. Clippy clean with -D warnings.

## [0.11.12] - 2026-04-17

### Added

- **Per-run engine-version stamp.** `start_workflow` now auto-stamps the running engine's version
  into each workflow's `search_attributes` as `assay_engine_version`. Triages "which engine started
  this run" without operators having to keep their own bookkeeping — searchable via
  `workflow.list({ search_attrs = { assay_engine_version = "0.11.12" } })`. Caller-supplied
  `assay_engine_version` wins on conflict (explicit override preserved for replay / testing
  scenarios).

- **More whitelabel knobs:**
  | Variable                             | Purpose                          | Default      |
  | ------------------------------------ | -------------------------------- | ------------ |
  | `ASSAY_WHITELABEL_FAVICON_URL`       | Replace the browser-tab icon     | Built-in SVG |
  | `ASSAY_WHITELABEL_DEFAULT_NAMESPACE` | Namespace the dashboard opens on | `main`       |

- **Tabbed detail view.** The workflow detail block is now organised into tabs — Overview
  (input/result/error), State (register_query snapshot), Events, Children, Attributes.
  Variable-height sections live behind tabs so the meta grid + actions stay compact and scannable
  regardless of how much a run has accumulated. Empty tabs (no state snapshot, no children, no
  search attrs) dim rather than hide, so operators see a consistent shape across runs.

- **Inline row-expansion.** Clicking a row in the workflows list toggles an inline detail block
  beneath it. Click again to collapse. Opening a new row auto-collapses the previous one. The
  right-hand detail panel is retained for child-workflow click-through navigation. Matches the
  "drill into one run while keeping context above/below visible" pattern.

### Changed

- **Footer attribution wording** — whitelabel mode now says "Powered by Assay" with a link to
  https://assay.rs, not "Powered by Assay Workflow Engine". Less redundant when the operator's own
  `_SUBTITLE` already includes "Workflow Engine" (e.g. CC embeds).

- **Clickable namespace in the status bar.** The footer's current namespace value is now a button
  that focuses / opens the sidebar's namespace dropdown — saves a trip to the top of the sidebar
  when the user's already looking at the footer.

- **Collapse-arrow SVGs** replacing the ASCII `<` / `>` chars in the sidebar toggle. Same toggle
  behaviour, cleaner visual, aligned to the rest of assay's outlined-stroke icon set.

- **Workflow IDs get a `title=` tooltip** everywhere they're truncated in the dashboard (workflows
  list, workers list, detail header, run ID, children table). Hover reveals the full ID without
  operators having to open the detail panel to see it.

- **Pagination hides on single-page lists.** The "Prev / Page 1 / Next" chrome used to render even
  when there was only one page; now it renders only when there's actually content to page through.

### Fixed

- **Undefined CSS custom properties** `--surface-1`, `--surface-2`, and `--text-primary` referenced
  by `.btn-action:hover`, `.inline-form`, and the toast component fell back to `transparent` /
  `initial`, which made buttons appear "completely white" on hover against a white page. All three
  references renamed to their defined counterparts (`--surface`, `--surface-hover`, `--text`) — 37
  references now point at defined tokens, zero undefined references remaining.

### Tests

- 5 new whitelabel render tests: favicon URL override, default-namespace data-attribute, "Powered by
  Assay" wording (not Workflow Engine), attribution link presence, favicon-only customisation
  flipping the footer. Total whitelabel unit coverage: 18 tests.
- 5 new `inject_engine_version` unit tests: default (no attrs), existing attrs gain the field,
  caller override wins, non-object JSON preserved, unparsable JSON preserved.
- 40 orchestration + 32 lib tests all pass. Clippy clean with -D warnings.

## [0.11.11] - 2026-04-17

### Added

- **Whitelabel: subtitle + mark-badge + two-line brand layout.** The dashboard sidebar header now
  renders a mark-badge (filled accent square with a single-letter glyph), a bold brand name, and an
  optional muted subtitle underneath — giving operators the canonical two-line brand block without
  needing a bespoke logo SVG. The mark-badge is now always visible (previously only when the sidebar
  was collapsed), so standalone and whitelabel dashboards alike get a proper brand block.

  Two new env vars:

  | Variable                    | Purpose                                                           | Default                         |
  | --------------------------- | ----------------------------------------------------------------- | ------------------------------- |
  | `ASSAY_WHITELABEL_SUBTITLE` | Small muted line under the brand name                             | — (no subtitle rendered)        |
  | `ASSAY_WHITELABEL_MARK`     | Glyph in the badge; override when `NAME`'s first char isn't right | First char of `NAME` uppercased |

  When `ASSAY_WHITELABEL_LOGO_URL` is set, the supplied image replaces the badge glyph entirely via
  `:has(.logo-img)` targeting.

- **Footer attribution: "Powered by" in whitelabel mode.** Any customised identity (non-default
  `NAME`, non-empty `SUBTITLE`, or a `LOGO_URL` / `CSS_URL` set) flips the status-bar engine line
  from `Assay Workflow Engine vX.Y.Z` to `Powered by Assay Workflow Engine vX.Y.Z`, with "Assay
  Workflow Engine" linked to https://assay.rs. Attribution without burying the engine.
  Non-whitelabel deployments see no change.

- **`ctx:cancel(reason)` — workflows can land themselves in CANCELLED.** Raises the internal
  cancellation sentinel the task runner already handles, so a workflow that decides it should stop
  early (human approver rejected, preconditions fail) transitions to engine-level `CANCELLED`
  instead of `COMPLETED`. Previously the only way to reach that status was an external
  `POST /workflows/{id}/cancel`, which forced workflow authors to either return normally (wrong
  status surfaced in dashboards) or raise a generic error (status became `FAILED`, also wrong).
  Distinct from an externally-requested cancel — same terminal state.

  ```lua
  workflow.define("ApproveAndDeploy", function(ctx, input)
      local d = ctx:wait_for_signal("decision")
      if d.action == "reject" then
          state.rejected_by = d.user
          ctx:cancel("rejected by " .. d.user)
      end
      return ctx:execute_activity("deploy", input)
  end)
  ```

### Tests

- 5 new whitelabel render tests: subtitle rendering (set + unset), mark override, "Powered
  by"-footer variant, and the `is_customised()` detection that drives it. Total whitelabel coverage:
  15 tests.
- New orchestration test `lua_workflow_ctx_cancel_lands_in_cancelled_status` verifies a workflow
  calling `ctx:cancel("reason")` ends with `status = "CANCELLED"` and no result payload.

## [0.11.10] - 2026-04-17

### Added

- **Dashboard whitelabel support** — six optional env vars let operators rebrand the embedded
  `/workflow` dashboard per-deployment, so a platform team can surface assay inside their own admin
  UI under their own company name, logo, and browser title without forking the binary. Every knob
  defaults to assay's built-in identity, so an unset env keeps the standalone experience unchanged.

  | Variable                        | Purpose                                       | Default                    |
  | ------------------------------- | --------------------------------------------- | -------------------------- |
  | `ASSAY_WHITELABEL_NAME`         | Text in the sidebar header                    | `Assay`                    |
  | `ASSAY_WHITELABEL_LOGO_URL`     | Image URL rendered before the brand text      | — (no image)               |
  | `ASSAY_WHITELABEL_PAGE_TITLE`   | Browser tab title                             | `Assay Workflow Dashboard` |
  | `ASSAY_WHITELABEL_PARENT_URL`   | Back-link URL in the sidebar footer           | — (hidden)                 |
  | `ASSAY_WHITELABEL_PARENT_NAME`  | Label for the back-link                       | `Back`                     |
  | `ASSAY_WHITELABEL_API_DOCS_URL` | Override / hide the sidebar API Docs link     | `/api/v1/docs`             |
  | `ASSAY_WHITELABEL_CSS_URL`      | Extra stylesheet loaded after assay's own CSS | — (no extra sheet)         |

  `ASSAY_WHITELABEL_API_DOCS_URL=""` (empty string) hides the link entirely — useful when the
  embedding app's ingress doesn't route the OpenAPI path or the docs are provided elsewhere. Any
  other value redirects the link to that URL.

  `ASSAY_WHITELABEL_CSS_URL` lets operators re-skin the dashboard without forking. The extra
  stylesheet loads at the end of `<head>`, after assay's `theme.css` + `style.css`, so source-order
  specificity lets it override any CSS custom property (e.g. `--accent`, `--bg`, `--text`) or
  specific selector. Full design-token list in `docs/modules/workflow.md#dashboard-whitelabel`.
  Asset-version is appended automatically so a redeploy that changes the stylesheet forces a browser
  re-fetch.

  Hosting the logo: if assay is mounted on the same origin as the embedding app (e.g. behind a
  reverse proxy at `/workflow/*`), a path-absolute URL like `/static/my-logo.svg` loads from the
  host app with no CORS plumbing.

- **`workflow.start({namespace, search_attributes})` — full engine parity.** `workflow.start()` now
  passes `opts.namespace` and `opts.search_attributes` through to the engine, so Lua callers can
  scope workflows to a non-default namespace and seed indexed metadata at start time. Previously
  these fields were accepted by `POST /api/v1/workflows` but silently dropped by the Lua stdlib
  client, forcing callers to hit the REST API directly for any multi-tenant deployment.

- **`workflow.listen({namespace})` — namespace-scoped workers.** Workers register into
  `opts.namespace` (default `"main"`) on `POST /workers/register`, so a worker pool in one namespace
  no longer accidentally picks up tasks from a sibling namespace that happens to share its queue
  name. The startup log line now carries the namespace alongside the queue for easy `kubectl logs`
  triage.

Both changes close a gap surfaced by consumers building multi-tenant deployment pipelines on top of
the engine (e.g. a platform-engineering namespace for promotions, a data-engineering namespace for
backfills, both sharing one assay-serve instance). No engine changes — the engine already supported
namespace on these endpoints; only the stdlib was missing.

### Tests

- New orchestration test (`orchestration.rs`): `lua_workflow_namespace_scoping_end_to_end` — creates
  a non-default namespace via the engine API, starts a worker with `namespace="deployments"`, starts
  a workflow in the same namespace, and asserts the completed record carries
  `namespace: "deployments"` and the expected result.

## [0.11.9] - 2026-04-17

### Added

- **`ctx:wait_for_signal(name, { timeout = seconds })` — bounded signal wait.** Returns the signal's
  JSON payload when a matching signal arrives within the timeout, or `nil` when the timer expires
  first. Enables approval gates, external-callback waits, and any workflow that must abandon its
  wait after a deadline — without a side-channel timer or manual race logic in user code.

  The call yields a batch of two commands (`ScheduleTimer` + `WaitForSignal`); on replay the winner
  is decided by comparing history event seqs of the next unconsumed `SignalReceived` against the
  paired `TimerFired`. Determinism matches `ctx:sleep` and `ctx:execute_parallel`.

  Backward compatible: `ctx:wait_for_signal(name)` without opts is unchanged.

### Changed

- `WaitForSignal` engine command accepts an optional `timer_seq`. When present, it is recorded in
  the `WorkflowAwaitingSignal` event payload so the dashboard can show which timer is racing the
  wait.

### Tests

- Two new orchestration tests (`orchestration.rs`):
  - `lua_workflow_wait_for_signal_timeout_signal_wins` — signal arrives before the 30s timer;
    workflow completes with the payload.
  - `lua_workflow_wait_for_signal_timeout_timer_wins` — no signal sent; the 1s timer fires and the
    workflow completes with the timeout branch.

## [0.11.8] - 2026-04-17

### Changed

- **`GET /api/v1/health` and `GET /api/v1/version` are now always unauthenticated,** regardless of
  whether `--auth-issuer` or `--auth-api-key` is set. Standard practice for liveness/readiness
  probes and version discovery — Kubernetes kubelet, load balancers, third-party monitors, and the
  CLI can now reach these endpoints without a bearer token.

  Previously both endpoints lived inside the auth-gated `/api/v1/*` surface, which forced
  `workflow.connect()`'s connectivity probe and kubelet probes to carry a valid credential. That
  blocked legitimate first-boot bootstrap flows (e.g. the gitops reconcile script trying to
  `POST /api/v1/api-keys` through the unauth bootstrap window had to sidestep `workflow.connect`
  entirely).

  All other `/api/v1/*` endpoints remain authenticated when auth is enabled.

### Internal

- `api/public.rs` — new module that owns the public (unauth) sub-router at `/api/v1/*`. Holds
  `health_check` + `version`.
- `api/meta.rs` deleted — its single `/version` route moved to `api/public.rs`. The `VersionInfo`
  struct moved with it.
- `api/workers.rs` no longer registers `/health`. Its single responsibility is `/workers` now.
- `api/mod.rs` `router()` grew a third tier alongside "authenticated /api/v1/_" and "dashboard +
  openapi": "public /api/v1/_", merged outside the auth middleware layer by construction.

### Tests

- Five new auth tests (`auth_test.rs`) verify `/api/v1/health` returns 200 unauth in api-key / jwt /
  combined modes, that `/api/v1/version` is unauth in api-key mode, and that other `/api/v1/*` paths
  still require auth (regression guard against accidentally opening up more of the surface).

## [0.11.7] - 2026-04-17

### Added

- **`POST /api/v1/api-keys` endpoint** — REST alternative to the `assay serve --generate-api-key`
  CLI subcommand. Accepts `{ label?, idempotent? }`. With `idempotent=true` and a key matching the
  label already exists, returns `200 OK` with the existing record's metadata (no plaintext).
  Otherwise mints a fresh key and returns `201 Created` with the plaintext.

  **Bootstrap window:** when the `api_keys` table is empty, `POST
  /api/v1/api-keys` is callable
  without authentication. This is the only way a freshly deployed server running in API-key or
  combined mode can receive its first credential without operator shell access. The window closes
  the moment any key exists.

- **`GET /api/v1/api-keys`** and **`DELETE /api/v1/api-keys/{prefix}`** — list and revoke.

- **`workflow.api_keys.{generate, list, delete}`** Lua stdlib helpers wrapping the above endpoints.
  Example:

  ```lua
  local resp = workflow.api_keys.generate("cc_api_key", { idempotent = true })
  if resp.plaintext then
      -- fresh mint; persist plaintext somewhere (e.g. a k8s Secret)
  else
      -- already exists; plaintext was issued on first call
  end
  ```

### Store

- New `WorkflowStore` trait methods: `api_keys_empty()` (used by the bootstrap-window gate) and
  `get_api_key_by_label(label)` (used by the idempotent-mode lookup). Implemented for both SQLite
  and Postgres.

- `ApiKeyRecord` now derives `utoipa::ToSchema` so the OpenAPI spec includes it.

### Changed

- **`assay-workflow` crate** bumped to `0.1.5` (from `0.1.4`). Additive API changes; downstream
  consumers on `version = "0.1"` continue to work.

## [0.11.6] - 2026-04-17

### Fixed

- **Postgres schema migration crash on startup.** `PostgresStore::migrate()` split the embedded
  `SCHEMA` string by `;` and executed each fragment as SQL. A semicolon inside an SQL line comment
  (`-- Idempotent across startups; fresh installs pick
  the column up from the…`) produced a
  phantom fragment starting with naked prose, and Postgres rejected it with
  `syntax error at or near "fresh"` — which crashed `assay serve` on every boot against a Postgres
  backend, regardless of whether the target database was fresh or already populated. Affects v0.11.3
  through v0.11.5.

  Fix: extract the split into a `sanitise_schema` helper that drops pure-comment lines (leading
  whitespace then `--`) before splitting on `;`. Inline `--`-after-code and string-literal contents
  are left untouched, so the filter is conservative enough to stay correct as the SCHEMA grows more
  prose.

### Changed

- **`assay-workflow` crate** bumped to `0.1.4` (from `0.1.3`). No public API changes. Downstream
  consumers on `version = "0.1"` continue to work.

### Tests

- Added five pure-Rust unit tests for `sanitise_schema` under `src/store/postgres.rs` that run on
  all platforms — no Docker required. Includes a regression test
  (`sanitise_schema_real_constant_produces_only_ddl`) that asserts the live `SCHEMA` constant never
  produces a statement whose first token isn't a recognised SQL keyword. This would have caught the
  v0.11.3 bug at CI time; the existing integration tests under `tests/postgres_store.rs` skip when
  Docker is unavailable (macOS default), which is why this class of bug slipped through.

## [0.11.5] - 2026-04-17

### Changed

- **`assay-workflow` crate version** bumped to `0.1.3` (from `0.1.2`) — carries the v0.11.4
  `AuthMode` refactor from enum to struct. Per assay's pre-1.0 policy of patch-bumps-by-default,
  both crates stay in their current minor tracks until there's a deliberate decision to signal API
  instability to downstream consumers.

### Fixed

- **crates.io publish.** v0.11.4 shipped the binary (GHCR, npm, Linux/macOS artefacts, GitHub
  release) but its crates.io publish failed because `assay-workflow` was still pinned to `0.1.2` —
  the same version already published for v0.11.3. v0.11.5 is a re-release of v0.11.4's code with
  both crates' versions bumped so the publish actually lands on crates.io.

### Docs

- `AGENTS.md` "Release docs checklist" gains an explicit note about `crates/*/Cargo.toml` and the
  independent-versioning policy for sub-crates — the gap that caused the v0.11.4 crates.io failure.

## [0.11.4] - 2026-04-17

### Added

- **Combined JWT + API-key authentication for `assay serve`.** `--auth-issuer` and `--auth-api-key`
  can now be set on the same invocation. When both are enabled, the auth middleware dispatches on
  token shape:

  - Bearer tokens that parse as a JWS header are validated against the OIDC issuer's JWKS.
  - Bearer tokens that are not JWT-shaped are hashed and looked up in the API-key store.

  A semantically-invalid JWT (expired, wrong issuer / audience, forged signature) is rejected on the
  JWT path and is **not** retried as an API key — a token that looks like a JWT is treated as a JWT.
  This lets a single server accept short-lived OIDC user tokens from a browser session and
  long-lived machine API keys from a CI job without the caller picking a mode up front.

### Changed

- **`AuthMode` is now a struct** (`api_key: bool`, `jwt: Option<JwtConfig>`) instead of an enum with
  three variants. Library constructors are unchanged in shape — `AuthMode::no_auth()`,
  `AuthMode::api_key()`, `AuthMode::jwt(issuer, audience)` — and a new
  `AuthMode::combined(issuer, audience)` enables both paths. `AuthMode::is_enabled()` replaces
  `!matches!(.., NoAuth)` call sites.

  Breaking for downstream Rust consumers that matched on `AuthMode::NoAuth | ApiKey |
  Jwt { .. }`.
  The `assay` binary and REST / dashboard users are unaffected.

### Docs

- `docs/modules/workflow.md` auth table adds the combined-mode row and documents the token-shape
  dispatch rule.

## [0.11.3] - 2026-04-16

### Added

- **`ctx:register_query`** — Lua workflows can expose live application-level state to external
  callers via named query handlers. After every worker replay the engine persists a snapshot of
  every handler's result; two new REST endpoints surface it:

  ```
  GET /api/v1/workflows/{id}/state         → latest full snapshot
  GET /api/v1/workflows/{id}/state/{name}  → one handler's value
  ```

  Workflows that don't call `register_query` pay nothing — the worker skips the snapshot command
  entirely when no handlers are registered. A handler that raises is dropped from the snapshot
  rather than crashing the workflow (queries are best-effort read-through).

- **`ctx:continue_as_new`** — Lua surface for the engine-level `continue_as_new` REST endpoint that
  already existed. Workflows yield a `ContinueAsNew` command and the engine closes out the current
  run, starts a fresh one with the same type / namespace / task_queue under `{id}-continued-{ts}`
  with the caller-supplied input and empty event history. Standard pattern for unbounded-loop
  workflows (pollers, schedulers) whose event log would otherwise grow forever.

- **`ctx:execute_parallel`** — Run multiple activities concurrently from a single handler run. The
  worker yields a batch of `ScheduleActivity` commands; the engine schedules them idempotently on
  `(workflow_id, seq)`. Each completion re-dispatches the workflow, replay cache-hits for completed
  activities and re-yields the rest (no-op at the store layer). The handler proceeds past the call
  only when every activity has a terminal event. Per-activity retry / timeout opts match
  `ctx:execute_activity`.

  ```lua
  local results = ctx:execute_parallel({
      { name = "check_a", input = { id = 1 } },
      { name = "check_b", input = { id = 2 }, opts = { max_attempts = 5 } },
      { name = "check_c", input = { id = 3 } },
  })
  -- results[1], [2], [3] in input order; raises if any fail after retries.
  ```

- **`ctx:upsert_search_attributes`** + **search attributes on workflows** — Workflows gain a
  `search_attributes` JSON object settable at start (`POST /workflows` body) and updatable at
  runtime (`ctx:upsert_search_attributes({ … })`). The list endpoint accepts a URL-encoded JSON
  filter that matches workflows whose attributes contain every listed key at the given value:

  ```
  GET /api/v1/workflows?search_attrs=%7B%22env%22%3A%22prod%22%7D
  ```

  SQLite uses `json_extract`; Postgres uses `(search_attributes::jsonb)->>'key'`. Filters AND-join.
  Unchanged keys are preserved across upserts.

- **Schedule `PATCH` / `pause` / `resume`** — Schedules can be updated in place without a
  delete-and-recreate. Only fields present on the patch are touched; unchanged fields keep their
  existing values.

  ```
  PATCH /api/v1/schedules/{name}  body: { cron_expr?, timezone?, input?, task_queue?, overlap_policy? }
  POST  /api/v1/schedules/{name}/pause
  POST  /api/v1/schedules/{name}/resume
  ```

  Paused schedules are skipped by the scheduler; resume recomputes `next_run_at` from now and does
  not backfill missed fires. Updates take effect within a scheduler tick (≤15s).

- **Cron timezone** — Schedules gain a `timezone` field (IANA name, e.g. `"Europe/Berlin"`,
  `"America/New_York"`). Default `"UTC"` preserves v0.11.2 behaviour. The scheduler parses the
  timezone via `chrono-tz` and evaluates the cron expression in that zone, then persists the UTC
  epoch as `next_run_at`. Invalid names are rejected at create / patch time.

- **Optional S3 archival for completed workflows** — Behind the `s3-archival` cargo feature
  (default-off). When compiled in and `ASSAY_ARCHIVE_S3_BUCKET` is set at runtime, a background task
  periodically finds workflows in terminal states older than `ASSAY_ARCHIVE_RETENTION_DAYS` (default
  30), bundles `{workflow_record, events}` as JSON, uploads to
  `s3://bucket/prefix/<namespace>/<workflow_id>.json`, and purges dependent rows (events,
  activities, timers, signals, snapshots). The workflow row itself is retained with `archived_at` +
  `archive_uri` set so `GET /workflows/{id}` still resolves with a pointer to the cold-storage
  bundle.

  Credentials resolve via the AWS SDK's default chain — env vars, shared config, or IRSA /
  pod-identity via web-identity token. Other env vars: `ASSAY_ARCHIVE_S3_PREFIX` (default `assay/`),
  `ASSAY_ARCHIVE_POLL_SECS` (default 3600), `ASSAY_ARCHIVE_BATCH_SIZE` (default 50).

- **`assay.workflow` Lua stdlib — full management surface.** The stdlib now covers every REST
  endpoint the engine exposes, so Lua scripts (including CC and Kubernetes Jobs running
  `assay run seed.lua`) can manage workflows, schedules, namespaces, workers, and queues without
  hand-rolling HTTP calls. New top-level functions:

  ```
  workflow.list(opts)              workflow.list_children(id)
  workflow.terminate(id, reason)   workflow.continue_as_new(id, input)
  workflow.get_events(id)          workflow.get_state(id, name?)
  ```

  New sub-tables (each exposes `create / list / describe / patch / pause / resume / delete` as
  applicable):

  ```
  workflow.schedules   workflow.namespaces   workflow.workers   workflow.queues
  ```

  Every function is a thin HTTP wrapper returning the parsed JSON response (or `nil` on a 404 for
  `describe`/`get_state`), raising on other non-2xx responses.

- **Full CLI for the workflow engine.** The clap-registered `assay workflow …` / `assay schedule …`
  subcommands that through v0.11.2 printed "not yet implemented" and exited 1 are replaced with real
  REST-client implementations, plus a considerable expansion. Everything visible in `assay --help`
  actually runs.

  **Subcommand trees:**

  ```
  assay workflow
    start --type T [--id ID] [--input JSON] [--queue Q] [--search-attrs JSON]
    list [--status S] [--type T] [--search-attrs JSON] [--limit N]
    describe <id>
    state <id> [<query-name>]                   # register_query reader
    events <id> [--follow]                      # log, or poll-stream until terminal
    children <id>
    signal <id> <name> [payload-as-json-or-@file-or--]
    cancel <id>
    terminate <id> [--reason R]
    continue-as-new <id> [--input JSON]         # client-side
    wait <id> [--timeout SECS] [--target STATUS]  # exit 0/1/2 for scripts

  assay schedule
    list
    describe <name>
    create <name> --type T --cron EXPR [--timezone TZ] [--input JSON] [--queue Q]
    patch <name> [--cron EXPR] [--timezone TZ] [--input JSON] [--queue Q] [--overlap POLICY]
    pause <name>
    resume <name>
    delete <name>

  assay namespace   create | list | describe | delete
  assay worker      list
  assay queue       stats
  assay completion  <bash|zsh|fish|powershell|elvish>
  ```

  **Global options** (all flag-backed, env-backed, and config-file-backed, resolved in that
  precedence order):

  - `--engine-url` / `ASSAY_ENGINE_URL` (default `http://127.0.0.1:8080`)
  - `--api-key` / `ASSAY_API_KEY` (bearer token, forwarded as `Authorization: Bearer <value>`)
  - `--namespace` / `ASSAY_NAMESPACE` (default `main`)
  - `--output` / `ASSAY_OUTPUT` — `table` | `json` | `jsonl` | `yaml`; TTY-adaptive default (`table`
    on a terminal, `json` when stdout is piped)
  - `--config` / `ASSAY_CONFIG_FILE` — YAML config file, discovered in this order: flag → env →
    `$XDG_CONFIG_HOME/assay/config.yaml` → `~/.config/assay/config.yaml` → `/etc/assay/config.yaml`

  **Config file** (every field optional):

  ```yaml
  engine_url: https://assay.example.com
  api_key_file: /run/secrets/assay-api-key # preferred over `api_key:`
  namespace: main
  output: table
  ```

  `api_key_file` reads the file contents, trims whitespace, and uses that as the bearer token. Lets
  the config live in a ConfigMap with the credential in a separate Secret.

  **JSON input indirection.** `--input`, `--search-attrs`, and signal payload args accept:

  - a literal JSON string (`'{"n":1}'`)
  - `@PATH` — read the file and parse
  - `-` — read stdin and parse

  **Exit codes:** 0 success, 1 HTTP error / unreachable / not-found, 2 `workflow wait` timeout, 64
  usage error (bad JSON input).

  **Shell completion.** `assay completion <shell> > /etc/bash_completion.d/assay` (or the equivalent
  for your shell). Buffered and graceful on SIGPIPE so piping to `head` doesn't panic. Adds one new
  crate dep: `clap_complete`.

- **Tier-1 dashboard mutations.** The built-in dashboard at `/workflow/` was read-only through
  v0.11.2; every existing view now pairs with its matching operator control:

  - **Workflows view** — new `+ Start workflow` inline form (type / id / task_queue / input JSON /
    search_attributes JSON); per-row Signal / Cancel / Terminate; search-attributes filter in the
    toolbar (debounced, with client-side JSON validation).
  - **Workflow detail panel** — Signal, Cancel, Terminate, and Continue-as-new buttons, all with
    toast feedback. "Live state" card renders the latest snapshot written by `ctx:register_query`
    handlers (with the event seq and timestamp the snapshot was taken at).
  - **Schedules view** — per-row Edit (PATCH form pre-filled with the schedule's values), Pause /
    Resume toggle, Delete. Create form picks up a Timezone field.
  - **Settings view** — Engine Info card shows the engine version + build profile, fetched from
    `/api/v1/version`. Namespace create / delete upgraded to toast feedback and refreshes the
    sidebar namespace switcher.
  - Shared `toast()` + `apiFetchRaw()` helpers exposed via the component context for consistent
    success/error feedback across every mutation.

  Explicitly tier 1 — no in-browser workflow authoring, no batch operations, no reset-to-event, no
  in-browser RBAC. Those are tier 2 / tier 3 and deferred to later releases.

- **`GET /api/v1/version` endpoint.** Returns `{ version, build_profile }`. The CLI passes its own
  `CARGO_PKG_VERSION` to `assay_workflow::api::serve_with_version`, so the field reflects the
  user-facing binary (e.g. `0.11.3`) and not the internal `assay-workflow` crate version. Embedders
  using plain `serve` get the crate version as a fallback. `AppState` gains a
  `binary_version: Option<&'static str>` field.

### Changed

- **`Engine::start_workflow` signature** gains a `search_attributes: Option<&str>` parameter (for
  embedders using the crate directly). REST callers are unaffected; the field is optional on
  `StartWorkflowRequest`.

- **`WorkflowStore::list_workflows` signature** gains a `search_attrs_filter: Option<&str>`
  parameter (for embedders).

- **`WorkflowSchedule`** struct gains a `timezone: String` field. Deserialisers that accept the type
  from an older v0.11.2 engine will need to tolerate the missing field (default "UTC").

- **`WorkflowRecord`** struct gains `search_attributes`, `archived_at`, `archive_uri` fields.

### Fixed

- Removed three pre-existing `clippy::map_identity` warnings in orchestration test helpers so
  `cargo clippy --tests -- -D warnings` stays clean under rust 1.92 / clippy 1.91.

### Notes

- **No migrations from v0.11.2.** The engine is pre-1.0 and no v0.11.x release has been deployed
  against a real workload yet, so all v0.11.3 columns (`search_attributes`, `archived_at`,
  `archive_uri` on `workflows`; `timezone` on `workflow_schedules`) live in the baseline
  `CREATE TABLE` statements only. A fresh DB picks them up automatically; an existing v0.11.2 DB
  needs to be recreated. The migration plumbing is kept in place for post-v0.11.3 additive
  migrations — Postgres does `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` natively, SQLite has a
  dormant `add_column_if_missing` helper that pragma-checks before ALTER. The pattern is documented
  at the bottom of each store's `SCHEMA` constant / `migrate()` fn.
- Parallel activities are still best-effort in the sense that each completion triggers a replay;
  deeply parallel fan-outs generate O(N²) idempotent `schedule_activity` calls. The store-level
  idempotency makes this correct but not minimal; a follow-up can short-circuit re-yields for
  already-scheduled seqs.

## [0.11.2] - 2026-04-16

### Fixed

- **Docker image build** — `Dockerfile` now `COPY crates/` so the `assay-workflow` workspace
  member's manifest is in the build context. Without this, the v0.11.1 release.yml docker job failed
  with `failed to read /app/crates/assay-workflow/Cargo.toml` and no
  `ghcr.io/developerinlondon/assay:v0.11.1` image was published. v0.11.2 republishes everything
  (binaries / crates.io / npm / docker) so `:latest` points at a working image again.

### Notes

- No source-level changes versus v0.11.1 — `assay-lua` and `assay-workflow` crates are
  byte-identical to v0.11.1 except for the version bumps. Existing v0.11.1 binaries, crates.io
  packages, and npm packages remain valid; only the GHCR image was missing.

## [0.11.1] - 2026-04-16

### Added

- **`assay serve`** — Native durable workflow engine built into assay. One binary, multiple modes:
  `assay serve` runs the engine; `assay run worker.lua` runs a worker; `assay workflow` /
  `assay schedule` manage from the shell. Replaces the need for external workflow infrastructure
  (Temporal, Celery, Inngest).

- **Deterministic-replay runtime** — Workflow code is plain Lua run as a coroutine; each `ctx:` call
  gets a per-execution sequence number and the engine persists every completed command
  (`ActivityCompleted`, `TimerFired`, `SignalReceived`, `SideEffectRecorded`,
  `ChildWorkflowCompleted`, …). On replay, `ctx:` calls short-circuit to cached values for
  everything in history; only the next unfulfilled step actually runs. This is how worker crashes
  don't lose work and side effects don't duplicate.

- **Crash safety** — Three independent recovery layers:
  - Activity worker dies → `last_heartbeat` ages out per-activity; engine re-queues per retry
    policy.
  - Workflow worker dies → `dispatch_last_heartbeat` ages out (`ASSAY_WF_DISPATCH_TIMEOUT_SECS`,
    default 30s); any worker on the queue picks up and replays from the event log.
  - Engine dies → all state is in the DB; in-flight tasks become claimable again as heartbeats age
    out. Verified by an end-to-end SIGKILL test in the orchestration suite.

- **Workflow handler context (`ctx`)** — `ctx:execute_activity` (sync, returns result, raises on
  failure after retries), `ctx:sleep(seconds)` (durable timer; survives worker bouncing),
  `ctx:wait_for_signal(name)` (block until matching signal arrives, returns its payload),
  `ctx:start_child_workflow(type, opts)` (sync, parent waits for child), `ctx:side_effect(name, fn)`
  (run non-deterministic op exactly once, cache in event log).

- **REST API** (~25 endpoints) — Workflow lifecycle (`start`, `list`, `describe`, `signal`,
  `cancel`, `terminate`, `continue-as-new`, `events`, `children`); workflow-task dispatch
  (`/workflow-tasks/poll`, `/workflow-tasks/:id/commands`); activity scheduling
  (`/workflows/:id/activities`, `/activities/:id`); worker registration & polling; schedule CRUD;
  namespace CRUD; queue stats. All documented in the served OpenAPI spec.

- **OpenAPI spec** — Machine-readable spec at `/api/v1/openapi.json`. Interactive docs at
  `/api/v1/docs` (Scalar). Enables auto-generation of typed client SDKs in any language via
  `openapi-generator`.

- **Built-in dashboard** — Real-time workflow monitoring at `/workflow/`, brand-aligned with
  [assay.rs](https://assay.rs). Light/dark theme, foldable sidebar, favicon. Six views: Workflows
  (list with status filter, drill-in to event timeline + children), Schedules (list + create),
  Workers (live status + active task count), Queues (pending/running stats
  - warnings when no worker is registered), Namespaces, Settings. Live updates via SSE. Cache-busted
    asset URLs (per-process startup stamp) so a deploy is reflected immediately.

- **Provider-agnostic auth** — Three modes: no-auth (default), API keys (SHA256-hashed in DB),
  JWT/OIDC (validates against any OIDC provider via JWKS with caching, e.g. Cloudflare Access,
  Auth0, Okta, Dex, Keycloak). CLI: `--generate-api-key`, `--list-api-keys`, `--auth-issuer`,
  `--auth-audience`, `--auth-api-key`.

- **Multi-namespace** — Logical-tenant isolation. Workflows / schedules / workers in one namespace
  are invisible to others. Default `main`. CRUD via REST + dashboard.

- **Postgres + multi-instance** — Same engine, swap the backend with `--backend postgres://...` or
  `DATABASE_URL=...`. Cron scheduler uses `pg_try_advisory_lock` for leader election so only one
  instance fires schedules. Activity
  - workflow-task claiming uses `FOR UPDATE SKIP LOCKED` so multiple engine instances don't race.
    SQLite is single-instance only (engine takes an `engine_lock` row at startup).

- **`assay.workflow` Lua stdlib module** — `workflow.connect()`, `workflow.define()`,
  `workflow.activity()`, `workflow.listen()`, plus `workflow.start()` / `signal()` / `describe()` /
  `cancel()` for client-side control. The same `listen()` loop drives both workflow handlers and
  activity handlers — one process, both roles.

- **`examples/workflows/`** — Three runnable examples with READMEs: `hello-workflow/` (smallest
  case), `approval-pipeline/` (signal-based pause/resume), `nightly-report/` (cron + side_effect +
  child workflows).

- **`assay-workflow` crate** — The workflow engine is also publishable as a standalone Rust crate
  (`assay-workflow = "0.1"`) for embedding in non-Lua Rust applications. Zero Lua dependency.

- **SSE client in `http.get`** — Auto-detects `text/event-stream` responses and streams events to an
  `on_event` callback. Backwards compatible with existing `http.get` usage.

### Tests

- **17 end-to-end orchestration tests** (`crates/assay-workflow/tests/orchestration.rs`) including 9
  that boot a real assay subprocess and verify a full workflow runs to a real result. Highlights:
  - `lua_workflow_runs_to_completion` — two sequential activities, real result.
  - `lua_workflow_with_durable_timer` — `ctx:sleep(1)` actually pauses ~1s and resumes.
  - `lua_workflow_with_signal` — workflow blocks, test sends signal, workflow completes with the
    payload bubbled into the result.
  - `lua_workflow_cancellation_stops_work` — cancel mid-sleep; activity that was about to run is
    never scheduled.
  - `lua_workflow_side_effect_is_recorded_once` — side-effect counter file shows fn ran exactly once
    across all replays.
  - `lua_child_workflow_completes_before_parent` — parent + child each run as proper workflows,
    parent picks up child's result.
  - `lua_cron_schedule_fires_real_workflow` — schedule fires within the scheduler tick, workflow
    completes, result lands in DB.
  - `lua_worker_crash_resumes_workflow` — SIGKILL worker A mid-flight; worker B takes over via
    heartbeat-timeout release; workflow completes; side-effect counter still shows exactly one
    execution.

- **11 REST-level tests** (no Lua subprocess) covering scheduling, completion, retries,
  workflow-task dispatch, command processing.

- **10 Postgres tests** (testcontainers-backed) verifying store CRUD parity against a real Postgres
  instance.

### Notes

- The cron crate (`cron = "0.16"`) requires 6- or 7-field cron expressions (with seconds). The
  5-field form fails to parse — use `0 * * * * *` for "every minute on the zero second" or
  `* * * * * *` for "every second."
- The whole engine is gated behind the `workflow` cargo feature (default-on). To build assay without
  it: `cargo install assay-lua --no-default-features --features cli,db,server`.
- Parallel activities (Promise.all-style) are not yet supported; tracked as a follow-up. Sequential
  `ctx:execute_activity` calls and child workflows cover most patterns today.

## [0.11.0] - 2026-04-15

### Removed

- **Temporal integration** — The `temporal` feature flag and all Temporal SDK dependencies
  (`temporalio-client`, `temporalio-sdk`, `temporalio-sdk-core`, `temporalio-common`,
  `prost-wkt-types`) have been removed. The gRPC client (`temporal.connect()`, `temporal.start()`),
  worker runtime (`temporal.worker()`), and HTTP REST stdlib module (`require("assay.temporal")`)
  are no longer available. The Temporal integration never reached production stability and required
  an external Temporal cluster plus `protoc` at build time. A native workflow engine (`assay serve`)
  is planned for v0.11.1.

### Changed

- **Binary size** — 16MB → 11MB (-5MB) with Temporal dependencies removed.
- **Build time** — ~90s → ~34s. `protoc` is no longer required at build time.
- **Stdlib module count** — 35 → 34 (temporal module removed).

## [0.10.4] - 2026-04-12

### Added

- **`os.date(format?, time?)`** — Standard Lua time formatting. Supports strftime patterns (`%Y`,
  `%m`, `%d`, `%H`, `%M`, `%S`, `%c`), the `!` prefix for UTC, and `*t` table output. Previously
  missing from the sandboxed environment.
- **`os.time()`** — Returns current UTC epoch as integer (standard Lua).
- **`os.clock()`** — Returns CPU time in seconds (standard Lua).

## [0.10.3] - 2026-04-12

### Added

- **`ctx:register_query(name, handler)`** — Register query handlers in Temporal workflows. The
  handler function is called when Temporal dispatches a QueryWorkflow activation, and the result is
  returned as a JSON payload. Enables dashboard-style apps to read workflow state in real-time
  without signals.

- **`kratos.flows:get_login_admin(flow_id)`** — Fetch a login flow via the Kratos admin API (no CSRF
  cookie required). Server-side components like hydra-auth should use this instead of `get_login()`
  which requires browser cookies that may not be available across different cookie domains.

## [0.10.1] - 2026-04-12

### Fixed

- **Temporal worker identity** — `temporal.worker()` and `temporal.connect()` now set a non-empty
  `identity` on `ConnectionOptions`. The Temporal SDK v0.2.0 requires this field; without it,
  `init_worker` fails with "Client identity cannot be empty". Identity is set to
  `assay-worker@{task_queue}` for workers and `assay-client@{namespace}` for clients.

## [0.10.0] - 2026-04-11

### Added

- **`assay.gitlab`** — GitLab REST API v4 client. Full coverage of projects, repository files,
  atomic multi-file commits, branches, tags, merge requests, pipelines, jobs, releases, issues,
  groups, container registry, webhooks, environments, deploy tokens, and user endpoints. Supports
  both private access token and OAuth2 bearer authentication. Enables GitOps automation scripts to
  read/write repository content, trigger pipelines, manage merge requests, and interact with
  container registries without external CLI dependencies.

### Changed

- **Sub-object OO convention** across all 35 stdlib modules. Methods are now grouped by resource
  into sub-objects instead of flat on the client:

  ```lua
  -- Before (flat)
  c:merge_requests(project, opts)
  c:create_merge_request(project, opts)

  -- After (sub-objects)
  c.merge_requests:list(project, opts)
  c.merge_requests:create(project, opts)
  ```

  Standard CRUD verbs (`list`, `get`, `create`, `update`, `delete`) are consistent across all
  resources. This makes the API more intuitive and self-documenting. Modules refactored: gitlab,
  github, argocd, vault, s3, unleash, grafana, keto, kratos, hydra, rbac, prometheus, alertmanager,
  traefik, loki, k8s, harbor, temporal, dex, flux, certmanager, eso, crossplane, velero, kargo,
  gcal, gmail, openclaw, zitadel, postgres. Modules unchanged (no client pattern): healthcheck,
  oauth2, email_triage, openbao (alias).

## [0.9.0] - 2026-04-11

### Added

- **Temporal workflow engine** — full workflow execution via Lua coroutines. `temporal.worker()` now
  supports both activities and workflows. Each workflow runs as a coroutine with a deterministic
  `ctx` object:

  - `ctx:execute_activity(name, input, opts?)` — schedule activity, block until complete. Supports
    retry policies, timeouts, heartbeats. On replay, returns cached results without re-executing.
  - `ctx:wait_signal(name, opts?)` — block until external signal or timeout. Signals are buffered
    (safe to call after signal arrives).
  - `ctx:sleep(seconds)` — deterministic timer via Temporal, not wall clock.
  - `ctx:side_effect(fn)` — run non-deterministic function (IDs, timestamps).
  - `ctx:workflow_info()` — workflow metadata (id, type, namespace, attempt).

  Activities and workflows can be registered together in one worker:
  ```lua
  temporal.worker({
    url = "temporal-frontend:7233",
    task_queue = "promotions",
    activities = { update_gitops = function(input) ... end },
    workflows = {
      PromotionWorkflow = function(ctx, input)
        local approval = ctx:wait_signal("approve", { timeout = 86400 })
        local commit = ctx:execute_activity("update_gitops", input)
        return { status = "done", commit_id = commit.id }
      end,
    },
  })
  ```

- **`markdown.to_html(source)`** — new builtin for Markdown to HTML conversion via pulldown-cmark.
  Supports tables, strikethrough, and task lists. Zero binary size overhead (pulldown-cmark was
  already in the dependency tree via temporalio crates).

- **`http.serve()` wildcard routes** — routes ending with `/*` match any path with that prefix. More
  specific wildcards take priority:
  ```lua
  http.serve(8080, {
    GET = {
      ["/api/*"] = function(req) ... end,  -- matches /api/users/123
      ["/*"] = function(req) ... end,      -- catches everything else
    },
  })
  ```

- **Assay builds its own documentation site**. `site/build.lua` replaces the bash/awk/npx pipeline.
  Module count (54) is computed automatically from `src/lua/builtins/mod.rs` and `stdlib/**/*.lua`.
  Site source lives under `site/`, build output goes to `build/site/` (gitignored).

- **Per-module documentation pages**. 36 markdown source files under `docs/modules/` are the single
  source of truth. `build.lua` generates individual HTML pages, a module index, and `llms-full.txt`
  for LLM agents.

- **`site/serve.lua`** — assay serves its own docs site using wildcard routes. 40 lines of Lua, zero
  external dependencies.

- **`fs.read_bytes(path)` / `fs.write_bytes(path, data)`** — binary-safe file I/O. Lua strings can
  hold arbitrary bytes, so these work for images, WASM, protobuf, compressed data, etc.

- **Pagefind search** — full-text search across all docs pages via Ctrl+K modal. Indexed at build
  time (~100KB client bundle), runs entirely in the browser.

### Changed

- **`http.serve()` binary response body** — response `body` field now preserves raw bytes (read via
  `mlua::String`) instead of forcing UTF-8 conversion. Binary assets (WASM, images) serve correctly.

- Version bump to 0.9.0 (from 0.8.4).
- Site source consolidated under `site/` (was split across `site/`, `site-partials/`,
  `site-static/`).
- Nav redesign: no underlines, subtle active page pill, frosted glass header, theme toggle
  persistence across pages.
- `deploy.yml` updated: `cargo build` → `assay site/build.lua` → wrangler deploys `build/site/`.

## [0.8.4] - 2026-04-11

### Added

- **`assay.ory.keto` — OPL permit support and table-style check()**. `k:check()` now accepts a table
  argument in addition to positional args, making OPL permit checks natural:
  ```lua
  k:check({ namespace = "command_center", object = "cc",
            relation = "trigger", subject_id = "user:uuid" })
  ```
  Keto evaluates the OPL rewrite rules and returns true/false — no Lua-side capability mapping
  needed.

- **`k:batch_check(tuples)`** — check multiple permission tuples in a single call. Returns a list of
  booleans in the same order. Each entry uses the same table format as `check()`.

- **`assay.ory.kratos` — complete self-service flow coverage**. Three flow families that were
  missing are now implemented:

  - **Registration**: `c:submit_registration_flow(flow_id, payload, cookie?)` was missing entirely,
    making the registration API unusable.
  - **Recovery** (password reset): `c:create_recovery_flow(opts?)`,
    `c:get_recovery_flow(id, cookie?)`, `c:submit_recovery_flow(flow_id, payload, cookie?)`.
  - **Settings** (profile/password change): `c:create_settings_flow(cookie)`,
    `c:get_settings_flow(id, cookie?)`, `c:submit_settings_flow(flow_id, payload, cookie?)`.

### Fixed

- **`assay.ory.keto`**: `k:delete()` now supports subject_set tuples. Previously only `subject_id`
  was passed to the query string, silently ignoring subject_set-based tuples.

- **`assay.ory.keto`**: `build_query()` now URL-encodes parameter values. Previously special
  characters in subject IDs (e.g. `@` in email addresses) were passed raw, potentially corrupting
  the query string.

- **`assay.ory.kratos`**: `public_post()` now handles HTTP 422 responses (Kratos returns 422 for
  browser flows that need a redirect after successful submission).

## [0.8.3] - 2026-04-07

### Added

- **`assay.ory.rbac`** — capability-based RBAC engine layered on top of Ory Keto. Define a policy
  once (role → capability set) and get user lookups, capability checks, and membership management
  for free. Users can hold multiple roles and the effective capability set is the union, which means
  proper separation of duties is enforceable at the authorization layer (e.g. an "approver" role can
  have `approve` without also getting `trigger`, even if it's listed above an "operator" role with
  `trigger`).

  Public surface:
  - `rbac.policy({namespace, keto, roles, default_role?})`
  - `p:user_roles(user_id)` — sorted by rank, highest first
  - `p:user_primary_role(user_id)` — for compact UI badges
  - `p:user_capabilities(user_id)` — union set
  - `p:user_has_capability(user_id, cap)` — single check
  - `p:add(user_id, role)` / `p:remove(user_id, role)` — idempotent
  - `p:list_members(role)` / `p:list_all_memberships()`
  - `p:reset_role(role)` — for bootstrap/seed scripts
  - `p:require_capability(cap, handler)` — http.serve middleware

- **`crypto.jwt_decode(token)`** — decode a JWT WITHOUT verifying its signature. Returns
  `{header, claims}` parsed from the base64url segments. Useful when the JWT travels through a
  trusted channel (your own session cookie set over TLS) and you just need to read the claims rather
  than verify them. For untrusted JWTs, verify the signature with a JWKS-aware verifier instead.

- **Nested stdlib module loading**: `require("assay.ory.kratos")` now resolves to
  `stdlib/ory/kratos.lua`. The stdlib and filesystem loaders translate dotted module paths into
  directory paths and try both `<path>.lua` and `<path>/init.lua`, matching standard Lua package
  loading conventions.

### Changed

- **BREAKING: Ory stack modules moved under `assay.ory.*`**. The flat top-level `assay.kratos`,
  `assay.hydra`, and `assay.keto` modules are now `assay.ory.kratos`, `assay.ory.hydra`, and
  `assay.ory.keto`. The convenience wrapper `require("assay.ory")` is unchanged and still returns
  `{kratos, hydra, keto, rbac}`.

  Migration: replace `require("assay.kratos")` → `require("assay.ory.kratos")`
  `require("assay.hydra")` → `require("assay.ory.hydra")` `require("assay.keto")` →
  `require("assay.ory.keto")`

  This is the right architectural shape: Ory-specific modules sit under the `assay.ory.*` umbrella
  alongside the new `assay.ory.rbac`, leaving room for `assay.<other-vendor>.*` later without
  polluting the top-level namespace.

## [0.8.2] - 2026-04-07

### Added

- **`assay.hydra` logout challenge methods**: completes the OIDC challenge trio (login, consent,
  logout). When an app calls Hydra's `/oauth2/sessions/logout` endpoint with `id_token_hint` and
  `post_logout_redirect_uri`, Hydra creates a logout request and redirects the browser to the
  configured `urls.logout` endpoint with a `logout_challenge` query param. The handler now has SDK
  methods to process these requests:
  - `c:get_logout_request(challenge)` — fetch the pending logout request (subject, sid, client,
    rp_initiated flag)
  - `c:accept_logout(challenge)` — invalidate the Hydra and Kratos sessions and get back the
    `redirect_to` URL pointing at the app's `post_logout_redirect_uri`
  - `c:reject_logout(challenge)` — for "stay signed in" UIs that let the user cancel the logout

  Symmetric with the existing login/consent challenge methods.

## [0.8.1] - 2026-04-07

### Fixed

- **`req.params` now URL-decodes query string values** in `http.serve`. Previously
  `?challenge=abc%3D` produced `req.params.challenge == "abc%3D"`, so consumers that re-encoded the
  value (such as `assay.hydra:get_login_request`) ended up double-encoding it to `abc%253D` and
  getting a 404 from the upstream service. Values are now decoded with `form_urlencoded::parse`, so
  `+` becomes a space and percent-escapes are decoded correctly. The raw query string remains
  available as `req.query` for handlers that need the verbatim form.

## [0.8.0] - 2026-04-07

### Added

- **Ory stack stdlib modules** — full Lua SDK for the Ory identity/authorization stack:
  - **`assay.kratos`** — Identity management. Login/registration/recovery/settings flows, identity
    CRUD via admin API, session introspection (`whoami`), schema management.
  - **`assay.hydra`** — OAuth2 and OpenID Connect. Client CRUD, authorize URL builder, token
    exchange (authorization_code grant), accept/reject login and consent challenges, token
    introspection, JWK endpoint.
  - **`assay.keto`** — Relationship-based access control. Relation-tuple CRUD, permission checks
    (Zanzibar-style), role/group membership queries, expand API for role inheritance.
  - **`assay.ory`** — Convenience wrapper that re-exports all three modules, with
    `ory.connect(opts)` to build all three clients from one options table.

  Pure Lua wrappers over the Ory REST APIs. Zero new Rust dependencies — binary size unchanged. Each
  module follows the standard `M.client(url, opts)` pattern with comprehensive `@quickref` metadata
  for `assay context` discovery.

- **Multi-value response headers in `http.serve`**: Header values can now be a Lua array of strings,
  emitting the same header name multiple times. Required for `Set-Cookie` when setting multiple
  cookies in one response, and for other headers that legitimately repeat (e.g., `Link`, `Vary`,
  `Cache-Control`).

  ```lua
  return {
    status = 200,
    headers = {
      ["Set-Cookie"] = {
        "session=abc; Path=/",
        "csrf=xyz; Path=/",
      },
    },
  }
  ```

  String values continue to work as before.

### Theme

This is the **identity and auth stack** release. Assay now ships with a complete SDK for building
OIDC-integrated apps on Ory: one app can handle Hydra login/consent challenges, query Keto
permissions, and manage Kratos identities — all in idiomatic Lua with zero external dependencies
beyond the existing assay binary.

## [0.7.2] - 2026-04-07

### Added

- **`req.params` in `http.serve`**: Query string parameters are now automatically parsed into a
  `params` table on incoming requests. For example, `?login_challenge=abc&foo=bar` becomes
  `req.params.login_challenge == "abc"` and `req.params.foo == "bar"`. The raw query string remains
  available as `req.query`.

## [0.7.1] - 2026-04-06

### Changed

- **Temporal included by default**: The `temporal` feature is now part of the default build. The
  standard Docker image and binary include native gRPC workflow support out of the box.
- **CI/Release/Docker**: Added `protoc` installation to all build environments for gRPC proto
  compilation.

## [0.7.0] - 2026-04-06

### Added

- **Temporal gRPC client** (optional `temporal` feature): Native gRPC bridge for Temporal workflow
  engine via `temporalio-client` v0.2.0. The `temporal` global provides `connect()` for persistent
  clients and `start()` for one-shot workflow execution. Client methods: `start_workflow`,
  `signal_workflow`, `query_workflow`, `describe_workflow`, `get_result`, `cancel_workflow`,
  `terminate_workflow`. All methods are async and use JSON payload encoding. Build with
  `cargo build --features temporal` — requires `protoc` (install via `mise install protoc`).
- **8 new tests** for temporal gRPC registration, error handling, and stdlib compatibility.

### Dependencies (temporal feature only)

- `temporalio-client` 0.2.0
- `temporalio-sdk` 0.2.0
- `temporalio-common` 0.2.0
- `url` 2.x

## [0.6.1] - 2026-04-06

### Fixed

- **http.serve async handlers**: Route handlers are now async (`call_async`), allowing them to call
  `http.get`, `sleep`, and any other async builtins. Previously, calling an async function from a
  route handler would crash with "attempt to yield from outside a coroutine". This was the only
  remaining sync call site for user Lua functions.

### Added

- **`npx skills add developerinlondon/assay`** — install Assay's SKILL.md into your AI agent project
  via the skills CLI.
- **Dark/light theme toggle** on assay.rs with localStorage persistence.
- **Version stamp in site footer** — shows git tag or SHA from deploy pipeline.
- **Infrastructure Testing** highlighted as core capability on the homepage.

### Changed

- **Site overhaul** — compact hero, service grid above the fold with SVG icons, side-by-side size &
  speed comparison charts, consistent nav across all pages, accurate module coverage (removed
  misleading "Coming Soon" features).
- **Comparison page** — renamed from "MCP Comparison", removed out-of-scope entries, shows only
  domains Assay actually covers.
- **README** — full size & speed comparison table with all 10 runtimes and cold start times.

## [0.6.0] - 2026-04-05

### Added

- **6 new stdlib modules** (23 -> 29 total):
  - **assay.openclaw** — OpenClaw AI agent platform integration. Invoke tools, send messages, manage
    persistent state with JSON files, diff detection, approval gates, cron jobs, sub-agent spawning,
    and LLM task execution. Auto-discovers `$OPENCLAW_URL`/`$CLAWD_URL`.
  - **assay.github** — GitHub REST API client (no `gh` CLI dependency). Pull requests (view, list,
    reviews, merge), issues (list, get, create, comment), repositories, Actions workflow runs, and
    GraphQL queries. Bearer token auth via `$GITHUB_TOKEN`.
  - **assay.gmail** — Gmail REST API client with OAuth2 token auto-refresh. Search, read, reply,
    send emails, and list labels. Uses Google OAuth2 credentials and token files.
  - **assay.gcal** — Google Calendar REST API client with OAuth2 token auto-refresh. Events CRUD
    (list, get, create, update, delete) and calendar list. Same auth pattern as gmail.
  - **assay.oauth2** — Google OAuth2 token management. File-based credentials loading, automatic
    access token refresh via refresh_token grant, token persistence, and auth header generation.
    Used internally by gmail and gcal modules. Default paths: `~/.config/gog/credentials.json` and
    `~/.config/gog/token.json`.
  - **assay.email_triage** — Email classification and triage. Deterministic rule-based
    categorization of emails into needs_reply, needs_action, and fyi buckets. Optional LLM-assisted
    triage via OpenClaw for smarter classification. Subject and sender pattern matching for
    automated mail detection.
- **Tool mode**: `assay run --mode tool` for OpenClaw integration. Runs Lua scripts as deterministic
  tools invoked by AI agents, with structured JSON output.
- **Resume mechanism**: `assay resume --token <token> --approve yes|no` for resuming paused
  workflows after human approval gates.
- **OpenClaw extension**: `@developerinlondon/assay-openclaw-extension` package (GitHub Packages).
  Registers Assay as an OpenClaw agent tool with configurable script directory, timeout, output size
  limits, and approval-based resume flow. Install via
  `openclaw plugins install @developerinlondon/assay-openclaw-extension`.

### Architecture

- **Shell-free design**: All 6 new modules use native HTTP APIs exclusively. No shell commands, no
  CLI dependencies (no `gh`, no `gcloud`, no `oauth2l`). Pure Lua over Assay HTTP builtins.

## [0.5.6] - 2026-04-03

### Added

- **SSE streaming** for `http.serve` via `{ sse = function(send) ... end }` return shape. SSE
  handler runs async so `sleep()` and other async builtins work inside the producer. `send` callback
  uses async channel send with proper backpressure handling. Custom headers take precedence over SSE
  defaults (Content-Type, Cache-Control, Connection).
- **assert.ne(a, b, msg?)** — inequality assertion for the test framework.

### Fixed

- **Content-Type precedence**: User-provided `Content-Type` header no longer overwritten by defaults
  (`text/plain` / `application/json`) in `http.serve` responses.
- **SSE newline validation**: `event` and `id` fields reject values containing newlines or carriage
  returns to prevent SSE field injection.

## [0.5.5] - 2026-03-13

### Added

- **follow_redirects** option for YAML HTTP checks. Set `follow_redirects: false` to disable
  automatic redirect following, allowing verification of auth-protected endpoints that return 302
  redirects to identity providers. Defaults to `true` for backward compatibility.
- **follow_redirects** option for Lua `http.client()` builder. Create clients with
  `http.client({ follow_redirects = false })` for the same no-redirect behavior in scripts.

## [0.5.4] - 2026-03-12

### Fixed

- **unleash.ensure_token**: Send `tokenName` instead of `username` in create token API payload. The
  Unleash API expects `tokenName` — sending `username` caused HTTP 400 (BadDataError). Function now
  accepts both `opts.tokenName` and `opts.username` for backward compatibility. Existing token
  matching also checks `t.tokenName` with fallback to `t.username`.

## [0.5.3] - 2026-03-12

### Added

- **disk builtins**: `disk.usage(path)` and `disk.mounts()` for filesystem disk information
- **os builtins**: `os.info()` returning name, version, arch, hostname, uptime
- **Expanded fs builtins**: `fs.exists`, `fs.is_dir`, `fs.is_file`, `fs.list`, `fs.mkdir`,
  `fs.remove`, `fs.rename`, `fs.copy`, `fs.stat`, `fs.glob`, `fs.temp_dir`
- **Expanded env builtins**: `env.set`, `env.unset`, `env.list`, `env.home`, `env.cwd`

### Fixed

- Cross-platform casts in `disk.rs` (`u32` on macOS, `u64` on Linux)

## [0.5.2] - 2026-03-11

### Added

- **shell builtins**: `shell.run(cmd)`, `shell.output(cmd)`, `shell.which(name)`, `shell.pipe(cmds)`
- **process builtins**: `process.spawn(cmd, opts)`, `process.kill(pid)`, `process.pid()`,
  `process.list()`, `process.sleep(secs)`
- **Expanded fs builtins**: `fs.read_bytes`, `fs.write_bytes`, `fs.append`, `fs.symlink`,
  `fs.readlink`, `fs.canonicalize`, `fs.metadata`

### Fixed

- `http.serve` port race condition — use ephemeral ports with `_SERVER_PORT` global
- Symlink safety, timeout validation, pipe drain, PID validation hardening

## [0.5.1] - 2026-02-23

### Added

- **Website**: Static site at assay.rs on Cloudflare Pages with homepage, module reference, AI agent
  integration guides, and MCP comparison page mapping 42 servers
- **llms.txt**: LLM agent context traversal files (`llms.txt` and `llms-full.txt`)
- **Enriched search keywords**: All 23 stdlib modules and builtins enriched with `@keywords`
  metadata for improved discovery

### Changed

- Updated README with website links
- Updated SKILL.md with MCP comparison and agent integration guidance

## [0.5.0] - 2026-02-23

### Added

- **CLI subcommands**: `assay exec` for inline Lua execution, `assay context` for prompt-ready
  module output, `assay modules` for listing all available modules
- **Module discovery**: LDoc metadata parser with auto-function extraction from all 23 stdlib
  modules
- **Search engine**: Zero-dependency BM25 search with FTS5 backend for `db` feature
- **Filesystem module loader**: Project/global/builtin priority for `require()` resolution
- **LDoc metadata headers**: All 23 stdlib modules annotated with `@module`, `@description`,
  `@keywords`, `@quickref`

### Changed

- CLI restructured to clap subcommands with backward compatibility
- Feature flags added for optional `db`, `server`, and `cli` dependencies

## [0.4.4] - 2026-02-20

### Added

- **Unleash stdlib module** (`assay.unleash`): Feature flag management client for Unleash. Projects
  (CRUD, list), environments (enable/disable per project), features (CRUD, archive, toggle on/off),
  strategies (list, add), API tokens (CRUD). Idempotent helpers: `ensure_project`,
  `ensure_environment`, `ensure_token`.

## [0.4.3] - 2026-02-13

### Added

- **crypto.hmac**: HMAC builtin supporting all 8 hash algorithms (SHA-224/256/384/512,
  SHA3-224/256/384/512). Binary-safe key/data via `mlua::String`. Supports `raw` output mode for key
  chaining (required by AWS Sig V4). Manual RFC 2104 implementation using existing sha2/sha3 crates
  — zero new dependencies.
- **S3 stdlib module** (`assay.s3`): Pure Lua S3 client with AWS Signature V4 request signing. Works
  with any S3-compatible endpoint (AWS, iDrive e2, Cloudflare R2, MinIO). Operations: create/delete
  bucket, list buckets, put/get/delete/list/head/copy objects, bucket_exists. Path-style URLs
  default. Epoch-to-UTC date math (no os.date dependency). Simple XML response parsing via Lua
  patterns.
- 15 new tests (7 HMAC + 8 S3 stdlib)

### Changed

- **Modular builtins**: Split monolithic `builtins.rs` (1788 lines) into `src/lua/builtins/`
  directory with 10 focused modules: http, json, serialization, assert, crypto, db, ws, template,
  core, mod. Zero behavior change — pure refactoring for maintainability.

## [0.4.2] - 2026-02-13

### Fixed

- **zitadel.find_app**: Improved with name query filter and resilient 409 conflict handling

## [0.4.1] - 2026-02-13

### Fixed

- **zitadel.create_oidc_app**: Handle 409 conflict responses gracefully

## [0.4.0] - 2026-02-13

### Added

- **Zitadel stdlib module** (`assay.zitadel`): OIDC identity management with JWT machine auth
- **Postgres stdlib module** (`assay.postgres`): Postgres-specific helpers
- **Vault enhancements**: Additional vault helper functions
- **healthcheck.wait**: Wait helper for health check polling

### Fixed

- Use merge-patch content-type in `k8s.patch`

## [0.3.3] - 2026-02-12

### Added

- **Filesystem require fallback**: External Lua libraries can be loaded via filesystem `require()`

### Fixed

- Load K8s CA cert for in-cluster HTTPS API calls

## [0.3.2] - 2026-02-11

### Added

- **crypto.jwt_sign**: `kid` (Key ID) header support for JWT signing

### Fixed

- Release workflow: Filter artifact download to exclude Docker metadata

## [0.3.1] - 2026-02-11

- Publish crate as `assay-lua` on crates.io (binary still installs as `assay`)
- Add release pipeline: pre-built binaries (Linux x86_64 static, macOS Apple Silicon), Docker,
  crates.io
- Add prerequisite docs to K8s-dependent examples
- Fix flaky sleep timing test

## [0.3.0] - 2026-02-11

First feature-complete release. Assay is now a general-purpose Lua runtime for Kubernetes — covering
verification, scripting, automation, and lightweight web services in a single ~9 MB binary.

### Added

- **Direct Lua execution**: `assay script.lua` with auto-detection by file extension
- **Shebang support**: `#!/usr/bin/assay` for executable Lua scripts
- **HTTP server**: `http.serve(port, routes)` — Lua scripts become web services
- **Database access**: `db.connect/query/execute` — PostgreSQL, MySQL/MariaDB, SQLite via sqlx
- **WebSocket client**: `ws.connect/send/recv/close` via tokio-tungstenite
- **Template engine**: `template.render/render_string` via minijinja (Jinja2-compatible)
- **Filesystem write**: `fs.write(path, content)` complements existing `fs.read`
- **YAML builtins**: `yaml.parse/encode` for YAML processing in Lua scripts
- **TOML builtins**: `toml.parse/encode` for TOML processing in Lua scripts
- **Async primitives**: `async.spawn(fn)` and `async.spawn_interval(ms, fn)` with handles
- **Crypto hash**: `crypto.hash(algo, data)` — SHA-256, SHA-384, SHA-512, SHA3-256, SHA3-512
- **Crypto random**: `crypto.random(length)` — cryptographically secure random hex strings
- **JWT signing**: `crypto.jwt_sign(claims, key, algo)` — RS256/RS384/RS512
- **Regex**: `regex.match/find/find_all/replace` via regex-lite
- **Base64**: `base64.encode/decode`
- **19 stdlib modules**: prometheus, alertmanager, loki, grafana, k8s, argocd, kargo, flux, traefik,
  vault, openbao, certmanager, eso, dex, crossplane, velero, temporal, harbor, healthcheck
- **E2E dogfood tests**: Assay testing itself via YAML check mode
- **CI**: GitHub Actions with clippy + tests on Linux (x86_64) and macOS (Apple Silicon)
- **491 tests**, 0 clippy warnings

### Changed

- CLI changed from `assay --config file.yaml` to `assay <file>` (positional arg, auto-detect)
- Lua upgraded from 5.4 to 5.5 (global declarations, incremental major GC, compact arrays)
- HTTP builtins DRYed (collapsed 4x duplicated method registrations into generic loop)

## [0.0.1] - 2026-02-09

Initial release. YAML-based check orchestration for ArgoCD PostSync verification.

### Added

- YAML config with timeout, retries, backoff, parallel execution
- Check types: `type: http`, `type: prometheus`, `type: script` (Lua)
- Built-in retry with exponential backoff
- Structured JSON output with pass/fail per check
- K8s-native exit codes (0 = all passed, 1 = any failed)
- HTTP client builtins: `http.get/post/put/patch`
- JSON builtins: `json.parse/encode`
- Assert builtins: `assert.eq/gt/lt/contains/not_nil/matches`
- Logging builtins: `log.info/warn/error`
- Environment: `env.get`, `sleep`, `time`
- Prometheus stdlib module
- Docker image: Alpine 3.21 + ~5 MB binary
