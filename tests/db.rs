mod common;

use common::run_lua_local;

#[tokio::test]
async fn test_db_sqlite_create_and_insert() {
    run_lua_local(
        r#"
        local conn = db.connect("sqlite::memory:")
        db.execute(conn, "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)")
        local result = db.execute(conn, "INSERT INTO users (name, age) VALUES (?, ?)", {"Alice", 30})
        assert.eq(result.rows_affected, 1)
        db.close(conn)
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_db_sqlite_query_rows() {
    run_lua_local(
        r#"
        local conn = db.connect("sqlite::memory:")
        db.execute(conn, "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)")
        db.execute(conn, "INSERT INTO users (name, age) VALUES (?, ?)", {"Alice", 30})
        db.execute(conn, "INSERT INTO users (name, age) VALUES (?, ?)", {"Bob", 25})

        local rows = db.query(conn, "SELECT name, age FROM users ORDER BY name")
        assert.eq(#rows, 2)
        assert.eq(rows[1].name, "Alice")
        assert.eq(rows[1].age, 30)
        assert.eq(rows[2].name, "Bob")
        assert.eq(rows[2].age, 25)
        db.close(conn)
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_db_sqlite_query_with_params() {
    run_lua_local(
        r#"
        local conn = db.connect("sqlite::memory:")
        db.execute(conn, "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT, price REAL)")
        db.execute(conn, "INSERT INTO items (name, price) VALUES (?, ?)", {"Widget", 9.99})
        db.execute(conn, "INSERT INTO items (name, price) VALUES (?, ?)", {"Gadget", 19.99})
        db.execute(conn, "INSERT INTO items (name, price) VALUES (?, ?)", {"Doohickey", 4.99})

        local rows = db.query(conn, "SELECT name, price FROM items WHERE price > ? ORDER BY price", {5.0})
        assert.eq(#rows, 2)
        assert.eq(rows[1].name, "Widget")
        assert.eq(rows[2].name, "Gadget")
        db.close(conn)
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_db_sqlite_empty_result() {
    run_lua_local(
        r#"
        local conn = db.connect("sqlite::memory:")
        db.execute(conn, "CREATE TABLE empty_table (id INTEGER PRIMARY KEY, val TEXT)")
        local rows = db.query(conn, "SELECT * FROM empty_table")
        assert.eq(#rows, 0)
        db.close(conn)
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_db_sqlite_execute_returns_rows_affected() {
    run_lua_local(
        r#"
        local conn = db.connect("sqlite::memory:")
        db.execute(conn, "CREATE TABLE data (id INTEGER PRIMARY KEY, val TEXT)")
        db.execute(conn, "INSERT INTO data (val) VALUES (?)", {"a"})
        db.execute(conn, "INSERT INTO data (val) VALUES (?)", {"b"})
        db.execute(conn, "INSERT INTO data (val) VALUES (?)", {"c"})

        local result = db.execute(conn, "DELETE FROM data WHERE val IN (?, ?)", {"a", "c"})
        assert.eq(result.rows_affected, 2)
        db.close(conn)
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_db_sqlite_multiple_connections() {
    run_lua_local(
        r#"
        local conn1 = db.connect("sqlite::memory:")
        local conn2 = db.connect("sqlite::memory:")

        db.execute(conn1, "CREATE TABLE t1 (id INTEGER PRIMARY KEY, val TEXT)")
        db.execute(conn2, "CREATE TABLE t2 (id INTEGER PRIMARY KEY, val TEXT)")

        db.execute(conn1, "INSERT INTO t1 (val) VALUES (?)", {"from_conn1"})
        db.execute(conn2, "INSERT INTO t2 (val) VALUES (?)", {"from_conn2"})

        local rows1 = db.query(conn1, "SELECT val FROM t1")
        local rows2 = db.query(conn2, "SELECT val FROM t2")

        assert.eq(#rows1, 1)
        assert.eq(rows1[1].val, "from_conn1")
        assert.eq(#rows2, 1)
        assert.eq(rows2[1].val, "from_conn2")

        db.close(conn1)
        db.close(conn2)
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_db_sqlite_invalid_sql() {
    let result = run_lua_local(
        r#"
        local conn = db.connect("sqlite::memory:")
        db.execute(conn, "NOT VALID SQL")
    "#,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("db.execute"),
        "error should mention db.execute: {err}"
    );
}

#[tokio::test]
async fn test_db_sqlite_null_values() {
    run_lua_local(
        r#"
        local conn = db.connect("sqlite::memory:")
        db.execute(conn, "CREATE TABLE nullable (id INTEGER PRIMARY KEY, val TEXT)")
        db.execute(conn, "INSERT INTO nullable (val) VALUES (NULL)")

        local rows = db.query(conn, "SELECT val FROM nullable")
        assert.eq(#rows, 1)
        assert.eq(rows[1].val, nil)
        db.close(conn)
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_db_sqlite_query_no_params() {
    run_lua_local(
        r#"
        local conn = db.connect("sqlite::memory:")
        db.execute(conn, "CREATE TABLE simple (id INTEGER PRIMARY KEY, name TEXT)")
        db.execute(conn, "INSERT INTO simple (name) VALUES ('test')")

        local rows = db.query(conn, "SELECT name FROM simple")
        assert.eq(#rows, 1)
        assert.eq(rows[1].name, "test")
        db.close(conn)
    "#,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn test_db_sqlite_close() {
    run_lua_local(
        r#"
        local conn = db.connect("sqlite::memory:")
        db.execute(conn, "CREATE TABLE t (id INTEGER PRIMARY KEY)")
        db.close(conn)
    "#,
    )
    .await
    .unwrap();
}
