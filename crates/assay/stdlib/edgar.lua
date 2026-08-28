--- @module assay.edgar
--- @description SEC EDGAR — US public-company registry: ticker/name lookup, company submissions (SIC, addresses, filings), and full-text search over filings. Free; the SEC requires an identifying User-Agent.
--- @category registries
--- @icon building
--- @keywords edgar, sec, cik, filings, ticker, company, registry, prospect, 10-K
--- @quickref M.client(opts) -> c | Client; user_agent required (or EDGAR_USER_AGENT)
--- @quickref c:tickers() -> [row] | The raw ticker index: cik, ticker, name
--- @quickref c:find(name) -> [company] | Index rows whose name contains the query
--- @quickref c:submissions(cik) -> company|nil | SIC, tickers, addresses, recent filing counts
--- @quickref c:fulltext(q, opts?) -> [hit] | Full-text search over filings

local M = {}

local lp = require("assay.lead_provider")
local url = require("assay.url")

local PROVIDER = "registry:edgar"

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

-- data.sec.gov keys submissions by ten-digit, zero-padded CIK.
local function pad_cik(cik)
  local digits = tostring(cik):gsub("%D", "")
  return string.rep("0", 10 - #digits) .. digits
end

-- The ticker index reports CIK 320193 and submissions reports "0000320193" for
-- the same company. Left alone the module hands out two different identities
-- for one entity, so both are reduced to the unpadded form a CRM stores.
local function plain_cik(cik)
  local digits = tostring(cik or ""):gsub("%D", ""):gsub("^0+", "")
  return digits ~= "" and digits or nil
end

-- EDGAR does not publish a country for domestic filers: it puts a US state
-- code in `stateOrCountry` and leaves `country` null, so reading the former as
-- a country turns Apple's California into Canada. Foreign filers invert this
-- and carry free text, which is passed through for want of a code.
local function country_of(business)
  if business.country and business.country ~= "" then return business.country end
  return business.stateOrCountry and "US" or nil
end

function M.client(opts)
  opts = opts or {}
  local user_agent = opts.user_agent or env.get("EDGAR_USER_AGENT")
  if not user_agent or trim(user_agent) == "" then
    error("edgar: a contact-identifying user_agent is required (SEC fair-access policy) — "
      .. "pass opts.user_agent or set EDGAR_USER_AGENT")
  end
  -- Three hosts, one API: bulk files on www, structured data on data.,
  -- full-text search on efts. Each overridable so tests point all at one mock.
  local www_url = (opts.www_url or "https://www.sec.gov"):gsub("/+$", "")
  local data_url = (opts.data_url or "https://data.sec.gov"):gsub("/+$", "")
  local efts_url = (opts.efts_url or "https://efts.sec.gov"):gsub("/+$", "")

  local function api_get(base, path_str)
    local target = base .. path_str
    local resp = http.get(target, { headers = { ["User-Agent"] = user_agent, Accept = "application/json" } })
    if resp.status == 404 then return nil, target end
    if resp.status ~= 200 then
      error("edgar: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return json.parse(resp.body), target
  end

  local c = {}

  --- The raw ticker index: ten thousand rows, so it stays a lightweight
  --- three-field row sharing one provenance table rather than ten thousand
  --- separately stamped company records. `find` is what promotes a match.
  function c:tickers()
    if self._tickers then return self._tickers end
    local body, from = api_get(www_url, "/files/company_tickers.json")
    local out = {}
    local provenance = lp.provenance(PROVIDER, from)
    for _, row in pairs(body or {}) do
      out[#out + 1] = { cik = row.cik_str, ticker = row.ticker, name = row.title, provenance = provenance }
    end
    table.sort(out, function(x, y) return tostring(x.name) < tostring(y.name) end)
    self._tickers = out
    return out
  end

  function c:find(name)
    local needle = trim(name):lower()
    if needle == "" then error("edgar: find requires a non-empty name") end
    local out = {}
    for _, row in ipairs(self:tickers()) do
      if tostring(row.name):lower():find(needle, 1, true) then
        local record = lp.company(PROVIDER, row.provenance.retrieved_from, {
          registry_id = plain_cik(row.cik),
          name = row.name,
          jurisdiction = "US",
          country = "US",
        })
        record.cik = row.cik
        record.ticker = row.ticker
        out[#out + 1] = record
      end
    end
    return out
  end

  function c:submissions(cik)
    local body, from = api_get(data_url, "/submissions/CIK" .. pad_cik(cik) .. ".json")
    if not body then return nil end
    local recent = (body.filings or {}).recent or {}
    local business = (body.addresses or {}).business or {}
    local record = lp.company(PROVIDER, from, {
      registry_id = plain_cik(body.cik),
      name = body.name,
      -- EDGAR ships `website` as an empty string for the many registrants that
      -- publish none, which would otherwise reach a prospect list as a blank
      -- claim rather than an absent one.
      domain = lp.bare_domain(body.website),
      legal_form = body.entityType,
      jurisdiction = body.stateOfIncorporation,
      city = business.city,
      country = country_of(business),
      industry = body.sicDescription,
      industry_code = body.sic,
      phone = body.phone,
    })
    record.cik = body.cik
    record.tickers = body.tickers
    record.exchanges = body.exchanges
    record.addresses = body.addresses
    record.lei = body.lei
    record.recent_filing_count = recent.form and #recent.form or 0
    return record
  end

  function c:fulltext(q, o)
    o = o or {}
    local path_str = "/LATEST/search-index?q=" .. url.encode(trim(q))
    if o.forms then path_str = path_str .. "&forms=" .. url.encode(o.forms) end
    if o.date_range then path_str = path_str .. "&dateRange=" .. url.encode(o.date_range) end
    local body, from = api_get(efts_url, path_str)
    local out = {}
    local hits = body and body.hits and body.hits.hits or {}
    for _, hit in ipairs(hits) do
      local src = hit._source or {}
      out[#out + 1] = {
        id = hit._id,
        form = src.form or (src.file_type),
        filed_at = src.file_date,
        company = src.display_names and src.display_names[1] or nil,
        ciks = src.ciks,
        provenance = lp.provenance(PROVIDER, from),
      }
    end
    return out
  end

  return c
end

return M
