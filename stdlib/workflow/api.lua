--- HTTP wrapper used by every public method in `assay.workflow` to
--- talk to the engine. Lives in its own file so callers don't have to
--- read 1000+ lines of unrelated code to understand the auth/header
--- conventions.
---
--- The parent module (`stdlib/workflow.lua`) owns the connection
--- state (`M._engine_url`, `M._auth_token`); we accept it as the
--- first arg so this submodule stays state-free and trivially
--- testable.

local M = {}

--- Make an authenticated request to the engine.
--- @param parent table  The parent workflow module (provides `_engine_url`, `_auth_token`).
--- @param method string  HTTP method ("GET", "POST", "PATCH", "DELETE").
--- @param path string    Path relative to `/api/v1`, leading slash required.
--- @param body table?    Optional JSON-encodable body for POST/PATCH.
--- @return table         The http response (`{status, body, ...}`).
function M.call(parent, method, path, body)
    local url = parent._engine_url .. "/api/v1" .. path
    local opts = { headers = {} }

    if parent._auth_token then
        opts.headers["Authorization"] = "Bearer " .. parent._auth_token
    end

    if method == "GET" then
        return http.get(url, opts)
    elseif method == "POST" then
        return http.post(url, body or {}, opts)
    elseif method == "PATCH" then
        return http.patch(url, body or {}, opts)
    elseif method == "DELETE" then
        return http.delete(url, opts)
    else
        error("workflow._api: unsupported method: " .. method)
    end
end

return M
