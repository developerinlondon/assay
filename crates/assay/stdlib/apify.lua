--- @module assay.apify
--- @description Apify actor runs behind a mandatory spend cap — start an actor, wait for a terminal state, read the dataset back together with what the run cost — plus typed readers over the Instagram, LinkedIn and contact-details actors that answer in one stable shape per record.
--- @category registries
--- @icon apify
--- @keywords apify, actor, scrape, instagram, linkedin, profile, hashtag, comments, contact, email, phone, dataset, run, spend, cap
--- @quickref M.client(opts?) -> c | Token via opts.token or APIFY_TOKEN; base_url overridable
--- @quickref c:run(actor, input, opts) -> result | nil, reason, result | Start, wait, read the items; opts.max_total_charge_usd is mandatory
--- @quickref c:start(actor, input, opts) -> run | Start without waiting; the same cap rule applies
--- @quickref c:run_status(run_id, opts?) -> run | One read of the run; opts.wait_s holds the request open server-side
--- @quickref c:await(run_id, opts?) -> run | Poll until terminal or the attempt budget runs out
--- @quickref c:dataset_items(dataset_id, opts?) -> [item] | Every clean item, paged
--- @quickref c:abort(run_id) -> run | Abort a run that is still going
--- @quickref c:instagram_profiles(usernames, opts) -> {profiles, run} | apify/instagram-profile-scraper
--- @quickref c:instagram_comments(post_urls, limit, opts) -> {comments, run} | apify/instagram-scraper, billed per comment
--- @quickref c:instagram_hashtag_posts(tags, limit, opts) -> {posts, run} | apify/instagram-hashtag-scraper
--- @quickref c:linkedin_profiles(urls, opts) -> {people, run} | harvestapi/linkedin-profile-scraper, lead_provider person shape
--- @quickref c:contact_details(urls, opts) -> {sites, run} | vdrmota/contact-info-scraper, emails/phones/socials per start URL

local M = {}

local lp = require("assay.lead_provider")
local url = require("assay.url")

M.ACTORS = {
  instagram_profiles = "apify/instagram-profile-scraper",
  instagram_scraper = "apify/instagram-scraper",
  instagram_hashtags = "apify/instagram-hashtag-scraper",
  linkedin_profiles = "harvestapi/linkedin-profile-scraper",
  contact_details = "vdrmota/contact-info-scraper",
}

-- READY and RUNNING are the two states a run passes through; TIMING-OUT and
-- ABORTING are transitions, not answers, so a poll keeps going through them.
local TERMINAL = { SUCCEEDED = true, FAILED = true, ["TIMED-OUT"] = true, ABORTED = true }

local DEFAULT_TIMEOUT_S = 300
-- The HTTP client gives up at thirty seconds, so the server may hold a poll
-- open for less than that or the wait reads as a network failure.
local DEFAULT_WAIT_S = 20
local DEFAULT_POLL_S = 1
local DEFAULT_PAGE_SIZE = 1000
-- The bill lands after the run does: the event counts a few seconds after
-- SUCCEEDED, the dollar figure a few seconds after those. The cost is re-read
-- until two consecutive reads agree and the figure is in.
local DEFAULT_SETTLE_S = 2
local DEFAULT_SETTLE_READS = 8
-- No figure is believed before this many reads: the events of one run land
-- one at a time over several seconds, and two early reads can agree on a
-- partial bill as easily as on a zero.
local BELIEVED_AFTER_READS = 3

-- The contact actor refuses a lower cap with an HTTP 400, whatever the run
-- will actually cost, so a caller who sizes the cap to the expected spend is
-- refused at the network rather than told why.
local CONTACT_MIN_CAP_USD = 0.5

local LINKEDIN_MODE = {
  [false] = "Profile details no email ($4 per 1k)",
  [true] = "Profile details + email search ($10 per 1k)",
}

local function trim(s) return (tostring(s or ""):gsub("^%s+", ""):gsub("%s+$", "")) end

-- Counts arrive as numbers most of the time and as strings some of the time;
-- a string that is not digits is not a count.
local function to_count(v)
  if type(v) == "number" then return math.floor(v) end
  if type(v) == "string" and v:match("^%s*%d+%s*$") then return tonumber(trim(v)) end
  return nil
end

-- Rounded up rather than to nearest: a profile read costs a quarter of a cent,
-- and a ledger that rounds each run to nearest would record most of them as
-- free.
local function cents_up(usd)
  if type(usd) ~= "number" or usd ~= usd or usd < 0 then return nil end
  return math.ceil(usd * 100 - 1e-9)
