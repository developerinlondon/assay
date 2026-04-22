--- @module assay.oauth2
--- @description Google OAuth2 helper for loading credentials, refreshing access tokens, persisting token files, and building auth headers.
--- @keywords oauth2, google, auth, token, refresh, credentials, bearer, gmail, gcal
--- @quickref oauth2.from_file(credentials_path?, token_path?, opts?) -> client | Load OAuth2 credentials and token files
--- @quickref client:access_token() -> string | Return current access token
--- @quickref client:refresh() -> string | Refresh access token using refresh_token grant
--- @quickref client:save() -> true | Persist token data back to token file
--- @quickref client:headers() -> {Authorization, Content-Type} | Return JSON auth headers

local M = {}

local DEFAULT_CREDENTIALS = "~/.config/gog/credentials.json"
local DEFAULT_TOKEN = "~/.config/gog/token.json"
local DEFAULT_TOKEN_URL = "https://oauth2.googleapis.com/token"

local function expand_path(path)
  local home = env.get("HOME") or ""
  return path:gsub("^~", home)
end

local function load_json_file(path)
  local content = fs.read(path)
  if not content or content == "" then
    error("oauth2: failed to read file: " .. path)
  end
  return json.parse(content)
end

function M.from_file(credentials_path, token_path, opts)
  opts = opts or {}

  local resolved_credentials = expand_path(credentials_path or DEFAULT_CREDENTIALS)
  local resolved_token = expand_path(token_path or DEFAULT_TOKEN)
  local creds_data = load_json_file(resolved_credentials)
  local token_data = load_json_file(resolved_token)
  local creds = creds_data.installed or creds_data.web or creds_data

  local client = {
    _credentials = creds,
    _token_data = token_data,
    _token_file = resolved_token,
    _token_url = opts.token_url or DEFAULT_TOKEN_URL,
    _access_token = token_data.access_token,
  }

  function client:access_token()
    return self._access_token
  end

  function client:refresh()
    local resp = http.post(self._token_url, {
      grant_type = "refresh_token",
      refresh_token = self._token_data.refresh_token,
      client_id = self._credentials.client_id,
      client_secret = self._credentials.client_secret,
    })
    if resp.status ~= 200 then
      error("oauth2: token refresh failed HTTP " .. resp.status .. ": " .. resp.body)
    end

    local result = json.parse(resp.body)
    self._access_token = result.access_token
    self._token_data.access_token = result.access_token
    if result.refresh_token then
      self._token_data.refresh_token = result.refresh_token
    end
    if result.expires_in then
      self._token_data.expires_in = result.expires_in
    end
    if result.token_type then
      self._token_data.token_type = result.token_type
    end
    return self._access_token
  end

  function client:save()
    fs.write(self._token_file, json.encode(self._token_data))
    return true
  end

  function client:headers()
    return {
      ["Authorization"] = "Bearer " .. self:access_token(),
      ["Content-Type"] = "application/json",
    }
  end

  return client
end

return M
