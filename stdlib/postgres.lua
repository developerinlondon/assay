local M = {}

function M.client(host, port, username, password, database)
  local c = {}
  local dsn = "postgres://" .. username .. ":" .. password .. "@" .. host .. ":" .. tostring(port) .. "/" .. (database or "postgres")
  local pool = db.connect(dsn)

  function c:query(sql, params)
    return db.query(pool, sql, params or {})
  end

  function c:execute(sql, params)
    return db.execute(pool, sql, params or {})
  end

  function c:close()
    db.close(pool)
  end

  function c:user_exists(username_check)
    local rows = db.query(pool, "SELECT 1 FROM pg_roles WHERE rolname = $1", { username_check })
    return #rows > 0
  end

  function c:ensure_user(target_user, target_password, opts)
    opts = opts or {}
    if self:user_exists(target_user) then
      log.info("PostgreSQL user '" .. target_user .. "' already exists")
      return false
    end
    local create_sql = "CREATE USER " .. M._quote_ident(target_user)
      .. " WITH PASSWORD " .. M._quote_literal(target_password)
    if opts.createdb then
      create_sql = create_sql .. " CREATEDB"
    end
    if opts.superuser then
      create_sql = create_sql .. " SUPERUSER"
    end
    db.execute(pool, create_sql, {})
    log.info("Created PostgreSQL user '" .. target_user .. "'")
    return true
  end

  function c:database_exists(dbname)
    local rows = db.query(pool, "SELECT 1 FROM pg_database WHERE datname = $1", { dbname })
    return #rows > 0
  end

  function c:ensure_database(dbname, owner)
    if self:database_exists(dbname) then
      log.info("PostgreSQL database '" .. dbname .. "' already exists")
      return false
    end
    local create_sql = "CREATE DATABASE " .. M._quote_ident(dbname)
    if owner then
      create_sql = create_sql .. " OWNER " .. M._quote_ident(owner)
    end
    db.execute(pool, create_sql, {})
    log.info("Created PostgreSQL database '" .. dbname .. "'")
    return true
  end

  function c:grant(database_name, target_user, privileges)
    privileges = privileges or "ALL PRIVILEGES"
    local sql = "GRANT " .. privileges .. " ON DATABASE "
      .. M._quote_ident(database_name) .. " TO " .. M._quote_ident(target_user)
    db.execute(pool, sql, {})
    log.info("Granted " .. privileges .. " on " .. database_name .. " to " .. target_user)
  end

  return c
end

function M.client_from_vault(vault_client, vault_path, host, port)
  local data = vault_client:kv_get("secrets", vault_path)
  assert.not_nil(data, "postgres.client_from_vault: no secret at secrets/" .. vault_path)
  assert.not_nil(data.data, "postgres.client_from_vault: no data at secrets/" .. vault_path)
  local creds = data.data
  assert.not_nil(creds.username, "postgres.client_from_vault: missing username")
  assert.not_nil(creds.password, "postgres.client_from_vault: missing password")
  local database = creds.database or "postgres"
  return M.client(host, tostring(port or 5432), creds.username, creds.password, database)
end

-- SQL injection prevention: DDL statements cannot use parameterized queries
function M._quote_ident(s)
  return '"' .. s:gsub('"', '""') .. '"'
end

-- SQL injection prevention: DDL statements cannot use parameterized queries
function M._quote_literal(s)
  return "'" .. s:gsub("'", "''") .. "'"
end

return M
