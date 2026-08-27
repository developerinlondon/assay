---
category: Registries
tagline: GLEIF LEI registry — global legal-entity search, keyless and free
---

## assay.gleif

The Global Legal Entity Identifier Foundation's open API: search legal entities by name across
every jurisdiction, fetch one record by LEI, and fuzzy-complete partial names. No key, no
account — the everywhere-fallback of the registry family. Every record carries provenance
(`provider = "registry:gleif"`, the exact URL fetched), so facts sourced here stay auditable
downstream.

### Client

```lua
local gleif = require("assay.gleif")
local c = gleif.client()
```

`gleif.client(opts)` accepts `base_url` (default `https://api.gleif.org/api/v1`; override for a
test double).

### Search and fetch

```lua
local matches = c:search("Joseph Cheaney", { limit = 5 })
-- { { lei, name, status, jurisdiction, legal_form, city, country, registered_at, provenance }, … }

local names = c:fuzzy("Cheaney")       -- completion strings for a partial name
local one = c:get("529900T8BM49AURSDO55")  -- one record, or nil when the LEI is unknown
```

`search` filters on the exact legal name (GLEIF matches case-insensitively and on word
boundaries); use `fuzzy` first when the spelling is uncertain. Results are normalized to the
flat registry shape shared by the registry modules — downstream code never learns GLEIF's
JSON:API envelope.
