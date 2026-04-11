## assay.postgres

PostgreSQL database helpers. User/database management, grants, Vault integration.
Client: `postgres.client(host, port, username, password, database?)`. Database defaults to `"postgres"`.
Module helper: `M.client_from_vault(vault_client, vault_path, host, port?)`.

- `c:query(sql, params?)` → [row] — Execute SQL query, return rows
- `c:execute(sql, params?)` → number — Execute SQL statement, return affected count
- `c:close()` → nil — Close database connection
- `c:user_exists(username)` → bool — Check if PostgreSQL role exists
- `c:ensure_user(username, password, opts?)` → bool — Create user if not exists. `opts`: `{createdb, superuser}`. Returns true if created.
- `c:database_exists(dbname)` → bool — Check if database exists
- `c:ensure_database(dbname, owner?)` → bool — Create database if not exists. Returns true if created.
- `c:grant(database_name, username, privileges?)` → nil — Grant privileges. Default: `"ALL PRIVILEGES"`.
- `M.client_from_vault(vault_client, vault_path, host, port?)` → client — Create client using credentials from Vault KV. Port defaults to 5432.

Example:
```lua
local postgres = require("assay.postgres")
local vault = require("assay.vault")
local vc = vault.authenticated_client("http://vault:8200")
local pg = postgres.client_from_vault(vc, "myapp/postgres", "postgres.default.svc", 5432)
pg:ensure_user("myapp", crypto.random(16), {createdb = true})
pg:ensure_database("myapp_db", "myapp")
pg:grant("myapp_db", "myapp")
pg:close()
```
