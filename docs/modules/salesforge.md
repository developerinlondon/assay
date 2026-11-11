---
category: Registries
tagline: Salesforge sequencer — workspaces, mailboxes, sequences, contacts, do-not-contact, replies, webhooks, warm-up state and plan costs
---

## assay.salesforge

A client for the Salesforge sequencer. The public REST API (`/public/v2`) covers workspaces,
mailboxes, sequences, contacts, do-not-contact and replies. The web app's own API, reached by a
Firebase password sign-in, carries the warm-up state and the plan, which the public one does not.
Credentials come from the caller; the module reads no secret store.

```lua
local sf = require("assay.salesforge")

local c = sf.client({
  api_key = env.get("SALESFORGE_API_KEY"),  -- or opts.api_key
  workspace_id = "wks_…",                     -- required, a single id
  email = env.get("SALESFORGE_EMAIL"),        -- only the internal API uses these
  password = env.get("SALESFORGE_PASSWORD"),
})

local ok, existed = c:delete_contact("con_…")  -- true, true | true, false | nil, err
```

Options: `api_key` (or `SALESFORGE_API_KEY`), `workspace_id`, `email` / `password` (or
`SALESFORGE_EMAIL` / `SALESFORGE_PASSWORD`), and `base_url`, `internal_base_url`, `identity_url` for
tests. The key travels bare in `Authorization`; a `Bearer` prefix is refused by the vendor.

### Methods

| Method                                       | Returns             | Notes                                                                    |
| -------------------------------------------- | ------------------- | ------------------------------------------------------------------------ |
| `c:workspaces()`                             | `[workspace], meta` | Every workspace the key can see                                          |
| `c:mailboxes()`                              | `[box], meta`       | Public API; no warm-up state                                             |
| `c:sequences()`                              | `[sequence], meta`  |                                                                          |
| `c:sequence(id)`                             | `sequence`          | With its mailbox rotation                                                |
| `c:create_contact(fields)`                   | `contact`           | `firstName` is required by the vendor                                    |
| `c:enrol(sequence_id, contact_ids)`          | `true`              | An empty list is refused before sending                                  |
| `c:dnc(addresses)`                           | `true`              | Stop writing to these addresses                                          |
| `c:delete_contact(id)`                       | `true, existed`     | See below                                                                |
| `c:reply(mailbox_id, email_id, body)`        | `true`              | Reply on an existing thread                                              |
| `c:set_rotation(sequence_id, mailbox_ids)`   | `true`              | Replaces the rotation; an empty list clears it                           |
| `c:set_sequence_status(sequence_id, status)` | `true`              | `"paused"` or `"active"`                                                 |
| `c:sign_in()`                                | `true`              | Internal API sign-in; memoised, token never returned                     |
| `c:mailboxes_internal()`                     | `[box], meta`       | `warmupActivated`, `daysUntilWarm`, heat                                 |
| `c:mailbox_internal(id)`                     | `box`               | One box with its warm-up state                                           |
| `c:mailbox_id(id_or_address)`                | `id`                | Resolves an address; an id passes through                                |
| `c:connect_smtp(address, password, opts)`    | `box`               | `opts.first/last/smtp/imap/daily_limit`; warm-up stays off               |
| `c:set_warmup(id_or_address, on)`            | `box`               | Sets the switch and reads the box back                                   |
| `c:webhooks()`                               | `[webhook], meta`   | Capped at ten rows by the vendor                                         |
| `c:webhook(id)`                              | `webhook`           | The signing secret is not returned after creation                        |
| `c:create_webhook(spec)`                     | `webhook`           | `spec.url`, `spec.event`; one event per hook; the secret comes back once |
| `c:costs()`                                  | `{items, meta}`     | Plan, limits and credits left; the vendor names no price                 |

Every method returns `nil, err` on failure. `err.code` is one of `config`, `auth`, `plan` (HTTP 402,
the public API is Growth-plan-only), `rate_limit`, `server`, `http`, `transport`, `unreadable`,
`sign_in` or `not_found`, with `err.status` and `err.message`. List calls return
`meta = {truncated, cap, seen}`; `truncated` means a page cap stopped the walk.

### Ids

Every id that goes into a URL path (workspace, contact, sequence, mailbox, webhook) must be a single
segment of letters, digits, `_` or `-`; that alphabet is observed, not documented. Anything else
(`../x`, `a?b`) is a `config` error before any request, so an id cannot address a resource in
another workspace. A bad `workspace_id` raises at `sf.client`.

The one exception is `reply`'s email id, which may be an RFC Message-ID (`<x@y>`). It is
percent-encoded into a single path component instead, so `/`, `?`, `#` and `%` cannot escape it;
only an empty id, `.` and `..` are refused.

### delete_contact

`c:delete_contact(id)` deletes a contact record with `DELETE /workspaces/{ws}/contacts/{id}`, for
erasure requests. Salesforge documents only 204/500 for DELETE. Already-gone is confirmed by GET
(documented 404). Live behaviour of a repeat DELETE is unverified, so the raw DELETE status is
logged with `log.debug`.

- First, once per client, it reads `GET /workspaces/{ws}`. The vendor may answer 404 for an unknown
  workspace (inferred; only GET /workspaces documents 404), so an unknown workspace is a `config`
  error and never reads as "already gone".
- A 2xx answer returns `true, true`.
- Any other answer except 401, 402, 403 and 429 is settled by reading the contact back. A 404 there
  ("Contact Not Found" in the vendor's spec) returns `true, false`: the contact is not present in
  this workspace, so a retried erase succeeds. Anything else returns the DELETE's own error.
- 401, 402, 403 and 429 return their error as they are; a refused key says nothing about the
  contact.