end

-- The API addresses an actor as `owner~name`; the store shows `owner/name`.
-- Bare run ids and actor ids carry neither and pass through.
local function actor_path(actor)
  actor = trim(actor)
  if actor == "" then error("apify: actor id required") end
  return (actor:gsub("/", "~"))
end

local function to_run(data, actor)
  data = data or {}
  local opts = data.options or {}
  return {
    id = data.id,
    actor = actor or data.actId,
    act_id = data.actId,
    status = data.status,
    terminated = TERMINAL[data.status] == true,
    succeeded = data.status == "SUCCEEDED",
    status_message = data.statusMessage,
    dataset_id = data.defaultDatasetId,
    kv_store_id = data.defaultKeyValueStoreId,
    started_at = data.startedAt,
    finished_at = data.finishedAt,
    usage_total_usd = data.usageTotalUsd,
    usage_cents = cents_up(data.usageTotalUsd),
    charged_event_counts = data.chargedEventCounts,
    max_total_charge_usd = opts.maxTotalChargeUsd,
    exit_code = data.exitCode,
  }
end

local function same_counts(a, b)
  if type(a) ~= "table" or type(b) ~= "table" then return a == b end
  for k, v in pairs(a) do if b[k] ~= v then return false end end
  for k in pairs(b) do if a[k] == nil then return false end end
  return true
end

local function nothing_charged(counts)
  if type(counts) ~= "table" then return true end
  for _, n in pairs(counts) do
    if type(n) == "number" and n > 0 then return false end
  end
  return true
end

-- Agreeing reads are a settled bill once enough of them have passed, and a
-- zero only while no event stands against the run.
local function settled(x, y, reads)
  if reads < BELIEVED_AFTER_READS then return false end
  if x.usage_total_usd ~= y.usage_total_usd then return false end
  if not same_counts(x.charged_event_counts, y.charged_event_counts) then return false end
  return (x.usage_total_usd or 0) > 0 or nothing_charged(x.charged_event_counts)
end

local function source(actor, run_id)
  return actor .. " run " .. tostring(run_id)
end

