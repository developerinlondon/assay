local M = {}

function M.client(url, token)
  local c = {
    url = url:gsub("/+$", ""),
    token = token,
  }

  function c:read(path)
    local resp = http.get(self.url .. "/v1/" .. path, {
      headers = { ["X-Vault-Token"] = self.token },
    })
    if resp.status == 404 then
      return nil
    end
    if resp.status ~= 200 then
      error("openbao.read: HTTP " .. resp.status .. ": " .. resp.body)
    end
    local data = json.parse(resp.body)
    return data.data
  end

  function c:write(path, payload)
    local resp = http.post(self.url .. "/v1/" .. path, payload, {
      headers = { ["X-Vault-Token"] = self.token },
    })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("openbao.write: HTTP " .. resp.status .. ": " .. resp.body)
    end
    if resp.status == 204 then
      return nil
    end
    local data = json.parse(resp.body)
    return data.data
  end

  function c:delete(path)
    local resp = http.delete(self.url .. "/v1/" .. path, {
      headers = { ["X-Vault-Token"] = self.token },
    })
    if resp.status ~= 200 and resp.status ~= 204 then
      error("openbao.delete: HTTP " .. resp.status .. ": " .. resp.body)
    end
  end

  function c:kv_get(mount, key)
    return self:read(mount .. "/data/" .. key)
  end

  function c:kv_put(mount, key, data)
    return self:write(mount .. "/data/" .. key, { data = data })
  end

  function c:kv_delete(mount, key)
    return self:delete(mount .. "/data/" .. key)
  end

  return c
end

return M
