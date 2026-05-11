--- @module assay.aws.ecr
--- @description AWS Elastic Container Registry. Get authorization tokens for pushing/pulling images.
--- @keywords aws, ecr, container, registry, docker, authorization, token, login
--- @quickref client(access_key, secret_key, session_token, region) -> client | Create an ECR client
--- @quickref c:get_authorization_token() -> {token, proxy_endpoint, expires_at} | Get ECR auth token

local M = {}

local sigv4 = require("assay.aws.sigv4")

function M.client(access_key, secret_key, session_token, region)
  local host = "api.ecr." .. region .. ".amazonaws.com"

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

    local response = http.post(
      "https://" .. host .. "/",
      "{}",
      { headers = signed_headers }
    )

    if not response or response.status ~= 200 then
      local s = response and response.status or "no response"
      error("ECR GetAuthorizationToken failed: status=" .. tostring(s))
    end

    local data = json.parse(response.body)
    local auth_list = data.authorizationData
    if not auth_list or #auth_list == 0 then
      error("ECR response missing authorizationData")
    end

    local auth = auth_list[1]
    local decoded = base64.decode(auth.authorizationToken)
    local colon = decoded:find(":")
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
