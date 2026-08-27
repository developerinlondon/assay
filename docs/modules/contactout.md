---
category: Registries
tagline: ContactOut lead data — LinkedIn enrichment and work-email lookup, budget-gated
---

## assay.contactout

Every call here costs money, so the [budget gate](lead_provider.html) is the **first argument and is
not optional**: a client that could be built without one would make an unmetered paid call reachable
by forgetting a parameter.

```lua
local lp = require("assay.lead_provider")
local co = require("assay.contactout")

local gate = lp.gate({ approve = ..., meter = ... })
local c = co.client(gate, { token = os.getenv("CONTACTOUT_TOKEN") })

c:enrich_linkedin("https://www.linkedin.com/in/jchurch")   -- person | nil
c:profile_by_email("jc@cheaney.co.uk")                     -- reverse lookup
c:find_person({ first_name = "Jonathan", last_name = "Church", company = "Cheaney" })
c:resolve_email({ linkedin_url = "https://www.linkedin.com/in/jchurch" })  -- [email]
```

### The header

ContactOut wants the bare header name `token` — **not** `Authorization`, and **no** `Bearer` prefix.
A wrong header here comes back as an invalid key rather than a malformed request, which is a slow
thing to debug, so the tests pin it.

### Cost

`opts.cents` states what a call is worth to the caller; the gate decides whether it may proceed. The
default is `0` deliberately — a made-up price would meter fiction into the spend ledger. Pass the
real figure from your plan.

Records come back in the shared person/email shape with provenance. Emails are reported
work-address-first and carry `verification_status = "UNKNOWN"`: ContactOut returning an address is
not evidence that it delivers. Run it through [`email_verify:probe`](email_verify.html) for that.
