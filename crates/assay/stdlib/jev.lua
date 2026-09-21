--- @module assay.jev
--- @description TypeSafe Jev System One — ask a small semantic judgment as a typed question and get a typed answer back, never free text a caller has to parse. Three primitives: Choice picks one named option, Score reads a level off an ordered ladder, Noul returns the probability, 0 to 1, that a statement holds. Every answer carries the probabilities behind it, and Choice and Score carry a calibrated confidence, so the threshold a script branches on is written in the script rather than inferred from prose.
--- @category saas
--- @keywords jev, typesafe, system one, systemone, judgment, judgement, decision, classify, classification, choice, score, noul, yes/no, confidence, probability, calibrated, typed, llm, model
--- @env TYPESAFE_API_KEY
--- @quickref M.client(opts?) -> c | Key via opts.api_key or TYPESAFE_API_KEY; opts.base_url and opts.model override the defaults
--- @quickref c:system_one(state, questions, opts?) -> answers, info | POST /v1/systemone; answers is keyed as you keyed questions, info = {usage, model}
--- @quickref M.choice(instructions, criteria) -> question | criteria maps each option name to what it means; up to 255 of them
--- @quickref M.score(instructions, levels) -> question | levels is an ordered list low to high, 2 to 10 of them; the index is the score
--- @quickref M.noul(instructions, criteria?) -> question | A yes/no question; criteria optionally says what a yes and a no mean
--- @quickref choice answer -> {type, choice, probabilities, confidence} | choice is one of your option names; probabilities is keyed by them
--- @quickref score answer -> {type, score, probabilities, legend, confidence} | score indexes your levels; probabilities is keyed "0", "1", ...
--- @quickref noul answer -> {type, noul} | A probability 0 to 1; one number describes it, so there is no confidence beside it

local M = {}

local DEFAULT_BASE_URL = "https://api.typesafe.ai"
local DEFAULT_MODEL = "jev-latest"

-- The vendor's own SDK makes three attempts in all, backing off from half a
-- second and doubling to a five-second ceiling.
local DEFAULT_ATTEMPTS = 3
local DEFAULT_RETRY_DELAY_S = 0.5
local MAX_BACKOFF_S = 5
-- A `Retry-After` is the server's own number and beats the guess, but not past
-- a minute: a script parked longer than that on a rate limit has stopped being
-- a script, and the caller is better told to come back later.
local MAX_RETRY_AFTER_S = 60

-- What the API itself will refuse. Checked here so the error names the mistake
-- in the caller's own words rather than arriving as a 422 about a field.
local MAX_CHOICE_OPTIONS = 255
local MIN_SCORE_LEVELS = 2
local MAX_SCORE_LEVELS = 10

-- 429 is this caller going too fast; 529 is the service itself full. Both are
-- answers that may be different a second later, which is what makes them worth
-- repeating. Nothing else is.
local RETRYABLE = { [429] = "rate limited", [529] = "overloaded" }

local ENDPOINT = "/v1/systemone"
local WHERE = "POST " .. ENDPOINT

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

local function instructions_of(instructions, kind)
  local text = trim(instructions)
  if text == "" then
    error("jev: " .. kind .. " needs instructions — the question in words")
  end
  return text
end

--- A Choice: one named option out of a set, with the probability of each.
---
--- `criteria` maps an option name to what it means. The names are the caller's
--- own and come back verbatim in `answer.choice`, so a script can branch on
--- them directly. Describing each option is not decoration — it is the whole
--- of what the model is told the option stands for.
function M.choice(instructions, criteria)
  if type(criteria) ~= "table" then
    error("jev: choice criteria must be a table of option name to what it means")
  end
  local options = {}
  local count = 0
  for name, meaning in pairs(criteria) do
    if type(name) ~= "string" or trim(name) == "" then
      error("jev: choice option names must be non-empty strings — a criteria list is a map, not an array")
    end
    options[name] = tostring(meaning)
    count = count + 1
  end
  if count == 0 then error("jev: choice needs at least one option") end
  if count > MAX_CHOICE_OPTIONS then
    error("jev: choice takes at most " .. MAX_CHOICE_OPTIONS .. " options (got " .. count .. ")")
  end
  return {
    type = "choice",
    instructions = instructions_of(instructions, "choice"),
    criteria = options,
  }
