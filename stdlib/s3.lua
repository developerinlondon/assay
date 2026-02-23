--- @module assay.s3
--- @description S3-compatible object storage. Buckets, objects, copy, list with AWS Signature V4 auth.
--- @keywords s3, storage, buckets, objects, aws, minio, r2, sigv4, bucket, object, copy, metadata, signature-v4, compatible, cloudflare-r2
--- @quickref c:create_bucket(bucket) -> true | Create a new bucket
--- @quickref c:delete_bucket(bucket) -> true | Delete a bucket
--- @quickref c:list_buckets() -> [{name, creation_date}] | List all buckets
--- @quickref c:put_object(bucket, key, body, opts?) -> true | Upload an object
--- @quickref c:get_object(bucket, key) -> string|nil | Download object content
--- @quickref c:delete_object(bucket, key) -> true | Delete an object
--- @quickref c:list_objects(bucket, opts?) -> {objects, is_truncated} | List objects in bucket
--- @quickref c:head_object(bucket, key) -> {status, headers}|nil | Get object metadata
--- @quickref c:copy_object(src_bucket, src_key, dst_bucket, dst_key) -> true | Copy object between buckets
--- @quickref c:bucket_exists(bucket) -> bool | Check if bucket exists

local M = {}

local EMPTY_SHA256 = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"

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

local DAYS_IN_MONTH = { 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31 }

local function is_leap_year(y)
  return (y % 4 == 0 and y % 100 ~= 0) or (y % 400 == 0)
end

local function epoch_to_utc(epoch)
  local secs = math.floor(epoch)
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

  return {
    year = year, month = month, day = day,
    hour = hours, min = minutes, sec = seconds,
  }
end

local function fmt02(n)
  if n < 10 then return "0" .. tostring(n) end
  return tostring(n)
end

local function utc_date_stamp(dt)
  return tostring(dt.year) .. fmt02(dt.month) .. fmt02(dt.day)
end

local function utc_datetime_stamp(dt)
  return utc_date_stamp(dt) .. "T" .. fmt02(dt.hour) .. fmt02(dt.min) .. fmt02(dt.sec) .. "Z"
end

