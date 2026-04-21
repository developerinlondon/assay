--- @module assay.email_triage
--- @description Email triage helpers for deterministic categorization or OpenClaw-assisted classification into action, reply, and FYI buckets.
--- @keywords email, triage, gmail, inbox, classify, categorize, openclaw, llm, workflow
--- @quickref email_triage.categorize(emails, opts?) -> buckets | Deterministically bucket emails by subject and sender
--- @quickref email_triage.categorize_llm(emails, openclaw_client, opts?) -> buckets | Use OpenClaw LLM task for smarter bucketing

local M = {}

local function empty_buckets()
  return {
    needs_reply = {},
    needs_action = {},
    fyi = {},
  }
end

local function is_automated(email)
  local from = (email.from or ""):lower()
  local subject = (email.subject or ""):lower()
  return email.automated == true
    or from:match("noreply") ~= nil
    or from:match("no%-reply") ~= nil
    or from:match("newsletter") ~= nil
    or subject:match("newsletter") ~= nil
    or subject:match("automated") ~= nil
  end

local function needs_action(email)
  local subject = (email.subject or ""):lower()
  return subject:match("action required") ~= nil
    or subject:match("urgent") ~= nil
    or subject:match("deadline") ~= nil
end

local function normalize(result)
  local buckets = result.categories or result.result or result
  if type(buckets) ~= "table" then
    error("email_triage: invalid LLM response")
  end
  buckets.needs_reply = buckets.needs_reply or {}
  buckets.needs_action = buckets.needs_action or {}
  buckets.fyi = buckets.fyi or {}
  return buckets
end

function M.categorize(emails, opts)
  opts = opts or {}
  local buckets = empty_buckets()
  for _, email in ipairs(emails or {}) do
    if needs_action(email) then
      buckets.needs_action[#buckets.needs_action + 1] = email
    elseif not is_automated(email) then
      buckets.needs_reply[#buckets.needs_reply + 1] = email
    else
      buckets.fyi[#buckets.fyi + 1] = email
    end
  end
  return buckets
end

function M.categorize_llm(emails, openclaw_client, opts)
  opts = opts or {}
  if not openclaw_client or not openclaw_client.llm_task then
    error("email_triage: openclaw_client with llm_task is required")
  end

  local prompt = opts.prompt or [[
Classify the provided email artifacts into exactly three buckets: needs_reply, needs_action, and fyi.

- needs_reply: human emails that likely need a response
- needs_action: emails that require urgent or explicit action
- fyi: newsletters, noreply mail, automated mail, or informational updates

Return only the bucketed emails.
]]

  local result = openclaw_client:llm_task(prompt, {
    artifacts = emails or {},
    output_schema = opts.output_schema or {
      type = "object",
      properties = {
        needs_reply = { type = "array" },
        needs_action = { type = "array" },
        fyi = { type = "array" },
      },
      required = { "needs_reply", "needs_action", "fyi" },
    },
  })

  return normalize(result)
end

return M
