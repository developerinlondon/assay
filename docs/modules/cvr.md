---
category: Registries
tagline: Denmark's CVR via the public cvrapi.dk gateway — keyless company lookup by name, VAT or phone
---

## assay.cvr

Danish (and Norwegian) entities from Det Centrale Virksomhedsregister, through the public
`cvrapi.dk` gateway. Keyless — but the gateway asks callers to identify themselves and throttles
those who do not, so `user_agent` is **required** and the client refuses to construct without one.
Same stance [`assay.edgar`](edgar.html) takes for the SEC's fair-access rule.

```lua
local cv = require("assay.cvr").client({ user_agent = "acme-crm (ops@acme.com)" })

cv:search("maersk")        -- company | nil, best match by name
cv:get("32345794")         -- company | nil, by CVR/VAT number
cv:by_phone("33633363")    -- company | nil, reverse lookup
```

`opts.country` defaults to `dk`; the gateway also serves `no`.

### Absence is not failure

The gateway signals "no such company" two different ways — a `404`, or a `200` carrying an `error`
body. Both return `nil`. A `429` raises, because being throttled is a failure and must not look like
an empty result.

### Dates

CVR reports `04/12 - 2013`. Passed through, that would sort and compare wrongly against every other
registry, so `founded_at` is converted to `2013-12-04`.

Records use the shared [`lead_provider`](lead_provider.html) company shape with
`provider = "registry:cvr"`.
