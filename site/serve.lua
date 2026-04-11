-- Serve the assay.rs site locally using assay's http.serve() builtin.
-- Usage: assay site/serve.lua [port]
-- Then open http://localhost:3000

local port = tonumber(arg and arg[1]) or 3000
local site_dir = "build/site"

local content_types = {
  html = "text/html; charset=utf-8",
  css  = "text/css; charset=utf-8",
  js   = "application/javascript; charset=utf-8",
  json = "application/json",
  txt  = "text/plain; charset=utf-8",
  png  = "image/png",
  svg  = "image/svg+xml",
  ico  = "image/x-icon",
}

local function serve_file(path)
  local file_path = site_dir .. path
  local ok, content = pcall(fs.read, file_path)
  if not ok then
    ok, content = pcall(fs.read, file_path .. ".html")
  end
  if not ok then return { status = 404, body = "Not found: " .. path } end
  local ext = path:match("%.(%w+)$") or "html"
  return {
    status = 200,
    body = content,
    headers = {
      ["Content-Type"] = content_types[ext] or "application/octet-stream",
      ["Cache-Control"] = "no-cache",
    },
  }
end

log.info("Serving " .. site_dir .. "/ at http://localhost:" .. port)

http.serve(port, {
  GET = {
    ["/"] = function() return serve_file("/index.html") end,
    ["/*"] = function(req) return serve_file(req.path) end,
  },
})
