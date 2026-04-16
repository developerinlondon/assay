## db

SQL database access. No `require()` needed. Supports Postgres, MySQL, SQLite via connection URL.

- `db.connect(url)` → conn — Connect to database
  - URLs: `postgres://user:pass@host:5432/db`, `mysql://user:pass@host:3306/db`, `sqlite:///path.db`
- `db.query(conn, sql, params?)` → [row] — Execute query, return rows as tables
  - Parameterized: `db.query(conn, "SELECT * FROM users WHERE id = $1", {42})`
- `db.execute(conn, sql, params?)` → number — Execute statement, return affected row count
- `db.close(conn)` → nil — Close database connection
