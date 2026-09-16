---
category: Registries
tagline: Apify actor runs behind a mandatory spend cap — start, wait, read the dataset and what it cost; typed Instagram, LinkedIn and contact-details readers
---

## assay.apify

Runs any [Apify](https://apify.com) actor and reads its dataset back, with the run's cost on the
result so a caller can write the spend down. Five typed readers sit on top for the actors a
person-centric lead pipeline reaches for: Instagram profiles, comments and hashtag posts, LinkedIn
profiles, and the contact details behind a website.

```lua
local apify = require("assay.apify")
local c = apify.client({ token = env.get("APIFY_TOKEN") })

local r = c:instagram_profiles({ "natgeo", "humansofny" }, { max_total_charge_usd = 0.10 })
for _, p in ipairs(r.profiles) do
  print(p.username, p.followers, p.category, p.external_url)
end
print(r.run.usage_total_usd, r.run.usage_cents)   -- what the run cost

local any = c:run("apify/google-search-scraper", { queries = "bachata festival 2027" },
  { max_total_charge_usd = 0.50, timeout_s = 120 })
```

The token comes from the caller — `opts.token` or `APIFY_TOKEN`. The module reads no secret store.

### Every run carries a cap

An actor billed per event has no price until it has run. `max_total_charge_usd` is therefore
**required on every run**, typed helpers included; a call without it raises before anything reaches
the network. The cap goes to the API as `maxTotalChargeUsd`, and the actor stops when it is reached.
`timeout_s` (default 300) rides along as the run timeout.

### What a run returns

`run()` starts the actor, waits, reads the default dataset and answers with the run:

| Field                                                      | Meaning                                                                              |
| ---------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| `id`, `actor`, `dataset_id`                                | the run, the actor it was asked for, where the items live                            |
| `status`, `terminated`, `succeeded`                        | `SUCCEEDED`, `FAILED`, `TIMED-OUT`, `ABORTED` are terminal                           |
| `items`                                                    | every clean dataset item, read a page at a time                                      |
| `usage_total_usd`                                          | the vendor's figure for the run                                                      |
| `usage_cents`                                              | the same rounded **up** to whole cents, so a ledger of quarter-cent runs is not zero |
| `charged_event_counts`                                     | the pay-per-event tally (`profile`, `result`, …)                                     |
| `max_total_charge_usd`                                     | the cap the run was started with, as the API recorded it                             |
| `started_at`, `finished_at`, `status_message`, `exit_code` | as reported                                                                          |
| `poll_error`, `settle_error`, `items_error`                | set when a read that happens after the run started failed                            |

A run that ends any way but `SUCCEEDED` comes back as
`nil, "run_failed" | "run_aborted" |
"run_timed_out", result`, and one that has not ended when the
attempt budget runs out as `nil, "not_terminated", result`. The result travels with the reason in
both cases: a failed run has usually both spent money and written part of its dataset, and a ledger
that forgets failed runs under-counts. An unfinished run is still running against its cap;
`c:abort(run.id)` stops it.

For the same reason no read that happens after the actor started is allowed to throw the run away —
the actor spends whether or not the API is answering:

- A **poll** that fails costs an attempt and nothing else. The previous read stands and the wait
  carries on, so one 503 in the middle of a three-minute run does not end it. A run whose polls
  never succeed comes back as `nil, "not_terminated", result` carrying its `id` — which is what lets
  the caller abort it — and `poll_error`.
- A **settle** read that fails gives back the best figure reached so far, not the one the call
  started from, with `settle_error` set. The run terminates reporting zero, so every read that
  landed is progress worth keeping. The items are still read.
- A **dataset** read that fails gives `nil, "items_unreadable", result` with `items = {}` and
  `items_error` set, the result still carrying `id` and the cost. A run that had already failed
  keeps its own reason rather than trading it for the dataset's.

Called directly, `dataset_items` still raises: a caller holding a run id has asked for the dataset,
not for a best effort at it. `settle` is the other way round — it is handed a run that already cost
money, so it answers with that run and `settle_error` rather than raising over it.

The one failure this module cannot report a run id for is a `201` from `start` whose body will not
parse: the id was in that body. The raise carries the body verbatim so the id can be recovered from
it, or the run found in the Apify console by its actor and start time.

The bill lands after the run does: measured against the profile and hashtag actors, the event counts
reach the run record about three seconds after `SUCCEEDED` and the dollar figure about seven. So
after a terminal status the run is read again, `settle_s` apart (default 2), until two consecutive
reads agree and the figure is in — never before the third read, since the events of one run land one
at a time and two early reads can agree on a partial bill — up to `settle_reads` times (default 8).
A long-tailed bill can still settle later than that; `run_status(id)` read a minute on gives the
final figure for a ledger that wants it exact. A caller that cannot wait sets `settle_reads = 0` and
takes the first figure.

Polling uses the API's own long wait: each status read asks the server to hold the request for
`wait_s` seconds (default 20, under the HTTP client's 30-second limit), so a three-minute run is a
handful of requests rather than a hundred.

The lower-level pieces are exposed for callers that want to hold a run id across their own
scheduling: `start(actor, input, opts)`, `run_status(run_id, { wait_s })`,
`await(run_id, { wait_s, attempts, poll_s })`,
`dataset_items(dataset_id, { limit, page_size, offset })`, `abort(run_id)`.

### Typed readers

Each answers `{ <records>, run }` — the mapped records plus the run above — and on failure
`nil, reason, { <records>, run }` with whatever was mapped before it failed.

| Reader                                   | Actor                                 | Records    | Billed per            |
| ---------------------------------------- | ------------------------------------- | ---------- | --------------------- |
| `instagram_profiles(usernames, opts)`    | `apify/instagram-profile-scraper`     | `profiles` | profile (about ¼ ¢)   |
| `instagram_comments(post_urls, n, opts)` | `apify/instagram-scraper`             | `comments` | **comment**, not post |
| `instagram_hashtag_posts(tags, n, opts)` | `apify/instagram-hashtag-scraper`     | `posts`    | post                  |
| `linkedin_profiles(urls, opts)`          | `harvestapi/linkedin-profile-scraper` | `people`   | profile               |
| `contact_details(urls, opts)`            | `vdrmota/contact-info-scraper`        | `sites`    | **page scraped**      |

A profile: `username`, `full_name`, `biography`, `followers`, `follows`, `posts_count`,
`external_url`, `external_urls`, `profile_pic_url`, `verified`, `private`, `business`, `category`,
`latest_posts` (each with `shortcode`, `url`, `type`, `caption`, `likes`, `comments`, `taken_at`,
`hashtags`, `mentions`) and `provenance`. Counts are numbers even when the vendor sends them as
strings, which it sometimes does for `followersCount`; a category the vendor writes as the string
`"None"` is nil. A username the actor could not read comes back with `error` set and no counts, so a
miss is visible rather than dropped.

A comment: `text`, `owner_username`, `owner_id`, `owner_verified`, `likes`, `replies`, `taken_at`,
`post_url`, `comment_url`. `n` is **per URL**, and the bill is up to `#post_urls × n` comment
events; forty comments under each of seventy posts is a few dollars, not a few cents.

A hashtag post: the post fields above plus `owner_username`, `owner_full_name`, `owner_id`,
`video_views`, `display_url`, `location`, `sponsored` and `tag` (read back from the tag URL the
actor records on each item). This goes to the dedicated hashtag actor on purpose — the general
scraper accepts a `hashtags` input and answers with a single post.

A LinkedIn record is a [`lead_provider`](lead_provider.html) person (`first_name`, `last_name`,
`full_name`, `title`, `company`, `location`, `linkedin`, `emails`, `provenance`) with
`public_identifier`, `headline`, `about`, `photo` and `followers` added. The cheaper no-email mode
is the default; `with_email = true` switches the actor to its email search at two and a half times
the price. An address it finds is recorded as `email_type = "provider"` with
`verification_status =
"UNKNOWN"`: a vendor asserting deliverability is not a delivery, so nothing
here reaches `VERIFIED`.

A contact row is one merged record per start URL: `url`, `domain`, `emails`, `phones`,
`phones_uncertain` (digit runs the actor found but would not vouch for), `linkedins`, `instagrams`,
`twitters`, `facebooks`, `youtubes`, `tiktoks`, `pages_visited` and `provenance`. Every list is
present and de-duplicated even when nothing was found, and emails are lower-cased, so a caller
counting addresses need not nil-check each network. The bill is per **page**, so `max_pages`
(default 5) is what it multiplies by: the whole scrape is bounded at `#urls × max_pages`, not just
each start URL, and `depth` (default 1) says how far from each start URL to follow links.
`same_domain` (default true) keeps the crawl on the site — except on a [Linktree](https://linktr.ee)
start URL, which the actor follows off-domain by design, so the link-in-bio from an Instagram
profile resolves to the sites behind it in the same run. The business-leads and email-verification
add-ons are held off; both carry personal data and their own per-record price. Iframes are left
unread unless `frames = true`, since they carry the contact details of whoever is advertising on the
page alongside the site's own. This actor refuses a cap below **$0.50** whatever the run will
actually cost, so `contact_details` raises on a smaller one before any request rather than letting
the API answer `400`, and the page budget rather than the cap is what keeps a run small.

Provenance on every record is
`{ provider = "apify", retrieved_from = "<actor> run <run id>",
retrieved_at }`, so a fact can be
traced to the run that bought it and the dataset that still holds the raw item.

### Where the budget gate sits

The [`lead_provider`](lead_provider.html) gate prices an operation before the call. An Apify run
cannot be priced before it runs, so this module does not take a gate: the cap is the approval, and
`usage_cents` on the result is what to meter. A caller that keeps a ledger approves the cap, runs,
and records `usage_cents` against the run id.
