## assay.ansi

ANSI SGR → HTML conversion + stripper for log viewers. Pure Lua, no state, no dependencies — safe to
call on every log line rendered in a browser.

### `to_html(line) → string`

HTML-escapes `&`, `<`, `>`, `"`, `'` first, then converts SGR (`ESC [ ... m`) sequences into
`<span class="ansi-...">` tags. Recognised codes: `0` reset, `1` bold (`ansi-bold`), `30..37` /
`90..97` foreground (`ansi-fg-N`), `40..47` / `100..107` background (`ansi-bg-N`), `39` default fg,
`49` default bg. Unknown SGR codes and non-SGR CSI sequences (cursor moves, erase line, …) are
silently dropped. Spans are always closed at reset, default-fg/bg, and end-of-input.

```lua
local ansi = require("assay.ansi")
local html = ansi.to_html("\27[32mok\27[0m")
-- html == '<span class="ansi-fg-32">ok</span>'
```

### `strip(line) → string`

Removes every ANSI CSI sequence (any final byte in `0x40..0x7E`, not just `m`) and returns the
surrounding text byte-for-byte. Does **not** HTML-escape — the result is plain text, suitable for
non-browser sinks.

```lua
local ansi = require("assay.ansi")
local plain = ansi.strip("\27[32mhi\27[0m")
-- plain == "hi"
```
