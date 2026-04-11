## assay.alertmanager

Alertmanager alert and silence management. Query, create, and delete alerts and silences.
Module-level functions (no client needed): `M.function(url, ...)`.

- `M.alerts(url, opts?)` → [alert] — List alerts. `opts`: `{active, silenced, inhibited, unprocessed, filter, receiver}`
- `M.post_alerts(url, alerts)` → true — Post new alerts (array of alert objects)
- `M.alert_groups(url, opts?)` → [group] — List alert groups. `opts`: `{active, silenced, inhibited, filter, receiver}`
- `M.silences(url, opts?)` → [silence] — List silences. `opts`: `{filter}`
- `M.silence(url, id)` → silence — Get silence by ID
- `M.create_silence(url, silence)` → `{silenceID}` — Create a silence
- `M.delete_silence(url, id)` → true — Delete silence by ID
- `M.status(url)` → `{cluster, config}` — Get Alertmanager status and cluster info
- `M.receivers(url)` → [receiver] — List notification receivers
- `M.is_firing(url, alertname)` → bool — Check if a specific alert is currently firing
- `M.silence_alert(url, alertname, duration_hours, opts?)` → silenceID — Silence an alert by name for N hours. `opts`: `{created_by, comment}`
- `M.active_count(url)` → number — Count active non-silenced, non-inhibited alerts

Example:
```lua
local am = require("assay.alertmanager")
local firing = am.is_firing("http://alertmanager:9093", "HighCPU")
if firing then
  am.silence_alert("http://alertmanager:9093", "HighCPU", 2, {comment = "Investigating"})
end
```
