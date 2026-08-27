---
category: Registries
tagline: BetterContact waterfall enrichment — submit a batch, poll the run, read verified emails
---

## assay.bettercontact

Waterfall enrichment across many upstream providers. The API is **asynchronous**: submit a batch,
then poll until the run terminates. Like [`contactout`](contactout.html), the
[budget gate](lead_provider.html) is required.

```lua
local bc = require("assay.bettercontact")
local c = bc.client(gate, { api_key = os.getenv("BETTERCONTACT_API_KEY") })

local id = c:submit({
  { first_name = "Jonathan", last_name = "Church", company_domain = "cheaney.co.uk" },
})
local run = c:await(id, { poll_ms = 3000, attempts = 40 })
if run.terminated then for _, p in ipairs(run.people) do ... end end

c:find_person({ first_name = "Jonathan", last_name = "Church", domain = "cheaney.co.uk" })
c:resolve_email({ linkedin_url = "..." })
```

### Only `terminated` means finished

The run reports `processing`, `on_hold` or `terminated`, and a poll **returns HTTP 202 while still
processing** with no `data` and no `summary`. Branching on the HTTP code instead of `status` reads
an in-flight run as an empty result — so `result()` exposes `terminated` as a boolean and `await()`
branches on that alone.

`await` returns the last run it saw when the attempt budget runs out, with `terminated = false`.
That is not an error: the request id stays valid, and a caller may reasonably come back to it later.

Only the submission is metered. **Polling is free and does not go through the gate.**

### Verification status

BetterContact's own per-contact verdict maps into NEP-0007 §2's vocabulary, and stops at `PROBABLE`:

| BetterContact                      | Recorded as |
| ---------------------------------- | ----------- |
| `valid`, `deliverable`             | `PROBABLE`  |
| `catch_all`                        | `CATCH_ALL` |
| `undeliverable`, `invalid`         | `INVALID`   |
| `not_found`, anything unrecognised | `UNKNOWN`   |

A vendor asserting an address is good is not a delivery, so nothing here can reach `VERIFIED`.