local function list_of(items, map)
  local out = {}
  for _, u in ipairs(items or {}) do
    local v = map(u)
    if v ~= nil then out[#out + 1] = v end
  end
  return out
end

-- A network with nothing found arrives as an empty value rather than an empty
-- list, and a network with something found repeats the same profile under
-- http and https and under a mobile host.
local function unique_list(items, normalise)
  local out, seen = {}, {}
  for _, v in ipairs(type(items) == "table" and items or {}) do
    local s = trim(v)
    if normalise then s = normalise(s) end
    if s ~= "" and not seen[s] then
      seen[s] = true
      out[#out + 1] = s
    end
  end
  return out
end

local function lowercased(s) return s:lower() end

local function post_from(p)
  return {
    id = p.id,
    shortcode = p.shortCode,
    url = p.url,
    type = p.type,
    caption = p.caption,
    hashtags = p.hashtags,
    mentions = p.mentions,
    owner_username = p.ownerUsername,
    owner_id = p.ownerId,
    owner_full_name = p.ownerFullName,
    likes = to_count(p.likesCount),
    comments = to_count(p.commentsCount),
    video_views = to_count(p.videoViewCount),
    video_plays = to_count(p.videoPlayCount),
    taken_at = p.timestamp,
    display_url = p.displayUrl,
    video_url = p.videoUrl,
    location = p.locationName,
    sponsored = p.isSponsored == true,
  }
end

local function profile_from(item, from)
  if item.error then
    return { username = item.username or item.inputUrl, error = item.error, provenance = lp.provenance("apify", from) }
  end
  return {
    id = item.id,
    username = item.username,
    full_name = item.fullName,
    biography = item.biography,
    followers = to_count(item.followersCount),
    follows = to_count(item.followsCount),
    posts_count = to_count(item.postsCount),
    external_url = item.externalUrl,
    external_urls = list_of(item.externalUrls, function(u) return type(u) == "table" and u.url or u end),
    profile_pic_url = item.profilePicUrlHD or item.profilePicUrl,
    verified = item.verified == true,
    private = item.private == true,
    business = item.isBusinessAccount == true,
    category = (item.businessCategoryName ~= "None" and item.businessCategoryName or nil),
    latest_posts = list_of(item.latestPosts, post_from),
    provenance = lp.provenance("apify", from),
  }
end

local function comment_from(item, from)
  local owner = type(item.owner) == "table" and item.owner or {}
  return {
    id = item.id,
    comment_url = item.commentUrl,
    post_url = item.postUrl or item.inputUrl,
    text = item.text,
    owner_username = item.ownerUsername or owner.username,
    owner_id = item.ownerId or owner.id,
    owner_verified = (item.ownerIsVerified or owner.is_verified) == true,
    likes = to_count(item.likesCount),
    replies = to_count(item.repliesCount),
    taken_at = item.timestamp,
    provenance = lp.provenance("apify", from),
  }
end

local function hashtag_post_from(item, from)
  local post = post_from(item)
  local from_url = type(item.inputUrl) == "string" and item.inputUrl:match("/explore/tags/([^/?#]+)") or nil
  post.tag = item.queryTag or from_url or item.inputUrl
  post.provenance = lp.provenance("apify", from)
  return post
end

-- The vendor's email search asserts deliverability; a vendor's assertion is
-- not a delivery, so the address stays below VERIFIED whatever it claims.
local function linkedin_person_from(item, from)
  local found = type(item.emails) == "table" and item.emails or { item.email }
  local emails = list_of(found, function(e)
    local address = trim(type(e) == "table" and (e.email or e.address) or e)
    if address == "" then return nil end
    return lp.email("apify", from, { address = address, email_type = "provider" })
  end)
  local current = (item.currentPosition or {})[1] or {}
  local latest = (item.experience or {})[1] or {}
  local loc = item.location
  if type(loc) == "table" then loc = loc.linkedinText end
  local first, last = trim(item.firstName), trim(item.lastName)
  local full = trim(first .. " " .. last)
  local person = lp.person("apify", from, {
    first_name = (first ~= "" and first or nil),
    last_name = (last ~= "" and last or nil),
    full_name = (full ~= "" and full or nil),
    title = latest.position or item.headline,
    company = current.companyName or latest.companyName,
    linkedin = item.linkedinUrl,
    location = loc,
    emails = emails,
  })
  person.public_identifier = item.publicIdentifier
  person.headline = item.headline
  person.about = item.about
  person.photo = item.photo
  person.followers = to_count(item.followerCount)
  return person
end

-- One merged row per start URL. The actor keeps the start URL rather than the
-- page a contact was found on, and records every page it visited beside it.
local function contact_site_from(item, from)
  return {
    url = item.originalStartUrl or item.url,
    domain = item.domain,
    emails = unique_list(item.emails, lowercased),
    phones = unique_list(item.phones),
    phones_uncertain = unique_list(item.phonesUncertain),
    linkedins = unique_list(item.linkedIns),
    instagrams = unique_list(item.instagrams),
    twitters = unique_list(item.twitters),
    facebooks = unique_list(item.facebooks),
    youtubes = unique_list(item.youtubes),
    tiktoks = unique_list(item.tiktoks),
    pages_visited = #unique_list(item.scrapedUrls),
    provenance = lp.provenance("apify", from),
  }
end

function M.client(opts)
  opts = opts or {}
  local token = opts.token or env.get("APIFY_TOKEN")
  if not token or trim(token) == "" then
    error("apify: token required (opts.token or APIFY_TOKEN)")
  end
  local base_url = (opts.base_url or "https://api.apify.com/v2"):gsub("/+$", "")

  local function headers()
    return {
      Authorization = "Bearer " .. token,
      Accept = "application/json",
      ["Content-Type"] = "application/json",
    }
  end

  local function fail(where, resp)
    if resp.status == 401 or resp.status == 403 then
      error("apify: " .. where .. " rejected the token (HTTP " .. resp.status .. ")")
    end
    if resp.status == 402 then
      error("apify: " .. where .. " refused for billing reasons (HTTP 402): " .. (resp.body or ""))
    end
    if resp.status == 404 then
      error("apify: " .. where .. " not found (HTTP 404)")
    end
    if resp.status == 429 then
      error("apify: " .. where .. " rate limited (HTTP 429)")
    end
    error("apify: " .. where .. " HTTP " .. resp.status .. ": " .. (resp.body or ""))
  end

  local function decode(resp, where)
    local ok, parsed = pcall(json.parse, resp.body or "")
    if not ok then error("apify: " .. where .. " returned unparseable JSON") end
    return parsed
  end

  -- Every run and dataset answer is wrapped in `data`; an answer without it
  -- is a shape this module was not written against.
  local function unwrap(resp, where)
    local body = decode(resp, where)
    if type(body) ~= "table" or body.data == nil then
      error("apify: " .. where .. " returned no data envelope")
    end
    return body.data
  end

  local c = {}

  --- Start a run and return at once. `opts.max_total_charge_usd` is required:
  --- an actor billed per event has no price until it has run, and the cap is
  --- the only thing standing between a typo in `resultsLimit` and the month's
  --- budget.
  ---
  --- A 201 whose body cannot be read is the one place this module cannot hand
  --- back a run id for a run that may already be spending: the id was in that
  --- body. The raise carries the body verbatim so the id can be recovered from
  --- it by hand, or the run found in the Apify console by its actor and time.
  function c:start(actor, input, o)
    o = o or {}
    local cap = o.max_total_charge_usd
    if type(cap) ~= "number" or cap ~= cap or cap <= 0 then
      error("apify: max_total_charge_usd is required on every run — a positive number of dollars")
    end
    if type(input) ~= "table" then error("apify: input must be a table") end
    local q = { maxTotalChargeUsd = cap, timeout = o.timeout_s or DEFAULT_TIMEOUT_S }
    if o.memory_mb then q.memory = o.memory_mb end
    if o.max_items then q.maxItems = o.max_items end
    if o.build then q.build = o.build end
    if o.wait_s and o.wait_s > 0 then q.waitForFinish = o.wait_s end
    local where = "POST /acts/" .. actor_path(actor) .. "/runs"
    local target = base_url .. "/acts/" .. actor_path(actor) .. "/runs?" .. url.encode_form(q)
    local resp = http.post(target, input, { headers = headers() })
    if resp.status ~= 201 and resp.status ~= 200 then fail(where, resp) end
    local read, data = pcall(unwrap, resp, where)
    if not read then
      error(tostring(data) .. " — the run may already be spending; body was: " .. (resp.body or ""))
    end
    local run = to_run(data, actor)
    if not run.id then
      error("apify: " .. where .. " returned no run id; body was: " .. (resp.body or ""))
    end
    return run
  end

  --- One read of a run. With `opts.wait_s` the server holds the request open
  --- until the run finishes or that many seconds pass, which turns a poll loop
  --- into a handful of long requests.
  function c:run_status(run_id, o)
    o = o or {}
    local where = "GET /actor-runs/" .. tostring(run_id)
    local target = base_url .. "/actor-runs/" .. tostring(run_id)
    if o.wait_s and o.wait_s > 0 then
      target = target .. "?" .. url.encode_form({ waitForFinish = o.wait_s })
    end
    local resp = http.get(target, { headers = headers() })
    if resp.status ~= 200 then fail(where, resp) end
    return to_run(unwrap(resp, where), o.actor)
  end

  --- Poll until the run terminates. The last run seen comes back with
  --- `terminated = false` when the attempt budget runs out: the id stays
  --- valid, the actor is still spending against its cap, and the caller may
  --- come back to it or abort it.
  ---
  --- A read that fails costs an attempt and nothing else. The actor goes on
  --- spending whether or not the status endpoint is answering, so a 503 in the
  --- middle of a poll must not end the wait: the previous read stands, the
  --- loop carries on, and a run that never terminates comes back carrying
  --- `poll_error` — with its id, which is what lets the caller come back to it
  --- or abort it.
  function c:await(run_id, o)
    o = o or {}
    local wait_s = o.wait_s or DEFAULT_WAIT_S
    local timeout_s = o.timeout_s or DEFAULT_TIMEOUT_S
    local attempts = o.attempts or (math.ceil(timeout_s / math.max(wait_s, 1)) + 2)
    local poll_s = o.poll_s or DEFAULT_POLL_S
    local run = { id = run_id, actor = o.actor, terminated = false, succeeded = false }
    local poll_error
    for i = 1, attempts do
      local read, again = pcall(self.run_status, self, run_id, { wait_s = wait_s, actor = o.actor })
      if read then
        run = again
        if run.terminated then return run end
      else
        poll_error = tostring(again)
      end
      if i < attempts and poll_s > 0 then sleep(poll_s) end
    end
    if poll_error then run.poll_error = poll_error end
    return run
  end

  --- Every item in a dataset, cleaned (no empty records, no `#hidden` fields),
  --- read a page at a time. `opts.limit` caps the total; `opts.page_size` the
  --- request.
  function c:dataset_items(dataset_id, o)
    o = o or {}
    local page_size = o.page_size or DEFAULT_PAGE_SIZE
    local offset = o.offset or 0
    local items = {}
    while true do
      local want = page_size
      if o.limit then want = math.min(want, o.limit - #items) end
      if want <= 0 then break end
      local q = { format = "json", clean = (o.clean ~= false), offset = offset, limit = want }
      local where = "GET /datasets/" .. tostring(dataset_id) .. "/items"
      local target = base_url .. "/datasets/" .. tostring(dataset_id) .. "/items?" .. url.encode_form(q)
      local resp = http.get(target, { headers = headers() })
      if resp.status ~= 200 then fail(where, resp) end
      local page = decode(resp, where)
      if type(page) ~= "table" then error("apify: " .. where .. " returned no item list") end
      for _, item in ipairs(page) do items[#items + 1] = item end
      if #page < want then break end
      offset = offset + #page
    end
    return items
  end

  --- Re-read a finished run until its cost is in and stops moving. The events
  --- an actor charged reach the run record a few seconds after it finishes and
  --- the dollar figure a few seconds after that, so the first read after
  --- SUCCEEDED is usually zero.
  ---
  --- Each read that lands is progress towards the real figure, so a read that
  --- fails gives back the best figure reached so far with `settle_error` set
  --- rather than the one this call started from. The money is already spent by
  --- the time any of this runs; the worst answer is the one that forgets it.
  function c:settle(run, o)
    o = o or {}
    local gap = o.settle_s or DEFAULT_SETTLE_S
    for i = 1, (o.settle_reads or DEFAULT_SETTLE_READS) do
      if gap > 0 then sleep(gap) end
      local read, again = pcall(self.run_status, self, run.id, { actor = run.actor })
      if not read then
        run.settle_error = tostring(again)
        return run
      end
      local done = settled(again, run, i)
      run = again
      if done then break end
    end
    return run
  end

  function c:abort(run_id)
    local where = "POST /actor-runs/" .. tostring(run_id) .. "/abort"
    local resp = http.post(base_url .. "/actor-runs/" .. tostring(run_id) .. "/abort", "", { headers = headers() })
    if resp.status ~= 200 then fail(where, resp) end
    return to_run(unwrap(resp, where))
  end

  --- Start, wait, read. The result is the run (with `usage_total_usd` and
  --- `usage_cents`, so the spend can be written down) plus `items`.
  ---
  --- A run that ended any way but SUCCEEDED comes back as
  --- `nil, "run_failed" | "run_aborted" | "run_timed_out", result`, and one
  --- that did not end in time as `nil, "not_terminated", result`. The result
  --- travels with the reason in both cases because a failed run has usually
  --- both spent money and written part of its dataset, and a ledger that
  --- forgets failed runs under-counts.
  ---
  --- For the same reason no read that happens after the actor started is
  --- allowed to throw the run away. A poll that fails costs an attempt and the
  --- wait carries on; a settle read that fails leaves the run at the best
  --- figure reached, with `settle_error` set; a dataset read that fails gives
  --- `nil, "items_unreadable", result` with `items = {}` and `items_error`
  --- set, and a run that had already failed keeps its own reason rather than
  --- trading it for this one. `dataset_items` called directly still raises: a
  --- caller holding a run id has asked for the dataset, not for a best effort
  --- at it.
  function c:run(actor, input, o)
    o = o or {}
    local run = self:start(actor, input, {
      max_total_charge_usd = o.max_total_charge_usd,
      timeout_s = o.timeout_s,
      memory_mb = o.memory_mb,
      max_items = o.max_items,
      build = o.build,
      wait_s = o.wait_s or DEFAULT_WAIT_S,
    })
    if not run.terminated then
      run = self:await(run.id, {
        wait_s = o.wait_s,
        timeout_s = o.timeout_s,
        attempts = o.attempts,
        poll_s = o.poll_s,
        actor = actor,
      })
    end
    if run.terminated then run = self:settle(run, o) end
    run.items = {}
    local items_read = true
    if run.terminated and run.dataset_id then
      local read, items = pcall(self.dataset_items, self, run.dataset_id, { limit = o.max_items })
      items_read = read
      if read then
        run.items = items
      else
        run.items_error = tostring(items)
      end
    end
    if not run.terminated then return nil, "not_terminated", run end
    if not run.succeeded then
      return nil, "run_" .. run.status:lower():gsub("%-", "_"), run
    end
    if not items_read then return nil, "items_unreadable", run end
    return run
  end

  -- One run mapped record by record. On any failure the partial mapping
  -- travels with the reason, for the same reason the raw items do.
  local function typed(key, actor, input, o, map)
    local result, reason, partial = c:run(actor, input, o)
    local run = result or partial
    local out = { run = run }
    out[key] = {}
    if run then
      local from = source(actor, run.id)
      for _, item in ipairs(run.items or {}) do
        out[key][#out[key] + 1] = map(item, from)
      end
    end
    if not result then return nil, reason, out end
    return out
  end

  local function need_list(name, list)
    if type(list) ~= "table" or #list == 0 then
      error("apify: " .. name .. " needs a non-empty list")
    end
  end

  --- Profiles by username: counts, bio, links, the latest posts. Cheap — one
  --- event per profile.
  function c:instagram_profiles(usernames, o)
    need_list("instagram_profiles", usernames)
    local input = { usernames = usernames }
    if o and o.about then input.includeAboutSection = true end
    return typed("profiles", M.ACTORS.instagram_profiles, input, o, profile_from)
  end

  --- Comments under posts. Billed per comment, not per post: `limit` is per
  --- URL, so the bill is up to `#post_urls * limit` events.
  function c:instagram_comments(post_urls, limit, o)
    need_list("instagram_comments", post_urls)
    local input = { directUrls = post_urls, resultsType = "comments", resultsLimit = limit or 20 }
    return typed("comments", M.ACTORS.instagram_scraper, input, o, comment_from)
  end

  --- Recent posts under hashtags. This goes to the dedicated hashtag actor on
  --- purpose: the general scraper accepts a `hashtags` input and answers with
  --- a single post.
  function c:instagram_hashtag_posts(tags, limit, o)
    need_list("instagram_hashtag_posts", tags)
    local input = { hashtags = tags, resultsType = "posts", resultsLimit = limit or 20 }
    return typed("posts", M.ACTORS.instagram_hashtags, input, o, hashtag_post_from)
  end

  --- LinkedIn profiles by URL or public identifier, answered in the
  --- lead_provider person shape. `opts.with_email` switches the actor to its
  --- email-search mode, which costs two and a half times as much.
  function c:linkedin_profiles(urls, o)
    need_list("linkedin_profiles", urls)
    local input = { queries = urls, profileScraperMode = LINKEDIN_MODE[o ~= nil and o.with_email == true] }
    return typed("people", M.ACTORS.linkedin_profiles, input, o, linkedin_person_from)
  end

  --- Emails, phones and social profiles from each site, one merged row per
  --- start URL. Billed per page scraped, so `opts.max_pages` (default 5) is
  --- what the bill multiplies by: `#urls * max_pages` pages at most. The cap
  --- cannot go below fifty cents whatever the run will actually cost, so the
  --- page budget rather than the cap is what keeps a run small. A `linktr.ee`
  --- start URL is followed off-domain by design, which is the reason to pass
  --- one.
  ---
  --- Iframes are left unread by default: `opts.frames = true` turns them on,
  --- and brings in the contact details of whoever is advertising on the page
  --- alongside those of the site itself.
  function c:contact_details(urls, o)
    need_list("contact_details", urls)
    o = o or {}
    local cap = o.max_total_charge_usd
    -- A missing or nonsensical cap is the generic refusal's to name, not this
    -- one's; the floor only applies to a cap the caller actually chose.
    if type(cap) == "number" and cap == cap and cap > 0 and cap < CONTACT_MIN_CAP_USD then
      error("apify: " .. M.ACTORS.contact_details .. " refuses a cap below $0.50 whatever the run "
        .. "costs — raise max_total_charge_usd to at least 0.5 and hold the spend down with "
        .. "max_pages instead")
    end
    local max_pages = o.max_pages or 5
    local input = {
      startUrls = list_of(urls, function(u) return { url = u } end),
      maxRequestsPerStartUrl = max_pages,
      maxDepth = o.depth or 1,
      maxRequests = #urls * max_pages,
      sameDomain = o.same_domain ~= false,
      mergeContacts = true,
      considerChildFrames = o.frames == true,
      useBrowser = false,
      proxyConfig = { useApifyProxy = true },
      maximumLeadsEnrichmentRecords = 0,
    }
    return typed("sites", M.ACTORS.contact_details, input, o, contact_site_from)
  end

  return c
end

return M
