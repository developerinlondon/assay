## assay.unleash

Unleash feature flag management. Projects, features, environments, strategies, API tokens.
Client: `unleash.client(url, {token="..."})`.
Module helpers: `M.wait()`, `M.ensure_project()`, `M.ensure_environment()`, `M.ensure_token()`.

### Health

- `c:health()` → `{health}` — Check Unleash health

### Projects

- `c:projects()` → [project] — List projects
- `c:project(id)` → project|nil — Get project by ID
- `c:create_project(project)` → project — Create project. `project`: `{id, name, description?}`
- `c:update_project(id, project)` → project — Update project
- `c:delete_project(id)` → nil — Delete project

### Environments

- `c:environments()` → [environment] — List all environments
- `c:enable_environment(project_id, env_name)` → nil — Enable environment on project
- `c:disable_environment(project_id, env_name)` → nil — Disable environment on project

### Features

- `c:features(project_id)` → [feature] — List features in project
- `c:feature(project_id, name)` → feature|nil — Get feature by name
- `c:create_feature(project_id, feature)` → feature — Create feature. `feature`: `{name, type?, description?}`
- `c:update_feature(project_id, name, feature)` → feature — Update feature
- `c:archive_feature(project_id, name)` → nil — Archive (soft-delete) a feature
- `c:toggle_on(project_id, name, env)` → nil — Enable feature in environment
- `c:toggle_off(project_id, name, env)` → nil — Disable feature in environment

### Strategies

- `c:strategies(project_id, feature_name, env)` → [strategy] — List strategies for feature in environment
- `c:add_strategy(project_id, feature_name, env, strategy)` → strategy — Add strategy. `strategy`: `{name, parameters?}`

### API Tokens

- `c:tokens()` → [token] — List API tokens
- `c:create_token(token_config)` → token — Create token. `token_config`: `{username, type, environment?, projects?}`
- `c:delete_token(secret)` → nil — Delete API token by secret

### Module Helpers

- `M.wait(url, opts?)` → true — Wait for Unleash healthy. `opts`: `{timeout, interval}`. Default 60s.
- `M.ensure_project(client, project_id, opts?)` → project — Ensure project exists. `opts`: `{name, description}`
- `M.ensure_environment(client, project_id, env_name)` → true — Ensure environment enabled on project
- `M.ensure_token(client, opts)` → token — Ensure API token exists. `opts`: `{username, type, environment?, projects?}`

Example:
```lua
local unleash = require("assay.unleash")
unleash.wait("http://unleash:4242")
local c = unleash.client("http://unleash:4242", {token = env.get("UNLEASH_ADMIN_TOKEN")})
unleash.ensure_project(c, "my-project", {name = "My Project"})
unleash.ensure_environment(c, "my-project", "production")
c:create_feature("my-project", {name = "dark-mode", type = "release"})
c:toggle_on("my-project", "dark-mode", "production")
```
