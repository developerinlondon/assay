## assay.postgres

PostgreSQL database helpers. User/database management, grants, Vault integration. Client:
`postgres.client(host, port, username, password, database?)`. Database defaults to `"postgres"`.
Module helper: `M.client_from_vault(vault_client, vault_path, host, port?)`.

### Queries

- `c.queries:query(sql, params?)` -> [row] -- Execute SQL query, return rows
- `c.queries:execute(sql, params?)` -> number -- Execute SQL statement, return affected count

### Connection

- `c:close()` -> nil -- Close database connection

### Users

- `c.users:exists(username)` -> bool -- Check if PostgreSQL role exists
- `c.users:ensure(username, password, opts?)` -> bool -- Create user if not exists. `opts`:
  `{createdb, superuser}`. Returns true if created.

### Databases

- `c.databases:exists(dbname)` -> bool -- Check if database exists
- `c.databases:ensure(dbname, owner?)` -> bool -- Create database if not exists. Returns true if
  created.
- `c.databases:grant(database_name, username, privileges?)` -> nil -- Grant privileges. Default:
  `"ALL PRIVILEGES"`.

### Module Helpers

- `M.client_from_vault(vault_client, vault_path, host, port?)` -> client -- Create client using
  credentials from Vault KV. Port defaults to 5432.

Example:

```lua
local postgres = require("assay.postgres")
local vault = require("assay.vault")
local vc = vault.authenticated_client("http://vault:8200")
local pg = postgres.client_from_vault(vc, "myapp/postgres", "postgres.default.svc", 5432)
pg.users:ensure("myapp", crypto.random(16), {createdb = true})
pg.databases:ensure("myapp_db", "myapp")
pg.databases:grant("myapp_db", "myapp")
pg:close()
```
