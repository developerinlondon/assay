## AI Agent & Workflow

### assay.openclaw

OpenClaw AI agent platform integration. Invoke tools, send messages, manage state, spawn sub-agents, approval gates, LLM tasks.

```lua
local openclaw = require("assay.openclaw")
local c = openclaw.client()  -- auto-discovers $OPENCLAW_URL + $OPENCLAW_TOKEN

-- Invoke any OpenClaw tool
local result = c:invoke("message", "send", {target = "#general", message = "Hello!"})

-- Shorthand: send message
c:send("discord", "#alerts", "Service is down!")
c:notify("ops-team", "Deployment complete")

-- Persistent state (JSON files in ~/.openclaw/state/)
c:state_set("last-deploy", {version = "1.2.3", time = time()})
local prev = c:state_get("last-deploy")

-- Diff detection
local diff = c:diff("pr-state", new_snapshot)
if diff.changed then log.info("State changed!") end

-- LLM task execution
local answer = c:llm_task("Summarize this PR", {model = "claude-sonnet"})

-- Cron job management
c:cron_add({schedule = "0 9 * * *", task = "daily-report"})
local jobs = c:cron_list()

-- Sub-agent spawning
c:spawn("Fix the login bug", {model = "gpt-4o"})

-- Approval gates (interactive TTY or structured request)
if c:approve("Deploy to production?", context_data) then
  -- proceed with deployment
end
```

### assay.github

GitHub REST API client. PRs, issues, actions, repositories, GraphQL. No `gh` CLI dependency.

```lua
local github = require("assay.github")
local c = github.client()  -- uses $GITHUB_TOKEN

-- Pull requests
local pr = c.pulls:get("owner/repo", 123)
local prs = c.pulls:list("owner/repo", {state = "open", per_page = 10})
local reviews = c.pulls:reviews("owner/repo", 123)
c.pulls:merge("owner/repo", 123, {merge_method = "squash"})

-- Issues
local issues = c.issues:list("owner/repo", {labels = "bug", state = "open"})
local issue = c.issues:get("owner/repo", 42)
c.issues:create("owner/repo", "Bug title", "Description", {labels = {"bug"}})
c.issues:create_note("owner/repo", 42, "Fixed in PR #123")

-- Repository info
local repo = c.repos:get("owner/repo")

-- GitHub Actions
local runs = c.runs:list("owner/repo", {status = "completed"})
local run = c.runs:get("owner/repo", 12345)

-- GraphQL queries
local data = c:graphql("query { viewer { login } }")
local complex = c:graphql([[
  query($owner: String!, $name: String!) {
    repository(owner: $owner, name: $name) {
      pullRequests(last: 10, states: OPEN) {
        nodes { number title author { login } }
      }
    }
  }
]], {owner = "owner", name = "repo"})
```

### assay.gmail

Gmail REST API with OAuth2 token auto-refresh. Search, read, reply, send emails.

```lua
local gmail = require("assay.gmail")
local c = gmail.client({
  credentials_file = "/path/to/google-oauth2-credentials.json",
  token_file = "/path/to/saved-oauth2-token.json"
})

-- Search emails (Gmail search syntax)
local emails = c:search("newer_than:1d is:unread", {max = 20})
local urgent = c:search("subject:urgent OR subject:ASAP", {max = 5})

-- Read a specific message
local msg = c:get("message-id-here")

-- Reply to an email (preserves thread, references)
c:reply("message-id", {body = "Thanks for the update! The fix looks good."})

-- Send new email
c:send("user@example.com", "Project Update", [[
Hello team,

The deployment completed successfully at 3:00 PM UTC.

Best regards,
Eda
]])

-- List labels
local labels = c:labels()
for _, label in ipairs(labels) do
  log.info("Label: " .. label.name .. " (" .. label.messagesTotal .. " messages)")
end
```

### assay.gcal

Google Calendar REST API with OAuth2 token auto-refresh. Events CRUD, calendar list.

