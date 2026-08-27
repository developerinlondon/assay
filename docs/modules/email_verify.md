---
category: Registries
tagline: Email verification — syntax, MX over DoH, list signals, and a live SMTP probe
---

## assay.email_verify

Keyless email verification in two rungs. `verify` is free and fully deterministic: refuse the
addresses that cannot work, generate the pattern candidates worth checking, and say honestly how far
DNS evidence goes. `probe` adds live SMTP evidence on top of it, using the `smtp_probe` builtin
compiled into the binary — no sidecar, no vendor key.

`verify`'s vocabulary is deliberately capped (NEP-0007 §2): it returns only `INVALID` (syntax, or a
domain that cannot receive mail) or `UNKNOWN` (the domain accepts mail; nothing proves this
mailbox). Reaching `PROBABLE` or `CATCH_ALL` requires `probe`.

### Static helpers

```lua
local ev = require("assay.email_verify")
ev.check_syntax("jane.doe@example.com")      -- true | false, reason
ev.candidates("Jane", "Doe", "example.com")  -- { "jane.doe@…", "janedoe@…", … }
```

### Client

```lua
local c = ev.client()                         -- doh_url override for tests/proxies
local hosts, weak = c:mx("example.com")       -- MX hosts, lowest preference first
local v = c:verify("jane.doe@example.com")    -- { email, status, method, mx, checked_at }
```

`mx` falls back to address records when the zone has no MX (RFC 5321) and marks that evidence
`weak`; `verify` reports it as `method = "dns:a_fallback"`.

### List signals

Flags, never a status. A shortlist can be wrong, and NEP-0007 §2 makes `INVALID` permanent — so
membership hints at a problem and leaves the verdict to actual evidence.

```lua
ev.is_disposable("mailinator.com")   -- true; throwaway provider
ev.is_role("info@cheaney.co.uk")     -- true; reaches a desk, not a person
ev.suggest("jane@gmial.com")         -- "jane@gmail.com"; nil when nothing is close
```

`suggest` uses optimal string alignment against the major consumer providers, so an adjacent
transposition — the typo people actually make — counts as one edit.

### SMTP probe

```lua
local v = c:probe("jane.doe@example.com", { from = "probe@yourdomain.com" })
```

`opts.from` is required and must be an address on a domain you control: receiving servers judge the
envelope sender, and a bogus one earns the probing IP a reputation hit that outlives the lookup.
Other options: `helo`, `port` (25), `catch_all` (true), `connect_timeout_ms`, `op_timeout_ms`,
`greylist_delay_ms` (3000; `0` disables the retry).

The dialogue is envelope-only — connect, greeting, EHLO (falling back to HELO), MAIL FROM, a
random-address RCPT to detect catch-alls, the target RCPT, QUIT. **DATA is never issued**, so a
probe cannot deliver anything.

| SMTP evidence                                 | `status`    | `method`                                   |
| --------------------------------------------- | ----------- | ------------------------------------------ |
| Target RCPT accepted                          | `PROBABLE`  | `smtp:accepted`                            |
| Unowned address also accepted                 | `CATCH_ALL` | `smtp:catch_all`                           |
| Mailbox named as absent (any 4xx/5xx wording) | `INVALID`   | `smtp:no_mailbox`                          |
| `554` not allowed, `551` moved                | `INVALID`   | `smtp:not_allowed`, `smtp:recipient_moved` |
| Greylisted, blocked, full inbox, unreachable  | `UNKNOWN`   | `smtp:<reason>`                            |

An accepted RCPT stops at `PROBABLE` on purpose: servers accept at RCPT and bounce afterwards.
`VERIFIED` is reserved for a delivery that actually happened.

The verdict carries `smtp` (the full probe result), plus `mx_host`, `full_inbox`, `greylisted`,
`disposable`, `role`, `suggestion` and `detail`.

### The caveat that matters

**Probing the large consumer and enterprise gateways from a datacenter IP is unreliable.** Gmail and
Microsoft 365 routinely accept every RCPT regardless of whether the mailbox exists, throttle or
block unknown probing IPs outright, and greylist first contact. A `CATCH_ALL` or `UNKNOWN` from one
of those hosts says something about the gateway, not about the address.

For gateway-fronted domains during warm-up, a paid verifier remains the honest backstop; the
warden's own bounce data (NEP-0007 §7) is what decides when it can be dropped.

### Port 25 egress is a deployment prerequisite

The probe speaks SMTP on port 25, and most consumer ISPs and several cloud providers silently drop
outbound connections to it. On such a host every probe returns `UNKNOWN` with
`method = "smtp:unreachable"` — indistinguishable at a glance from a genuinely unreachable mail
server. Confirm egress before trusting a run:

```sh
timeout 6 bash -c 'exec 3<>/dev/tcp/alt1.aspmx.l.google.com/25 && head -1 <&3'
```

A `220` greeting means the host can probe; a timeout means it cannot, and the verification worker
belongs somewhere else.
