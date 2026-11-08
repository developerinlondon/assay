---
category: Registries
tagline: Reoon email verification — budget-gated verify_email plus a free balance read
---

## assay.reoon

A paid `verify_email` rung. The [budget gate](lead_provider.html) is the first argument and is not
optional, because every verification spends a credit.

```lua
local lp = require("assay.lead_provider")
local reoon = require("assay.reoon")

local gate = lp.gate({ approve = ..., meter = ... })
local c = reoon.client(gate, { api_key = env.get("REOON_KEY") })

c:verify_email("jo@shop.example", { cents = 1 })  -- lead_provider email record | nil, reason
c:balance()                                        -- { api_status, daily, instant }, free
reoon.verdict("safe", false)                       -- "PROBABLE"
```

Options: `api_key` (or `REOON_KEY`), `base_url`, `mode` (`power` by default; `quick` skips the SMTP
check), `timeout` in seconds (75 by default; power mode can take up to a minute).

### Verdicts

| Reoon `status`                                        | `verification_status` |
| ----------------------------------------------------- | --------------------- |
| `safe`, `valid`                                       | `PROBABLE`            |
| `catch_all`, or `is_catch_all = true` on any status   | `CATCH_ALL`           |
| `invalid`, `disabled`, `spamtrap`, `disposable`       | `INVALID`             |
| `role_account`, `inbox_full`, `unknown`, anything new | `UNKNOWN`             |

`safe` is the vendor's opinion of an address, not a delivery, so it never becomes `VERIFIED`. The
raw status rides along as `vendor_result`, with `confidence` (Reoon's `overall_score`),
`is_safe_to_send`, `is_deliverable`, `mx_accepts_mail` and `can_connect_smtp`.

### The key

Reoon takes the key as a query parameter. The module scrubs it from every error it raises, including
transport errors that quote the request URL, and records provenance against the endpoint without the
query string. A refusal carries the vendor's own `reason` (`reoon: verify HTTP 402: …`), so an
exhausted balance does not read as an outage.
