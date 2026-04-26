--- @module assay.ansi
--- @description ANSI SGR → HTML conversion + stripper for log viewers. No state, no deps.
--- @keywords ansi, sgr, escape, html, log, color, terminal
--- @quickref ansi.to_html(line) -> string | Convert SGR to <span> tags, HTML-escape unsafe chars
--- @quickref ansi.strip(line) -> string | Drop all CSI sequences, return plain text

local M = {}

local ESC = string.char(27)

local HTML_ESCAPES = {
  ["&"] = "&amp;",
  ["<"] = "&lt;",
  [">"] = "&gt;",
  ['"'] = "&quot;",
  ["'"] = "&#39;",
}

local function html_escape(s)
  return (s:gsub("[&<>\"']", HTML_ESCAPES))
end

local function is_fg(code)
  return (code >= 30 and code <= 37) or (code >= 90 and code <= 97)
end

local function is_bg(code)
  return (code >= 40 and code <= 47) or (code >= 100 and code <= 107)
end

local function parse_params(params)
  local codes = {}
  if params == "" then
    codes[1] = 0
    return codes
  end
  for part in params:gmatch("([^;]*)") do
    local n = tonumber(part)
    if n then
      codes[#codes + 1] = n
    end
  end
  return codes
end

function M.strip(line)
  if line == nil or line == "" then
    return ""
  end
  local out = {}
  local i = 1
  local len = #line
  while i <= len do
    local c = line:sub(i, i)
    if c == ESC and line:sub(i + 1, i + 1) == "[" then
      local j = i + 2
      while j <= len do
        local b = line:byte(j)
        if b and b >= 0x40 and b <= 0x7E then
          break
        end
        j = j + 1
      end
      i = j + 1
    else
      out[#out + 1] = c
      i = i + 1
    end
  end
  return table.concat(out)
end

function M.to_html(line)
  if line == nil or line == "" then
    return ""
  end

  local out = {}
  local fg_open = false
  local bg_open = false
  local bold_open = false

  local function close_fg()
    if fg_open then
      out[#out + 1] = "</span>"
      fg_open = false
    end
  end
  local function close_bg()
    if bg_open then
      out[#out + 1] = "</span>"
      bg_open = false
    end
  end
  local function close_bold()
    if bold_open then
      out[#out + 1] = "</span>"
      bold_open = false
    end
  end
  local function close_all()
    close_fg()
    close_bg()
    close_bold()
  end

  local i = 1
  local len = #line
  local text_buf = {}

  local function flush_text()
    if #text_buf > 0 then
      out[#out + 1] = html_escape(table.concat(text_buf))
      text_buf = {}
    end
  end

  while i <= len do
    local c = line:sub(i, i)
    if c == ESC and line:sub(i + 1, i + 1) == "[" then
      flush_text()
      local j = i + 2
      while j <= len do
        local b = line:byte(j)
        if b and b >= 0x40 and b <= 0x7E then
          break
        end
        j = j + 1
      end
      local final = line:sub(j, j)
      local params = line:sub(i + 2, j - 1)
      if final == "m" then
        local codes = parse_params(params)
        for _, code in ipairs(codes) do
          if code == 0 then
            close_all()
          elseif code == 1 then
            if not bold_open then
              out[#out + 1] = '<span class="ansi-bold">'
              bold_open = true
            end
          elseif code == 39 then
            close_fg()
          elseif code == 49 then
            close_bg()
          elseif is_fg(code) then
            close_fg()
            out[#out + 1] = '<span class="ansi-fg-' .. code .. '">'
            fg_open = true
          elseif is_bg(code) then
            close_bg()
            out[#out + 1] = '<span class="ansi-bg-' .. code .. '">'
            bg_open = true
          end
        end
      end
      i = j + 1
    else
      text_buf[#text_buf + 1] = c
      i = i + 1
    end
  end

  flush_text()
  close_all()
  return table.concat(out)
end

return M
