---
category: Registries
tagline: Free email verification — syntax, MX over DoH, and pattern candidates
---

## assay.email_verify

The waterfall's free rung: refuse the addresses that cannot work, generate the pattern
candidates worth checking, and say honestly how far free evidence goes. Keyless and fully
deterministic — DNS answers come over DNS-over-HTTPS, so tests mock one HTTP endpoint.

The vocabulary is deliberately capped (NEP-0007 §2): this module returns only `INVALID`
(syntax or a domain that cannot receive mail) or `UNKNOWN` (the domain accepts mail; nothing
proves this mailbox). `PROBABLE`, `VERIFIED` and `CATCH_ALL` belong to the SMTP probe or a
paid verifier — the free rung never inflates its own evidence.

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

`mx` falls back to address records when the zone has no MX (RFC 5321) and marks that
evidence `weak`; `verify` reports it as `method = "dns:a_fallback"`.
