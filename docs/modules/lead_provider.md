---
category: Registries
tagline: The lead-data contract — uniform records with provenance, and the gate every paid call goes through
---

## assay.lead_provider

What every lead-data adapter implements, so downstream code never learns a vendor's envelope and
never reaches a paid endpoint by accident. Free registry modules (`assay.gleif`, `assay.edgar`)
present the same record shape with `provider = "registry:<name>"`, so provenance reads the same
whether a fact was bought or looked up for nothing.

### The budget gate

The spend ledger lives in the caller's database, not in assay, so the budget context is **injected
rather than discovered** — and a gate cannot be built without one. That is the point: a paid lookup
must not be reachable by forgetting an argument.

```lua
local lp = require("assay.lead_provider")

local gate = lp.gate({
  approve = function(op, cents) ... end,  -- true | false, reason
  meter   = function(op, cents, meta) ... end,
})

local person, reason = gate:paid("find_person", 25, function()
  return provider_call()
end)
```

- `approve` returns `false, reason` when the spend is over the line; `paid` then returns
  `nil, reason` and **the provider is never called**. A gate that logged after the fact would not be
  a gate.
- `meter` runs only after a call that succeeded. The ledger answers "what did this cost", and a call
  that raised bought nothing — metering it would inflate cost-per-qualified-lead with spend that
  never happened.
- The operation must be one of `search_companies`, `find_person`, `resolve_email`, `verify_email`.
  Spend is attributed per operation, so an adapter inventing its own name would make the ledger
  unqueryable.

### Record shapes

```lua
lp.person(provider, from_url, { first_name = ..., last_name = ..., domain = ... })
lp.email(provider, from_url, { address = ..., confidence = ... })
lp.provenance(provider, from_url)   -- { provider, retrieved_from, retrieved_at }
```

Fields the provider did not answer stay `nil` rather than becoming empty strings: absent evidence
and blank evidence are different claims, and only one of them should survive into a record someone
later acts on.

`email` defaults `verification_status` to `UNKNOWN`. An adapter reports what a provider claims and
never promotes that claim to `VERIFIED` — under NEP-0007 §2 only a delivery earns it, and `PROBABLE`
is the ceiling for an SMTP-level accept (see [`email_verify`](email_verify.html)).

### Running the live smoke tests

The adapters are tested against recorded shapes, which cannot catch the one thing that matters most:
whether the vendor still sends the fields the adapter reads. BetterContact's verdict field was wrong
once for exactly that reason — the fixture carried the same mistake the code did.

The live tests are gated on a key being present and skip without one, so the suite stays green
everywhere:

```sh
CONTACTOUT_TOKEN=… cargo test -p assay-lua --test lead_smoke
BETTERCONTACT_API_KEY=… cargo test -p assay-lua --test lead_smoke
```

They look up a public figure at a public company deliberately: a smoke run must not spend credits on
a real prospect, nor put a private individual's address in a CI log. One of them also proves a
declined budget stops a call that would have cost money — against the real API, not a mock.
