---
category: Registries
tagline: UK Companies House — every registered UK company and its officers, by name, number or director
---

## assay.companies_house

The UK register: company profiles, name search, and the officers a profile does not name. Companies
House is free but issues a key per caller, which makes it the one registry module here that
authenticates. The key is self-service from
[the developer hub](https://developer.company-information.service.gov.uk/).

```lua
local ch = require("assay.companies_house").client({ api_key = "..." })  -- or COMPANIES_HOUSE_KEY

ch:search("tesco", { limit = 20 })   -- [company], name search
ch:get("00445790")                   -- company | nil, full profile
ch:officers("00445790")              -- [person], directors and secretaries
ch:officers("00445790", { active_only = true })
```

### The key is required at construction

The API rejects an unauthenticated call, so the client refuses to build without a key rather than
failing later with a `401` that reads like an outage. Same stance [`assay.cvr`](cvr.html) and
[`assay.edgar`](edgar.html) take on identifying the caller.

Authentication is HTTP Basic with the key as the **username and an empty password**. The trailing
colon is load-bearing — omitting it authenticates as a user named after the key, which the API
rejects as a bad key rather than a malformed request.

### Two endpoints, two names for the same field

Search hits and company profiles describe the same entity under different field names, which is the
single easiest way to get a record with a nil name:

| Meaning      | `/search/companies` | `/company/{number}`         |
| ------------ | ------------------- | --------------------------- |
| Company name | `title`             | `company_name`              |
| Legal form   | `company_type`      | `type`                      |
| Address      | `address`           | `registered_office_address` |

Both are read, so a record from either endpoint is the same shape.

### Status

Companies House publishes twelve statuses where a caller asks one question — can this company still
be approached. They bucket into the vocabulary the other registry modules already answer in, so a UK
row and a Norwegian row compare directly:

| Registry status                                                                                    | `status`      |
| -------------------------------------------------------------------------------------------------- | ------------- |
| `active`, `open`, `registered`                                                                     | `ACTIVE`      |
| `dissolved`, `closed`, `removed`, `converted-closed`                                               | `CLOSED`      |
| `liquidation`, `receivership`, `administration`, `voluntary-arrangement`, `insolvency-proceedings` | `LIQUIDATING` |

A status the registry adds later is upper-cased rather than dropped. Mapping an unknown value to
`ACTIVE` would be the one wrong answer, so it never happens.

### Officers

`officers` is the reason to reach a UK registry for outreach at all: the profile names the company,
this names the person to write to. Records use the shared [`lead_provider`](lead_provider.html)
person shape, plus `officer_role`, `appointed_on`, `resigned_on`, `nationality`, `born_at` and
`active`.

The list mixes serving and departed officers and the distinction is carried solely by a resignation
date, so `active_only = true` is usually what you want. Companies House withholds an officer's day
of birth by design; `born_at` is therefore a partial `YYYY-MM`, not a truncated day.

### Absence, rejection and throttling

An unknown company number returns `nil`. A rejected key (`401`/`403`) and a throttle (`429`) both
raise, with distinct messages — being refused is a failure and must not look like an empty result.
The published limit is 600 requests per five minutes.

### What Companies House does not publish

No website and no employee count, so `domain` and `employees` are always nil. For the UK,
[`assay.gleif`](gleif.html) can supply an LEI and [`assay.brreg`](brreg.html) is the registry to
copy for the domain join — Companies House simply does not hold one.

Records use the shared [`lead_provider`](lead_provider.html) company shape with
`provider = "registry:companies_house"`.
