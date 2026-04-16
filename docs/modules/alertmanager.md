## assay.alertmanager

Alertmanager alert and silence management. Query, create, and delete alerts and silences. Client:
`alertmanager.client(url)`.

- `c.alerts:list(opts?)` → [alert] — List alerts. `opts`:
  `{active, silenced, inhibited, unprocessed, filter, receiver}`
- `c.alerts:post(alerts)` → true — Post new alerts (array of alert objects)
- `c.alerts:groups(opts?)` → [group] — List alert groups. `opts`:
  `{active, silenced, inhibited, filter, receiver}`
- `c.alerts:is_firing(alertname)` → bool — Check if a specific alert is currently firing
- `c.alerts:active_count()` → number — Count active non-silenced, non-inhibited alerts
- `c.silences:list(opts?)` → [silence] — List silences. `opts`: `{filter}`
- `c.silences:get(id)` → silence — Get silence by ID
- `c.silences:create(silence)` → `{silenceID}` — Create a silence
- `c.silences:delete(id)` → true — Delete silence by ID
- `c.silences:silence_alert(alertname, duration_hours, opts?)` → silenceID — Silence an alert by
  name for N hours. `opts`: `{created_by, comment}`
- `c.status:get()` → `{cluster, config}` — Get Alertmanager status and cluster info
- `c.receivers:list()` → [receiver] — List notification receivers

Example:

```lua
local am = require("assay.alertmanager")
local c = am.client("http://alertmanager:9093")
local firing = c.alerts:is_firing("HighCPU")
if firing then
  c.silences:silence_alert("HighCPU", 2, {comment = "Investigating"})
end
```
