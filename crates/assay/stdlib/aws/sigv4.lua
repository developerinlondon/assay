--- @module assay.aws.sigv4
--- @description AWS Signature V4 request signing. Generates Authorization headers for AWS API calls.
--- @keywords aws, sigv4, signature, authorization, signing, v4, signature-v4
--- @quickref M.sign(opts) -> headers | Sign an AWS request and return authorization headers

local M = {}

local EMPTY_SHA256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"

local DAYS_IN_MONTH = { 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31 }

local function is_leap_year(y)
  return (y % 4 == 0 and y % 100 ~= 0) or (y % 400 == 0)
end

local function fmt02(n)
  if n < 10 then return "0" .. tostring(n) end
  return tostring(n)
end

local function utc_now()
  local epoch = os.time()
  local secs = epoch
  local days = math.floor(secs / 86400)
  local time_of_day = secs % 86400
  local hours = math.floor(time_of_day / 3600)
  local minutes = math.floor((time_of_day % 3600) / 60)
  local seconds = time_of_day % 60

  local year = 1970
  while true do
    local days_in_year = is_leap_year(year) and 366 or 365
    if days < days_in_year then break end
    days = days - days_in_year
    year = year + 1
  end

  local month = 1
  while month <= 12 do
    local dim = DAYS_IN_MONTH[month]
    if month == 2 and is_leap_year(year) then dim = 29 end
    if days < dim then break end
    days = days - dim
    month = month + 1
  end
  local day = days + 1

  return { year = year, month = month, day = day, hour = hours, min = minutes, sec = seconds }
end

local function url_encode(str)
  local encoded = str:gsub("([^A-Za-z0-9%-_.~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end)
  return encoded
end

local function url_encode_path(path)
  local parts = {}
  for segment in path:gmatch("[^/]+") do
    parts[#parts + 1] = url_encode(segment)
  end
  local result = "/" .. table.concat(parts, "/")
  if path:sub(-1) == "/" and #path > 1 then
    result = result .. "/"
  end
  return result
end

local function sort_query_params(query)
  if not query or query == "" then return "" end
  local params = {}
  for pair in query:gmatch("[^&]+") do
    params[#params + 1] = pair
  end
  table.sort(params)
  return table.concat(params, "&")
end

local function canonical_headers_and_signed(headers_map)
  local names = {}
  local lower_map = {}
  for k, v in pairs(headers_map) do
    local lk = k:lower()
    lower_map[lk] = tostring(v):match("^%s*(.-)%s*$")
    names[#names + 1] = lk
  end
  table.sort(names)
  local canonical = {}
  for _, n in ipairs(names) do
    canonical[#canonical + 1] = n .. ":" .. lower_map[n]
  end
  return table.concat(canonical, "\n") .. "\n", table.concat(names, ";")
end

local function utc_date_stamp(dt)
  return tostring(dt.year) .. fmt02(dt.month) .. fmt02(dt.day)
end

local function utc_datetime_stamp(dt)
  return utc_date_stamp(dt) .. "T" .. fmt02(dt.hour) .. fmt02(dt.min) .. fmt02(dt.sec) .. "Z"
end

--- Sign an AWS API request using Signature V4.
---
--- @param opts table with fields:
---   access_key  (string) AWS access key ID
---   secret_key  (string) AWS secret access key
---   session_token (string|nil) AWS session token (for STS credentials)
---   service     (string) AWS service name (e.g. "ecr", "ec2")
---   region      (string) AWS region
---   method      (string) HTTP method (GET, POST, etc.)
---   host        (string) API hostname
---   path        (string) Request path (default "/")
---   query       (string|nil) Query string
---   payload     (string) Request body
---   headers     (table|nil) Additional headers
--- @return table Headers map including Authorization, X-Amz-Date, etc.
function M.sign(opts)
  local access_key = opts.access_key
  local secret_key = opts.secret_key
  local session_token = opts.session_token
  local service = opts.service
  local region = opts.region
  local method = opts.method or "GET"
  local host = opts.host
  local path = opts.path or "/"
  local query = opts.query
  local payload = opts.payload or ""
  local extra_headers = opts.headers or {}

  local dt = utc_now()
  local date_stamp = utc_date_stamp(dt)
  local datetime_stamp = utc_datetime_stamp(dt)
  local credential_scope = date_stamp .. "/" .. region .. "/" .. service .. "/aws4_request"

  local payload_hash = crypto.hash(payload, "sha256")

  local headers_map = {}
  headers_map["host"] = host
  headers_map["x-amz-date"] = datetime_stamp
  if session_token and session_token ~= "" then
    headers_map["x-amz-security-token"] = session_token
  end
  headers_map["x-amz-content-sha256"] = payload_hash
  for k, v in pairs(extra_headers) do
    headers_map[k:lower()] = v
  end

  local canonical_headers_str, signed_headers = canonical_headers_and_signed(headers_map)
  local canonical_request = table.concat({
    method,
    url_encode_path(path),
    sort_query_params(query),
    canonical_headers_str,
    signed_headers,
    payload_hash,
  }, "\n")

  local string_to_sign = table.concat({
    "AWS4-HMAC-SHA256",
    datetime_stamp,
    credential_scope,
    crypto.hash(canonical_request, "sha256"),
  }, "\n")

  local date_key = crypto.hmac("AWS4" .. secret_key, date_stamp, "sha256", true)
  local region_key = crypto.hmac(date_key, region, "sha256", true)
  local service_key = crypto.hmac(region_key, service, "sha256", true)
  local signing_key = crypto.hmac(service_key, "aws4_request", "sha256", true)
  local signature = crypto.hmac(signing_key, string_to_sign, "sha256")

  headers_map["authorization"] = "AWS4-HMAC-SHA256 "
    .. "Credential=" .. access_key .. "/" .. credential_scope .. ","
    .. "SignedHeaders=" .. signed_headers .. ","
    .. "Signature=" .. signature

  return headers_map
end

return M