end

--- A Score: a level off an ordered ladder, plus the probability of each rung.
---
--- `levels` is ordered low to high and its index *is* the score, so the ladder
--- a caller writes is the scale the answer is reported on. Two to ten rungs:
--- fewer is a Noul and more is a distinction the model cannot hold apart.
function M.score(instructions, levels)
  if type(levels) ~= "table" then
    error("jev: score levels must be an ordered list, low to high")
  end
  local n = #levels
  local count = 0
  for _ in pairs(levels) do count = count + 1 end
  if count ~= n then
    error("jev: score levels must be an ordered list, not a map — the order is the scale")
  end
  if n < MIN_SCORE_LEVELS or n > MAX_SCORE_LEVELS then
    error("jev: score needs between " .. MIN_SCORE_LEVELS .. " and " .. MAX_SCORE_LEVELS
      .. " levels, ordered low to high (got " .. n .. ")")
  end
  local ladder = {}
  for i = 1, n do ladder[i] = tostring(levels[i]) end
  return {
    type = "score",
    instructions = instructions_of(instructions, "score"),
    criteria = ladder,
  }
end

--- A Noul: the probability, 0 to 1, that the statement holds.
---
--- `criteria` is optional and says what each side stands for. Lua spells its
--- booleans as booleans, and `{ [true] = ... }` JSON-encodes to something the
--- API will not take, so both spellings are accepted here and one goes out.
---
--- The answer is that probability and carries no confidence beside it. One
--- number already describes a two-sided reading completely, and a second would
--- be the same information twice.
function M.noul(instructions, criteria)
  local question = {
    type = "noul",
    instructions = instructions_of(instructions, "noul"),
  }
  if criteria == nil then return question end
  if type(criteria) ~= "table" then
    error("jev: noul criteria must be a table keyed true and false")
  end
  local sides = {}
  local count = 0
  for key, meaning in pairs(criteria) do
    local side
    if key == true then side = "true"
    elseif key == false then side = "false"
    else side = tostring(key) end
    if side ~= "true" and side ~= "false" then
      error("jev: noul criteria keys must be true and false (got " .. side .. ")")
    end
    sides[side] = tostring(meaning)
    count = count + 1
  end
  if count == 0 then return question end
  question.criteria = sides
  return question
end

