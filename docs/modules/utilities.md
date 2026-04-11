## log

Structured logging. No `require()` needed.

- `log.info(msg)` — Log info message
- `log.warn(msg)` — Log warning message
- `log.error(msg)` — Log error message

## env

Environment variable access. No `require()` needed.

- `env.get(key)` → string|nil — Get environment variable value

## sleep

Sleep utility. No `require()` needed.

- `sleep(secs)` → nil — Sleep for N seconds (supports fractional: `sleep(0.5)`)

## time

Timestamp utility. No `require()` needed.

- `time()` → number — Unix timestamp in seconds (with fractional milliseconds)
