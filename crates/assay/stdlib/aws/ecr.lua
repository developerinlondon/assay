--- @module assay.aws.ecr
--- @description AWS Elastic Container Registry. Get authorization tokens for pushing/pulling images.
--- @keywords aws, ecr, container, registry, docker, authorization, token, login
--- @quickref client(opts) -> client | Create an ECR client (opts = {access_key?, secret_key?, profile?, role_arn?, region?, session_token?, endpoint?}; no keys -> standard chain via assay.aws.sts)
--- @quickref c:get_authorization_token() -> {token, proxy_endpoint, expires_at} | Get ECR auth token

local M = {}

local sigv4 = require("assay.aws.sigv4")

--- Create an ECR client.
---
--- @param opts table with fields:
---   access_key    (string|nil) AWS access key ID (omit to resolve via the
---                              standard chain — profile / env / IRSA)
---   secret_key    (string|nil) AWS secret access key
---   profile       (string|nil) AWS profile to resolve credentials from
---   role_arn      (string|nil) Role to assume on top of the resolved credentials
---   region        (string|nil) AWS region (default AWS_REGION/AWS_DEFAULT_REGION)
---   session_token (string|nil) AWS session token (for STS credentials)
---   endpoint      (string|nil) Override the API endpoint (for VPC endpoints
---                              or tests). Defaults to api.ecr.<region>.amazonaws.com.
function M.client(opts)
  opts = opts or {}
  -- No explicit keys → resolve via the standard chain (profile / env / IRSA),
  -- honoring opts.profile and opts.role_arn. See assay.aws.sts.
  local creds = opts
  if not opts.access_key then
    creds = require("assay.aws.sts").credentials(opts)
  end
  local access_key = creds.access_key or error("ecr.client: access_key is required")
  local secret_key = creds.secret_key or error("ecr.client: secret_key is required")
  local region = opts.region or env.get("AWS_REGION") or env.get("AWS_DEFAULT_REGION")
    or error("ecr.client: region is required")
  local session_token = creds.session_token
  local endpoint = opts.endpoint

  -- Endpoint can be a full URL (http://… for tests, https://… for VPC endpoints)
  -- or a bare host. Normalise to {url, host}.
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
    host = "api.ecr." .. region .. ".amazonaws.com"
    url = "https://" .. host
  end

  local c = {}

  function c:get_authorization_token()
    local signed_headers = sigv4.sign({
      access_key = access_key,
      secret_key = secret_key,
      session_token = session_token,
      service = "ecr",
      region = region,
      method = "POST",
      host = host,
      path = "/",
      payload = "{}",
      headers = {
        ["content-type"] = "application/x-amz-json-1.1",
        ["x-amz-target"] = "AmazonEC2ContainerRegistry_V20150921.GetAuthorizationToken",
      },
    })

    local response = http.post(url .. "/", "{}", { headers = signed_headers })

    if not response then
      error("ECR GetAuthorizationToken: no response")
    end
    if response.status ~= 200 then
      error("ECR GetAuthorizationToken: status=" .. tostring(response.status)
        .. " body=" .. tostring(response.body))
    end

    local ok, data = pcall(json.parse, response.body)
    if not ok or type(data) ~= "table" then
      error("ECR GetAuthorizationToken: malformed JSON response: " .. tostring(response.body))
    end
    local auth_list = data.authorizationData
    if not auth_list or #auth_list == 0 then
      error("ECR GetAuthorizationToken: response missing authorizationData: " .. response.body)
    end

    local auth = auth_list[1]
    local decoded = base64.decode(auth.authorizationToken)
    local colon = decoded:find(":")
    if not colon then
      error("ECR GetAuthorizationToken: decoded token missing 'AWS:' prefix")
    end
    local token = decoded:sub(colon + 1)

    return {
      token = token,
      proxy_endpoint = auth.proxyEndpoint,
      expires_at = auth.expiresAt,
    }
  end

  return c
end

return M
