--- @module assay.aws.s3
--- @description AWS S3 read-only queries via Signature V4. List buckets, list objects, head object.
--- @category cloud
--- @keywords aws, s3, buckets, objects, list, head, sigv4, storage, path-style
--- @quickref client(opts) -> client | Create an S3 client (opts = {access_key?, secret_key?, profile?, role_arn?, region?, session_token?, endpoint?}; no keys -> standard chain via assay.aws.sts)
--- @quickref c:list_buckets() -> [{name, creation_date}] | List buckets (GET, read)
--- @quickref c:list_objects(bucket, opts?) -> {objects, is_truncated, ...} | List objects (GET, read)
--- @quickref c:head_object(bucket, key) -> {status, headers}|nil | Object metadata, nil if absent (GET, read)

local M = {}

local sigv4 = require("assay.aws.sigv4")

local function encode(value)
  local out = tostring(value):gsub("([^A-Za-z0-9%-_.~])", function(ch)
    return string.format("%%%02X", string.byte(ch))
  end)
  return out
end

local function xml_first(body, tag)
  return body:match("<" .. tag .. ">(.-)</" .. tag .. ">")
end

function M.client(opts)
  opts = opts or {}
  -- No explicit keys → resolve via the standard chain (profile / env / IRSA),
  -- honoring opts.profile and opts.role_arn. See assay.aws.sts.
  local creds = opts
  if not opts.access_key then
    creds = require("assay.aws.sts").credentials(opts)
  end
  local access_key = creds.access_key or error("aws.s3.client: access_key is required")
  local secret_key = creds.secret_key or error("aws.s3.client: secret_key is required")
  local region = opts.region or env.get("AWS_REGION") or env.get("AWS_DEFAULT_REGION")
    or error("aws.s3.client: region is required")
  local session_token = creds.session_token
  local endpoint = opts.endpoint

  local url, host
  if endpoint and endpoint ~= "" then
    if endpoint:match("^https?://") then
      url = endpoint:gsub("/+$", "")
      host = url:gsub("^https?://", "")
    else
      host = endpoint
      url = "https://" .. host
    end
  else
    host = "s3." .. region .. ".amazonaws.com"
    url = "https://" .. host
  end

  local function get(path, query)
    local signed = sigv4.sign({
      access_key = access_key,
      secret_key = secret_key,
      session_token = session_token,
      service = "s3",
      region = region,
      method = "GET",
      host = host,
      path = path,
      query = query or "",
      payload = "",
    })
    local u = url .. path
    if query and query ~= "" then u = u .. "?" .. query end
    return http.get(u, { headers = signed })
  end

  local c = {}

  function c:list_buckets()
    local resp = get("/", nil)
    if resp.status ~= 200 then
      error("aws.s3: GET / HTTP " .. resp.status .. ": " .. resp.body)
    end
    local out = {}
    for block in resp.body:gmatch("<Bucket>(.-)</Bucket>") do
      out[#out + 1] = {
        name = xml_first(block, "Name"),
        creation_date = xml_first(block, "CreationDate"),
      }
    end
    return out
  end

  function c:list_objects(bucket, list_opts)
    list_opts = list_opts or {}
    local parts = { "list-type=2" }
    if list_opts.prefix then
      parts[#parts + 1] = "prefix=" .. encode(list_opts.prefix)
    end
    if list_opts.max_keys then
      parts[#parts + 1] = "max-keys=" .. tostring(list_opts.max_keys)
    end
    if list_opts.continuation_token then
      parts[#parts + 1] = "continuation-token=" .. encode(list_opts.continuation_token)
    end
    table.sort(parts)
    local query = table.concat(parts, "&")

    local resp = get("/" .. bucket, query)
    if resp.status ~= 200 then
      error("aws.s3: GET /" .. bucket .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    local objects = {}
    for block in resp.body:gmatch("<Contents>(.-)</Contents>") do
      local size = xml_first(block, "Size")
      objects[#objects + 1] = {
        key = xml_first(block, "Key"),
        size = size and tonumber(size) or 0,
        last_modified = xml_first(block, "LastModified"),
        etag = xml_first(block, "ETag"),
      }
    end
    return {
      objects = objects,
      is_truncated = xml_first(resp.body, "IsTruncated") == "true",
      next_continuation_token = xml_first(resp.body, "NextContinuationToken"),
      key_count = tonumber(xml_first(resp.body, "KeyCount") or "0"),
    }
  end

  function c:head_object(bucket, key)
    local resp = get("/" .. bucket .. "/" .. key, nil)
    if resp.status == 404 then return nil end
    if resp.status ~= 200 then
      error("aws.s3: HEAD /" .. bucket .. "/" .. key .. " HTTP " .. resp.status .. ": " .. resp.body)
    end
    return { status = resp.status, headers = resp.headers }
  end

  return c
end

return M
