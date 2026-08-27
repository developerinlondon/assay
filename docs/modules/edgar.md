---
category: Cloud & AWS
tagline: SEC EDGAR — US public-company registry, filings and full-text search
---

## assay.edgar

The SEC's EDGAR system: the full registered-ticker table, per-company submissions (SIC code,
tickers, exchanges, addresses, recent filings), and full-text search over filings. Free, but
the SEC's fair-access policy requires an identifying `User-Agent` — the client refuses to
construct without one. Records carry provenance (`provider = "registry:edgar"`).

### Client

```lua
local edgar = require("assay.edgar")
local c = edgar.client({ user_agent = "my-app contact@example.com" })
```

`edgar.client(opts)` accepts:

- `user_agent` — required, or set `EDGAR_USER_AGENT`. Identify yourself with a contact address.
- `www_url` / `data_url` / `efts_url` — the three EDGAR hosts (bulk files, structured data,
  full-text search); each overridable for tests.

### Lookups

```lua
local all = c:tickers()                  -- every registered ticker: { cik, ticker, name }
local hits = c:find("apple")             -- ticker rows whose name contains the query
local co = c:submissions(320193)         -- SIC, tickers, exchanges, addresses, filing counts
local docs = c:fulltext("supply chain", { forms = "10-K" })  -- filing search hits
```

`submissions` pads the CIK to EDGAR's ten-digit form for you. `fulltext` accepts `forms`
(comma-separated form types) and `date_range`. Only companies that file with the SEC appear —
EDGAR is the *public-company* half of the US market; state registries cover the rest.
