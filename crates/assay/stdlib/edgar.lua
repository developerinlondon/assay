--- @module assay.edgar
--- @description SEC EDGAR — US public-company registry: ticker/name lookup, company submissions (SIC, addresses, filings), and full-text search over filings. Free; the SEC requires an identifying User-Agent.
--- @category Cloud & AWS
--- @icon building
--- @keywords edgar, sec, cik, filings, ticker, company, registry, prospect, 10-K
--- @quickref M.client(opts) -> c | Client; user_agent required (or EDGAR_USER_AGENT)
--- @quickref c:tickers() -> [company] | Every registered ticker: cik, ticker, name
--- @quickref c:find(name) -> [company] | Ticker-table rows whose name contains the query
--- @quickref c:submissions(cik) -> company|nil | SIC, tickers, addresses, recent filing counts
--- @quickref c:fulltext(q, opts?) -> [hit] | Full-text search over filings

local M = {}

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

local function urlencode(str)
  return tostring(str):gsub("([^%w%-%.%_%~ ])", function(ch)
    return string.format("%%%02X", string.byte(ch))
  end):gsub(" ", "%%20")
end

-- data.sec.gov keys submissions by ten-digit, zero-padded CIK.
local function pad_cik(cik)
  local digits = tostring(cik):gsub("%D", "")
  return string.rep("0", 10 - #digits) .. digits
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
    local url = base .. path_str
    local resp = http.get(url, { headers = { ["User-Agent"] = user_agent, Accept = "application/json" } })
    if resp.status == 404 then return nil, url end
    if resp.status ~= 200 then
      error("edgar: GET " .. path_str .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
    end
    return json.parse(resp.body), url
  end

  local c = {}

  function c:tickers()
    local body, url = api_get(www_url, "/files/company_tickers.json")
    local out = {}
    for _, row in pairs(body or {}) do
      out[#out + 1] = {
        cik = row.cik_str,
        ticker = row.ticker,
        name = row.title,
        provenance = { provider = "registry:edgar", retrieved_from = url },
      }
    end
    table.sort(out, function(x, y) return tostring(x.name) < tostring(y.name) end)
    return out
  end

  function c:find(name)
    local needle = trim(name):lower()
    local out = {}
    for _, row in ipairs(self:tickers()) do
      if tostring(row.name):lower():find(needle, 1, true) then out[#out + 1] = row end
    end
    return out
  end

  function c:submissions(cik)
    local body, url = api_get(data_url, "/submissions/CIK" .. pad_cik(cik) .. ".json")
    if not body then return nil end
    local recent = (body.filings or {}).recent or {}
    return {
      cik = body.cik,
      name = body.name,
      sic = body.sic,
      sic_description = body.sicDescription,
      tickers = body.tickers,
      exchanges = body.exchanges,
      website = body.website,
      addresses = body.addresses,
      recent_filing_count = recent.form and #recent.form or 0,
      provenance = { provider = "registry:edgar", retrieved_from = url },
    }
  end

  function c:fulltext(q, o)
    o = o or {}
    local path_str = "/LATEST/search-index?q=" .. urlencode(trim(q))
    if o.forms then path_str = path_str .. "&forms=" .. urlencode(o.forms) end
    if o.date_range then path_str = path_str .. "&dateRange=" .. urlencode(o.date_range) end
    local body, url = api_get(efts_url, path_str)
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
        provenance = { provider = "registry:edgar", retrieved_from = url },
      }
    end
    return out
  end

  return c
end

return M
