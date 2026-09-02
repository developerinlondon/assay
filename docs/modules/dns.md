---
category: Builtins
---

## dns

DNS lookups. No `require()` needed.

Speaks the protocol directly rather than going through the C library's stub resolver, because
`getaddrinfo` can only answer "what address" — and domain health is a question about MX, TXT and
blacklists.

- `dns.lookup(name, type, opts?)` → array — Look up one record type
  - `type`: `"A"` | `"AAAA"` | `"CNAME"` | `"MX"` | `"NS"` | `"TXT"`, case-insensitive
  - `opts`: `{server = "1.1.1.1", timeout_ms = 5000, tries = 2}` — all optional
  - Returns an array of strings, except `MX`, which returns `{preference, exchange}` tables
- `dns.dnsbl(domain, list, opts?)` → `{listed, codes}` — Ask a DNS blacklist about a domain
  - Queries `<domain>.<list>` for `A` records; `opts` is the same table `dns.lookup` takes

### Where the query goes

By default, the nameservers in `/etc/resolv.conf`, in the order that file lists them. There is no
public fallback: a script that believes it is asking the corporate resolver should not silently ask
someone else's.

`opts.server` overrides that, and accepts a bare address (`"1.1.1.1"`), an address with a port
(`"1.1.1.1:5353"`), or a bracketed v6 address (`"[2606:4700:4700::1111]:53"`). With a policy
installed the option is refused rather than quietly ignored — a caller-chosen resolver is an egress
channel, since a restricted script could carry data out in the names it looks up.

A query goes out over UDP with an EDNS0 buffer of 1232 bytes, which is what keeps ordinary DKIM keys
from being truncated. If the answer still does not fit, the same question is asked again over TCP;
that is the protocol's own remedy and does not consume a try. `tries` covers unanswered queries, and
each try walks the whole server list.

### What each type answers with

```lua
local ips = dns.lookup("example.com", "A")
-- { "93.184.216.34" }

local mx = dns.lookup("example.com", "MX")
-- { { preference = 1, exchange = "aspmx.l.google.com" }, ... }

local txt = dns.lookup("example.com", "TXT")
-- { "v=spf1 include:_spf.google.com ~all" }
```

`MX` answers arrive sorted by `preference`, lowest first, so `mx[1]` is the host to try. Names in
`MX`, `NS` and `CNAME` answers come back lowercased and without the trailing root dot.

A `TXT` record travels the wire as chunks of at most 255 bytes each; they are one value the format
had to cut up, so they are rejoined with nothing between them. That is what makes a 2048-bit DKIM
key readable — a separator would corrupt every key long enough to need two chunks. Two separate
`TXT` records stay two separate strings.

A response to an `A` query often carries the `CNAME` chain that led to it. Only records of the type
you asked for are returned.

### Absent versus broken

A name that does not exist (`NXDOMAIN`) is an empty array. Anything else the resolver says went
wrong — `SERVFAIL`, `REFUSED`, a timeout — raises an error naming the reason:

```lua
local ok, err = pcall(dns.lookup, "example.com", "TXT")
-- err: "dns.lookup example.com TXT: 10.0.0.53:53: SERVFAIL"
```

The distinction is the point. "Nothing lists this domain" and "nobody answered" look identical if
failures collapse into an empty result, and they mean opposite things to whoever is about to send
mail.

### Blacklists

```lua
local r = dns.dnsbl("example.com", "fresh.spameatingmonkey.net")
if r.listed then
  log.warn("listed: " .. table.concat(r.codes, ", "))
end
```

A list answers in `127.0.0.0/8`, using the low octets to say which of its sub-lists matched.
Everything in that range counts as listed except the whole of `127.255.255.0/24`, which Spamhaus,
SURBL and others reserve for turning away resolvers they do not serve. Treating that reply as a hit
would mark every domain you check as blacklisted, so it does not count — but it is still reported in
`codes`, so you can tell "not listed" from "not allowed to ask".

`codes` carries every `A` answer whatever the verdict. A resolver failure raises rather than
returning `listed = false`.

### Example

`examples/domain-health.lua` checks MX, SPF, DKIM, DMARC and a blacklist for a domain, which is the
full set of questions behind "can this domain send mail, and will it arrive".