```lua
local gcal = require("assay.gcal")
local c = gcal.client({
  credentials_file = "/path/to/google-oauth2-credentials.json",
  token_file = "/path/to/saved-oauth2-token.json"
})

-- List upcoming events
local events = c:events({
  timeMin = "2026-04-05T00:00:00Z",
  timeMax = "2026-04-12T00:00:00Z",
  maxResults = 10
})

-- Get specific event
local event = c:event_get("event-id-from-google")

-- Create a new meeting
local new_event = c:event_create({
  summary = "Team standup",
  description = "Daily sync meeting",
  start = {dateTime = "2026-04-06T09:00:00Z", timeZone = "UTC"},
  ["end"] = {dateTime = "2026-04-06T09:30:00Z", timeZone = "UTC"},
  attendees = {
    {email = "alice@company.com"},
    {email = "bob@company.com"}
  }
})

-- Update existing event
c:event_update("event-id", {
  summary = "Team standup (updated agenda)",
  description = "Daily sync + sprint planning"
})

-- Delete event
c:event_delete("event-id")

-- List all calendars
local calendars = c:calendars()
for _, cal in ipairs(calendars) do
  log.info("Calendar: " .. cal.summary .. " (" .. cal.id .. ")")
end
```

### assay.oauth2

Google OAuth2 token management. File-based credentials loading, automatic access token refresh
via refresh_token grant, token persistence, and auth header generation. Used internally by
gmail and gcal modules.

Default credential paths: `~/.config/gog/credentials.json` (OAuth2 client credentials) and
`~/.config/gog/token.json` (saved access/refresh tokens).

- `M.from_file(credentials_path?, token_path?, opts?)` -> client -- Load OAuth2 credentials and token files. Defaults to `~/.config/gog/credentials.json` and `~/.config/gog/token.json`. Reads `installed` or `web` key from credentials JSON. `opts`: `{token_url}` to override the Google token endpoint.
- `client:access_token()` -> string -- Return current access token
- `client:refresh()` -> string -- Refresh access token using refresh_token grant. POSTs to `https://oauth2.googleapis.com/token` with client_id, client_secret, and refresh_token. Updates internal state with new access_token, refresh_token, expires_in, and token_type.
- `client:save()` -> true -- Persist current token data (including refreshed access_token) back to the token file
- `client:headers()` -> table -- Return `{Authorization = "Bearer <token>", ["Content-Type"] = "application/json"}` for use with http builtins

Example:
```lua
local oauth2 = require("assay.oauth2")

-- Load from default paths
local auth = oauth2.from_file()

-- Or specify custom paths
local auth = oauth2.from_file("/secrets/google-creds.json", "/data/google-token.json")

-- Refresh and persist
auth:refresh()
auth:save()

-- Use with http builtins
local resp = http.get("https://www.googleapis.com/calendar/v3/calendars/primary/events", {
  headers = auth:headers()
})
```

### assay.email_triage

Email triage helpers for deterministic categorization or OpenClaw LLM-assisted classification.
Sorts emails into three buckets: `needs_reply`, `needs_action`, and `fyi`.

Deterministic rules:
- `needs_action`: subject contains "action required", "urgent", or "deadline"
- `fyi`: from address contains "noreply", "no-reply", "newsletter"; subject contains "newsletter" or "automated"; or `email.automated == true`
- `needs_reply`: everything else (human emails that likely need a response)

- `M.categorize(emails, opts?)` -> buckets -- Deterministically bucket emails by subject and sender patterns. Returns `{needs_reply = [...], needs_action = [...], fyi = [...]}`. Each email should have `from`, `subject` fields (strings). `opts` is reserved for future use.
- `M.categorize_llm(emails, openclaw_client, opts?)` -> buckets -- Use OpenClaw LLM task for smarter bucketing. Requires an `openclaw_client` with `llm_task` method. Returns same bucket structure. `opts`: `{prompt, output_schema}` to customize the LLM classification prompt and expected JSON schema.

Example:
```lua
local email_triage = require("assay.email_triage")

-- Deterministic categorization (no LLM, no network)
local emails = {
  {from = "alice@company.com", subject = "Review PR #42"},
  {from = "noreply@github.com", subject = "CI build passed"},
  {from = "boss@company.com", subject = "Action required: quarterly report"},
}
local buckets = email_triage.categorize(emails)
-- buckets.needs_reply  = [{from="alice@...", subject="Review PR #42"}]
-- buckets.needs_action = [{from="boss@...", subject="Action required: ..."}]
-- buckets.fyi          = [{from="noreply@...", subject="CI build passed"}]

-- LLM-assisted triage via OpenClaw
local openclaw = require("assay.openclaw")
local oc = openclaw.client()
local smart_buckets = email_triage.categorize_llm(emails, oc)
```
