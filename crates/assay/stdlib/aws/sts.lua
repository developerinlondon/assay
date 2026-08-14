--- @module assay.aws.sts
--- @description AWS credential resolution + STS. Resolves credentials like the AWS CLI (explicit -> profile -> env -> IRSA web identity), assumes roles, all in-process (no aws CLI needed, works in readonly mode).
--- @category cloud
--- @keywords aws, sts, credentials, assume-role, web-identity, irsa, profile, aws-profile, credential-chain
--- @env AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, AWS_SESSION_TOKEN, AWS_PROFILE, AWS_ROLE_ARN, AWS_WEB_IDENTITY_TOKEN_FILE, AWS_REGION, AWS_DEFAULT_REGION, HOME
--- @quickref M.credentials(opts?) -> {access_key, secret_key, session_token?} | Resolve credentials via the standard chain
--- @quickref M.assume_role(role_arn, opts?) -> creds | Assume an IAM role (sts:AssumeRole)
--- @quickref M.assume_role_with_web_identity(role_arn, token, opts?) -> creds | Assume a role with a web identity token (IRSA)

local sigv4 = require("assay.aws.sigv4")

local M = {}

local DEFAULT_DURATION = 3600
local EXPIRY_SKEW_SECS = 120
local MAX_PROFILE_DEPTH = 5

-- Resolved-credential cache, keyed by how they were obtained. Entries carry
-- expires_at (epoch) for temporary credentials; static entries never expire.
local _cache = {}

local function region_of(opts)
  return (opts and opts.region)
    or env.get("AWS_REGION")
    or env.get("AWS_DEFAULT_REGION")
    or "us-east-1"
end

local function sts_url(opts)
  if opts and opts.sts_endpoint then
    return opts.sts_endpoint:gsub("/+$", "")
  end
  return "https://sts." .. region_of(opts) .. ".amazonaws.com"
end

local function url_encode(str)
  return (tostring(str):gsub("([^A-Za-z0-9%-_.~])", function(c)
    return string.format("%%%02X", string.byte(c))
  end))
end