local function trim(s)
  return s:match("^%s*(.-)%s*$")
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
    lower_map[lk] = trim(tostring(v))
    names[#names + 1] = lk
  end
  table.sort(names)
  local canonical = {}
  for _, n in ipairs(names) do
    canonical[#canonical + 1] = n .. ":" .. lower_map[n]
  end
  return table.concat(canonical, "\n") .. "\n", table.concat(names, ";")
end

local function sign(secret_key, region, dt, method, path, query, headers_map, payload_hash)
  local date_stamp = utc_date_stamp(dt)
  local datetime_stamp = utc_datetime_stamp(dt)
  local credential_scope = date_stamp .. "/" .. region .. "/s3/aws4_request"

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
  local service_key = crypto.hmac(region_key, "s3", "sha256", true)
  local signing_key = crypto.hmac(service_key, "aws4_request", "sha256", true)
  local signature = crypto.hmac(signing_key, string_to_sign, "sha256")

  return signature, signed_headers, credential_scope, datetime_stamp
end

local function xml_extract(body, tag)
  return body:match("<" .. tag .. ">(.-)</" .. tag .. ">")
end

local function xml_extract_all(body, tag)
  local results = {}
  for val in body:gmatch("<" .. tag .. ">(.-)</" .. tag .. ">") do
    results[#results + 1] = val
  end
  return results
end

function M.client(opts)
  opts = opts or {}
  local endpoint = opts.endpoint
  if not endpoint then error("s3.client: endpoint is required") end
  local region = opts.region
  if not region then error("s3.client: region is required") end
  local access_key = opts.access_key
  if not access_key then error("s3.client: access_key is required") end
  local secret_key = opts.secret_key
  if not secret_key then error("s3.client: secret_key is required") end
  local path_style = opts.path_style
  if path_style == nil then path_style = true end

  endpoint = endpoint:gsub("/+$", "")

  local c = {
    endpoint = endpoint,
    region = region,
    access_key = access_key,
    secret_key = secret_key,
    path_style = path_style,
  }

  local function make_url(self, bucket, key)
    if self.path_style then
      local url = self.endpoint
      if bucket then url = url .. "/" .. bucket end
      if key then url = url .. "/" .. key end
      return url
    else
      if bucket then
        local base = self.endpoint:gsub("^(https?://)", "%1" .. bucket .. ".")
        if key then return base .. "/" .. key end
        return base
      end
      return self.endpoint
    end
  end

  local function build_headers(self, method_str, bucket, key, query, payload_hash, extra_headers)
    local dt = epoch_to_utc(time())
    local datetime_stamp = utc_datetime_stamp(dt)

    local path = "/"
    if self.path_style then
      if bucket then path = path .. bucket end
      if key then path = path .. "/" .. key end
    else
      if key then path = path .. key end
    end

    local host
    if self.path_style then
      host = self.endpoint:gsub("^https?://", "")
    else
      if bucket then
        host = bucket .. "." .. self.endpoint:gsub("^https?://", "")
      else
        host = self.endpoint:gsub("^https?://", "")
      end
    end

    local headers_map = {
      host = host,
      ["x-amz-date"] = datetime_stamp,
      ["x-amz-content-sha256"] = payload_hash,
    }
    if extra_headers then
      for k, v in pairs(extra_headers) do
        headers_map[k:lower()] = v
      end
    end

    local signature, signed_headers, credential_scope = sign(
      self.secret_key, self.region, dt, method_str,
      path, query or "", headers_map, payload_hash
    )

    local auth = "AWS4-HMAC-SHA256 Credential=" .. self.access_key .. "/" .. credential_scope
      .. ", SignedHeaders=" .. signed_headers
      .. ", Signature=" .. signature

    local req_headers = {
      ["Authorization"] = auth,
      ["x-amz-date"] = datetime_stamp,
      ["x-amz-content-sha256"] = payload_hash,
    }
    if extra_headers then
      for k, v in pairs(extra_headers) do
        req_headers[k] = v
      end
    end

    return req_headers
  end

  function c:create_bucket(bucket)
    local body = ""
    if self.region ~= "us-east-1" then
      body = '<?xml version="1.0" encoding="UTF-8"?>'
        .. '<CreateBucketConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/">'
        .. '<LocationConstraint>' .. self.region .. '</LocationConstraint>'
        .. '</CreateBucketConfiguration>'
    end
    local payload_hash = crypto.hash(body, "sha256")
    local hdrs = build_headers(self, "PUT", bucket, nil, nil, payload_hash, {
      ["content-type"] = "application/xml",
    })
    local url = make_url(self, bucket)
    local resp = http.put(url, body, { headers = hdrs })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("s3: PUT /" .. bucket .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return true
  end

  function c:delete_bucket(bucket)
    local hdrs = build_headers(self, "DELETE", bucket, nil, nil, EMPTY_SHA256)
    local url = make_url(self, bucket)
    local resp = http.delete(url, { headers = hdrs })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("s3: DELETE /" .. bucket .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return true
  end

  function c:list_buckets()
    local hdrs = build_headers(self, "GET", nil, nil, nil, EMPTY_SHA256)
    local url = make_url(self)
    local resp = http.get(url, { headers = hdrs })
    if resp.status ~= 200 then
      error("s3: GET / HTTP " .. resp.status .. ": " .. resp.body)
    end
    local buckets = {}
    for block in resp.body:gmatch("<Bucket>(.-)</Bucket>") do
      local name = xml_extract(block, "Name")
      local creation_date = xml_extract(block, "CreationDate")
      if name then
        buckets[#buckets + 1] = { name = name, creation_date = creation_date }
      end
    end
    return buckets
  end

  function c:put_object(bucket, key, body, put_opts)
    put_opts = put_opts or {}
    body = body or ""
    local payload_hash = crypto.hash(body, "sha256")
    local extra = {}
    if put_opts.content_type then
      extra["content-type"] = put_opts.content_type
    end
    local hdrs = build_headers(self, "PUT", bucket, key, nil, payload_hash, extra)
    local url = make_url(self, bucket, key)
    local resp = http.put(url, body, { headers = hdrs })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("s3: PUT /" .. bucket .. "/" .. key .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return true
  end

  function c:get_object(bucket, key)
    local hdrs = build_headers(self, "GET", bucket, key, nil, EMPTY_SHA256)
    local url = make_url(self, bucket, key)
    local resp = http.get(url, { headers = hdrs })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("s3: GET /" .. bucket .. "/" .. key .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return resp.body
  end

  function c:delete_object(bucket, key)
    local hdrs = build_headers(self, "DELETE", bucket, key, nil, EMPTY_SHA256)
    local url = make_url(self, bucket, key)
    local resp = http.delete(url, { headers = hdrs })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("s3: DELETE /" .. bucket .. "/" .. key .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return true
  end

  function c:list_objects(bucket, list_opts)
    list_opts = list_opts or {}
    local query_parts = { "list-type=2" }
    if list_opts.prefix then
      query_parts[#query_parts + 1] = "prefix=" .. url_encode(list_opts.prefix)
    end
    if list_opts.max_keys then
      query_parts[#query_parts + 1] = "max-keys=" .. tostring(list_opts.max_keys)
    end
    if list_opts.continuation_token then
      query_parts[#query_parts + 1] = "continuation-token=" .. url_encode(list_opts.continuation_token)
    end
    local query = table.concat(query_parts, "&")
    local hdrs = build_headers(self, "GET", bucket, nil, query, EMPTY_SHA256)
    local url = make_url(self, bucket) .. "?" .. query
    local resp = http.get(url, { headers = hdrs })
    if resp.status ~= 200 then
      error("s3: GET /" .. bucket .. "?list-type=2 HTTP " .. resp.status .. ": " .. resp.body)
    end
    local objects = {}
    for block in resp.body:gmatch("<Contents>(.-)</Contents>") do
      local obj_key = xml_extract(block, "Key")
      local size = xml_extract(block, "Size")
      local last_modified = xml_extract(block, "LastModified")
      if obj_key then
        objects[#objects + 1] = {
          key = obj_key,
          size = size and tonumber(size) or 0,
          last_modified = last_modified,
        }
      end
    end
    local result = { objects = objects }
    result.is_truncated = xml_extract(resp.body, "IsTruncated") == "true"
    result.next_continuation_token = xml_extract(resp.body, "NextContinuationToken")
    result.key_count = tonumber(xml_extract(resp.body, "KeyCount") or "0")
    return result
  end

  function c:head_object(bucket, key)
    local hdrs = build_headers(self, "GET", bucket, key, nil, EMPTY_SHA256)
    local url = make_url(self, bucket, key)
    local resp = http.get(url, { headers = hdrs })
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("s3: HEAD /" .. bucket .. "/" .. key .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return { status = resp.status, headers = resp.headers }
  end

  function c:copy_object(src_bucket, src_key, dst_bucket, dst_key)
    local copy_source = "/" .. src_bucket .. "/" .. src_key
    local hdrs = build_headers(self, "PUT", dst_bucket, dst_key, nil, EMPTY_SHA256, {
      ["x-amz-copy-source"] = copy_source,
    })
    local url = make_url(self, dst_bucket, dst_key)
    local resp = http.put(url, "", { headers = hdrs })
    if resp.status ~= 200 then
      error("s3: COPY " .. copy_source .. " -> /" .. dst_bucket .. "/" .. dst_key
        .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return true
  end

  function c:bucket_exists(bucket)
    local query = "list-type=2&max-keys=0"
    local hdrs = build_headers(self, "GET", bucket, nil, query, EMPTY_SHA256)
    local url = make_url(self, bucket) .. "?" .. query
    local resp = http.get(url, { headers = hdrs })
    return resp.status == 200
  end

  return c
end

return M