-- The 422 envelope this API uses is not published, so every shape an API of
-- this kind writes is tried in turn and the raw body stands when none of them
-- fits. A message invented here would be worse than the one the server sent.
local function field_message(body, raw)
  if type(body) == "table" then
    if type(body.error) == "table" and type(body.error.message) == "string" then
      return body.error.message
    end
    if type(body.detail) == "string" and trim(body.detail) ~= "" then
      return body.detail
    end
    if type(body.detail) == "table" then
      local parts = {}
      for _, item in ipairs(body.detail) do
        if type(item) == "string" then
          parts[#parts + 1] = item
        elseif type(item) == "table" then
          local where = ""
          if type(item.loc) == "table" then
            local segments = {}
            for _, one in ipairs(item.loc) do segments[#segments + 1] = tostring(one) end
            where = table.concat(segments, ".")
          end
          local said = trim(item.msg or item.message or "")
          if said ~= "" then
            parts[#parts + 1] = (where ~= "" and (where .. ": ") or "") .. said
          end
        end
      end
      if #parts > 0 then return table.concat(parts, "; ") end
    end
    if type(body.message) == "string" and trim(body.message) ~= "" then
      return body.message
    end
  end
  local verbatim = trim(raw)
  if verbatim ~= "" then return verbatim end
  return WHERE .. " was refused (HTTP 422) with no message"
end

-- reqwest lowercases every header name it hands back, so this is the only
-- spelling that can arrive; the other is read anyway because a stub is not
-- reqwest.
local function retry_after(resp)
  if type(resp.headers) ~= "table" then return nil end
  local said = tonumber(resp.headers["retry-after"] or resp.headers["Retry-After"])
  if not said or said ~= said or said < 0 then return nil end
  return math.min(said, MAX_RETRY_AFTER_S)
end

-- A Noul answer is a probability, not a side. Reading it as a boolean is the
-- mistake this guards: every number but zero is truthy in Lua, so a 0.04 that
-- slipped through as `if answer.noul then` would read as a confident yes.
local function check_nouls(answers)
  for _, answer in pairs(answers) do
    if type(answer) == "table" and answer.type == "noul" then
      local p = answer.noul
      if type(p) ~= "number" or p ~= p or p < 0 or p > 1 then
        error("jev: noul answer out of range: " .. tostring(p))
      end
    end
  end
end

--- A client for one TypeSafe instance.
---
--- `opts.api_key` or `TYPESAFE_API_KEY`. `opts.base_url` and `opts.model`
--- override the defaults; `opts.attempts` and `opts.retry_delay_s` the retry
--- budget. A `retry_delay_s` of zero retries without waiting.
function M.client(opts)
  opts = opts or {}
  local api_key = opts.api_key or env.get("TYPESAFE_API_KEY")
  if not api_key or trim(api_key) == "" then
    error("jev: api_key required (opts.api_key or TYPESAFE_API_KEY)")
  end
  local base_url = (opts.base_url or DEFAULT_BASE_URL):gsub("/+$", "")
  local model = opts.model or DEFAULT_MODEL
  local attempts = opts.attempts or DEFAULT_ATTEMPTS
  local retry_delay_s = opts.retry_delay_s or DEFAULT_RETRY_DELAY_S

  local function headers()
    return {
      Authorization = "Bearer " .. api_key,
      Accept = "application/json",
      ["Content-Type"] = "application/json",
    }
  end

  -- No silent fallback anywhere below: a judgment a caller is about to branch
  -- on is not something to guess at when the service did not answer.
  local function fail(resp)
    if resp.status == 401 then error("jev: unauthorized") end
    if resp.status == 422 then
      local read, parsed = pcall(json.parse, resp.body or "")
      error("jev: " .. field_message(read and parsed or nil, resp.body))
    end
    error("jev: " .. WHERE .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
  end

  local function read_answers(resp)
    local read, parsed = pcall(json.parse, resp.body or "")
    if not read or type(parsed) ~= "table" then
      error("jev: " .. WHERE .. " returned unparseable JSON")
    end
    if type(parsed.answers) ~= "table" then
      error("jev: " .. WHERE .. " returned no answers")
    end
    check_nouls(parsed.answers)
    return parsed.answers, {
      usage = type(parsed.usage) == "table" and parsed.usage or {},
      model = parsed.model,
    }
  end

  local c = {}

  --- Ask one state a set of questions, and get one answer per question.
  ---
  --- `state` is whatever the questions are about — a string, a record, a list
  --- of them — and goes over the wire as written. `questions` is keyed by the
  --- caller, and `answers` comes back under the same keys, so a script reads
  --- `answers.bucket.choice` rather than counting positions.
  ---
  --- Returns the answers and, beside them, `{usage, model}`: what the call
  --- cost in tokens and which model actually answered.
  function c:system_one(state, questions, o)
    o = o or {}
    if state == nil then
      error("jev: system_one needs state — what the questions are about")
    end
    if type(questions) ~= "table" or next(questions) == nil then
      error("jev: system_one needs at least one question")
    end
    local body = { state = state, model = o.model or model, questions = questions }
    local target = base_url .. ENDPOINT
    local tries = o.attempts or attempts
    if type(tries) ~= "number" or tries < 1 then
      error("jev: attempts must be at least 1")
    end
    local base_delay = o.retry_delay_s or retry_delay_s

    for attempt = 1, tries do
      local resp = http.post(target, body, { headers = headers() })
      if resp.status == 200 then return read_answers(resp) end
      local kind = RETRYABLE[resp.status]
      if not kind then fail(resp) end
      if attempt >= tries then
        error("jev: " .. kind .. " (HTTP " .. resp.status .. ") after " .. tries .. " attempts")
      end
      if base_delay > 0 then
        local wait = retry_after(resp)
          or math.min(base_delay * (2 ^ (attempt - 1)), MAX_BACKOFF_S)
        if wait > 0 then sleep(wait) end
      end
    end
  end

  return c
end

return M
