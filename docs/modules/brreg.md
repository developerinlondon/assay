---
category: Registries
tagline: Norway's Enhetsregisteret — keyless company data with websites and live employee counts
---

## assay.brreg

The Brønnøysund register, free and keyless. Two fields make it unusually useful for outbound work:
it publishes the company's **website** and a **live employee count**, neither of which most national
registries expose.

```lua
local br = require("assay.brreg").client()

br:search("equinor", { limit = 10 })   -- [company]
br:get("923609016")                    -- company | nil
br:by_website("equinor.com")           -- [company] — the reverse join
br:sub_entities("923609016")           -- [company] — site-level rows
```

### The reverse join is the point

`by_website` answers _"which legal entity owns this domain?"_ — the join a prospect list actually
needs, since the list holds domains and the registry holds companies. The registry stores whatever
the entity registered, so a lookup is retried with the `www.` form before giving up, and inputs are
reduced to a bare host first (`https://EQUINOR.com/careers` → `equinor.com`).

### Normalisation worth knowing

| Registry reality                                                         | What the module returns                                                          |
| ------------------------------------------------------------------------ | -------------------------------------------------------------------------------- |
| `konkurs` / `underAvvikling` / `underTvangsavvikling…` as three booleans | one `status`: `BANKRUPT`, `LIQUIDATING`, `COMPULSORY_LIQUIDATION`, else `ACTIVE` |
| `antallAnsatte` present even when nobody reported                        | `employees` is `nil` unless `harRegistrertAntallAnsatte` is true                 |
| no hits omits `_embedded` entirely                                       | an empty list, not an error                                                      |
| website as `www.x.no`, `https://x.no/`, `X.NO`                           | `domain` as a bare lowercase host                                                |

Zero employees and "nobody reported" are different claims; only one of them should survive into a
record someone later acts on.

Records use the shared [`lead_provider`](lead_provider.html) company shape with
`provider = "registry:brreg"`, so a fact looked up for nothing and a fact bought from a vendor are
indistinguishable downstream except by provenance.