local function form_encode(params)
  local names = {}
  for k in pairs(params) do names[#names + 1] = k end
  table.sort(names)
  local parts = {}
  for _, k in ipairs(names) do
    parts[#parts + 1] = url_encode(k) .. "=" .. url_encode(params[k])
  end
  return table.concat(parts, "&")
end

-- STS answers the query protocol in JSON when asked (Accept header), sparing
-- us an XML parser. Expiration arrives as epoch seconds (number) in JSON.
local function parse_credentials(body, result_key)
  local doc = json.parse(body)
  local resp = doc[result_key .. "Response"]
  local result = resp and resp[result_key .. "Result"]
  local c = result and result.Credentials
  if not c then
    error("aws.sts: unexpected STS response shape (missing " .. result_key .. "Response)")
  end
  local expires_at = nil
  if type(c.Expiration) == "number" then
    expires_at = c.Expiration
  end
  return {
    access_key = c.AccessKeyId,
    secret_key = c.SecretAccessKey,
    session_token = c.SessionToken,
    expires_at = expires_at,
  }
end

local function cache_get(key)
  local e = _cache[key]
  if not e then return nil end
  if e.expires_at and os.time() > (e.expires_at - EXPIRY_SKEW_SECS) then
    _cache[key] = nil
    return nil
  end
  return e
end

--- Assume a role with a web identity token (e.g. an IRSA-mounted ServiceAccount
--- token). This STS call is unsigned — the token itself authenticates it.
---
--- @param role_arn string The role to assume
--- @param token string The web identity (OIDC) token
--- @param opts table|nil { region, sts_endpoint, session_name, duration_secs }
--- @return table { access_key, secret_key, session_token, expires_at }
function M.assume_role_with_web_identity(role_arn, token, opts)
  opts = opts or {}
  local body = form_encode({
    Action = "AssumeRoleWithWebIdentity",
    Version = "2011-06-15",
    RoleArn = role_arn,
    RoleSessionName = opts.session_name or "assay",
    WebIdentityToken = token,
    DurationSeconds = tostring(opts.duration_secs or DEFAULT_DURATION),
  })
  local resp = http.post(sts_url(opts), body, {
    headers = {
      ["Content-Type"] = "application/x-www-form-urlencoded",
      ["Accept"] = "application/json",
    },
  })
  if resp.status ~= 200 then
    error("aws.sts.assume_role_with_web_identity: HTTP " .. resp.status .. ": " .. resp.body)
  end
  return parse_credentials(resp.body, "AssumeRoleWithWebIdentity")
end

--- Assume an IAM role (sts:AssumeRole), signing with base credentials resolved
--- via the standard chain (or passed as opts.credentials).
---
--- @param role_arn string The role to assume
--- @param opts table|nil { credentials, region, sts_endpoint, session_name, duration_secs, profile }
--- @return table { access_key, secret_key, session_token, expires_at }
function M.assume_role(role_arn, opts)
  opts = opts or {}
  local base = opts.credentials or M.credentials({
    profile = opts.profile,
    region = opts.region,
    sts_endpoint = opts.sts_endpoint,
  })
  local body = form_encode({
    Action = "AssumeRole",
    Version = "2011-06-15",
    RoleArn = role_arn,
    RoleSessionName = opts.session_name or "assay",
    DurationSeconds = tostring(opts.duration_secs or DEFAULT_DURATION),
  })
  local url = sts_url(opts)
  local host = url:gsub("^https?://", ""):gsub("/.*$", "")
  local headers = sigv4.sign({
    access_key = base.access_key,
    secret_key = base.secret_key,
    session_token = base.session_token,
    service = "sts",
    region = region_of(opts),
    method = "POST",
    host = host,
    path = "/",
    payload = body,
    headers = { ["content-type"] = "application/x-www-form-urlencoded" },
  })
  headers["accept"] = "application/json"
  local resp = http.post(url, body, { headers = headers })
  if resp.status ~= 200 then
    error("aws.sts.assume_role: HTTP " .. resp.status .. " assuming " .. role_arn .. ": " .. resp.body)
  end
  return parse_credentials(resp.body, "AssumeRole")
end

-- ===== ~/.aws config-file profiles =====

-- Minimal INI parser for ~/.aws/config + ~/.aws/credentials: [section] headers,
-- k = v lines, ;/# comments. Returns { section_name -> { key -> value } }.
local function parse_ini(text)
  local sections = {}
  local current = nil
  for raw in (text .. "\n"):gmatch("(.-)\n") do
    local line = raw:match("^%s*(.-)%s*$")
    if line ~= "" and not line:match("^[;#]") then
      local section = line:match("^%[(.-)%]$")
      if section then
        current = {}
        sections[section] = current
      elseif current then
        local k, v = line:match("^([%w_%-%.]+)%s*=%s*(.-)$")
        if k then current[k:lower()] = v end
      end
    end
  end
  return sections
end
M._parse_ini = parse_ini -- exposed for tests

local function read_ini(path)
  local ok, text = pcall(fs.read, path)
  if not ok then return {} end
  return parse_ini(text)
end

local function aws_dir()
  return (env.get("HOME") or "") .. "/.aws"
end

-- Look up a profile across both files. ~/.aws/credentials uses bare [name]
-- sections; ~/.aws/config uses [profile name] (except [default]).
local function profile_entry(name, opts)
  local credentials_file = (opts and opts.credentials_file) or aws_dir() .. "/credentials"
  local config_file = (opts and opts.config_file) or aws_dir() .. "/config"
  local creds = read_ini(credentials_file)[name]
  local cfg = read_ini(config_file)[name == "default" and "default" or ("profile " .. name)]
  if not creds and not cfg then
    return nil
  end
  -- credentials-file keys win over config-file keys, per AWS CLI behavior.
  local merged = {}
  for k, v in pairs(cfg or {}) do merged[k] = v end
  for k, v in pairs(creds or {}) do merged[k] = v end
  return merged
end

local function resolve_profile(name, opts, depth)
  if depth > MAX_PROFILE_DEPTH then
    error("aws.sts: profile chain too deep resolving '" .. name .. "' (circular source_profile?)")
  end
  local p = profile_entry(name, opts)
  if not p then
    error("aws.sts: profile '" .. name .. "' not found in ~/.aws/config or ~/.aws/credentials")
  end
  if p.aws_access_key_id and p.aws_secret_access_key and not p.role_arn then
    return {
      access_key = p.aws_access_key_id,
      secret_key = p.aws_secret_access_key,
      session_token = p.aws_session_token,
    }
  end
  if p.role_arn then
    local call_opts = {
      region = (opts and opts.region) or p.region,
      sts_endpoint = opts and opts.sts_endpoint,
      session_name = "assay-" .. name,
    }
    if p.web_identity_token_file then
      local token = fs.read(p.web_identity_token_file)
      return M.assume_role_with_web_identity(p.role_arn, token, call_opts)
    end
    local base
    if p.aws_access_key_id and p.aws_secret_access_key then
      base = {
        access_key = p.aws_access_key_id,
        secret_key = p.aws_secret_access_key,
        session_token = p.aws_session_token,
      }
    elseif p.source_profile then
      base = resolve_profile(p.source_profile, opts, depth + 1)
    else
      -- credential_source=Environment or nothing: fall through to the env/IRSA chain.
      base = M.credentials({
        region = call_opts.region,
        sts_endpoint = call_opts.sts_endpoint,
        no_profile = true,
      })
    end
    call_opts.credentials = base
    return M.assume_role(p.role_arn, call_opts)
  end
  error("aws.sts: profile '" .. name .. "' has neither static keys nor a role_arn")
end

--- Resolve AWS credentials the way the AWS CLI does, in-process:
---   1. opts.access_key / opts.secret_key — explicit, used verbatim
---   2. opts.profile (or AWS_PROFILE) — ~/.aws/credentials + ~/.aws/config,
---      following role_arn + source_profile chains and web_identity_token_file
---   3. AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY (+ AWS_SESSION_TOKEN) env vars
---   4. IRSA: AWS_ROLE_ARN + AWS_WEB_IDENTITY_TOKEN_FILE (in-cluster pods)
--- If opts.role_arn is set, the resolved credentials then assume that role.
--- Temporary credentials are cached until shortly before expiry.
---
--- @param opts table|nil { access_key, secret_key, session_token, profile, role_arn, region, sts_endpoint, config_file, credentials_file }
--- @return table { access_key, secret_key, session_token? }
function M.credentials(opts)
  opts = opts or {}
  local base

  if opts.access_key and opts.secret_key then
    base = {
      access_key = opts.access_key,
      secret_key = opts.secret_key,
      session_token = opts.session_token,
    }
  else
    local profile = not opts.no_profile and (opts.profile or env.get("AWS_PROFILE")) or nil
    if profile then
      local key = "profile:" .. profile
      base = cache_get(key)
      if not base then
        base = resolve_profile(profile, opts, 1)
        _cache[key] = base
      end
    elseif env.get("AWS_ACCESS_KEY_ID") and env.get("AWS_SECRET_ACCESS_KEY") then
      base = {
        access_key = env.get("AWS_ACCESS_KEY_ID"),
        secret_key = env.get("AWS_SECRET_ACCESS_KEY"),
        session_token = env.get("AWS_SESSION_TOKEN"),
      }
    elseif env.get("AWS_ROLE_ARN") and env.get("AWS_WEB_IDENTITY_TOKEN_FILE") then
      local role = env.get("AWS_ROLE_ARN")
      local key = "irsa:" .. role
      base = cache_get(key)
      if not base then
        local token = fs.read(env.get("AWS_WEB_IDENTITY_TOKEN_FILE"))
        base = M.assume_role_with_web_identity(role, token, opts)
        _cache[key] = base
      end
    else
      error(
        "aws.sts: no credentials found — pass access_key/secret_key or profile, "
          .. "or set AWS_PROFILE / AWS_ACCESS_KEY_ID / AWS_ROLE_ARN+AWS_WEB_IDENTITY_TOKEN_FILE"
      )
    end
  end

  if opts.role_arn then
    local key = "role:" .. opts.role_arn .. "|" .. (base.access_key or "")
    local assumed = cache_get(key)
    if not assumed then
      assumed = M.assume_role(opts.role_arn, {
        credentials = base,
        region = opts.region,
        sts_endpoint = opts.sts_endpoint,
      })
      _cache[key] = assumed
    end
    return assumed
  end
  return base
end

return M
