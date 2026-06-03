use super::json::json_value_to_lua;
use mlua::{Lua, Table, UserData, Value};
use sqlx::any::AnyRow;
use sqlx::postgres::{PgArguments, PgPoolOptions, PgRow};
use sqlx::types::BigDecimal;
use sqlx::{AnyPool, Column, PgPool, Row, ValueRef};
use std::sync::Arc;

#[derive(Clone)]
enum DbPool {
    Any(Arc<AnyPool>),
    Postgres(Arc<PgPool>),
}

impl UserData for DbPool {}

pub fn register_db(lua: &Lua) -> mlua::Result<()> {
    sqlx::any::install_default_drivers();

    let db_table = lua.create_table()?;

    let connect_fn = lua.create_async_function(|lua, url: String| async move {
        if is_postgres_url(&url) {
            let pool = PgPoolOptions::new()
                .max_connections(5)
                .connect(&url)
                .await
                .map_err(|e| mlua::Error::runtime(format!("db.connect: {e}")))?;
            return lua.create_any_userdata(DbPool::Postgres(Arc::new(pool)));
        }

        let pool = sqlx::any::AnyPoolOptions::new()
            .max_connections(if url.starts_with("sqlite:") { 1 } else { 5 })
            .connect(&url)
            .await
            .map_err(|e| mlua::Error::runtime(format!("db.connect: {e}")))?;
        lua.create_any_userdata(DbPool::Any(Arc::new(pool)))
    })?;
    db_table.set("connect", connect_fn)?;

    let query_fn = lua.create_async_function(|lua, args: mlua::MultiValue| async move {
        let mut args_iter = args.into_iter();

        let pool = extract_db_pool(&args_iter.next(), "db.query")?;
        let sql = extract_sql_string(&args_iter.next(), "db.query")?;
        let params = extract_params(&args_iter.next())?;

        match pool {
            DbPool::Any(pool) => any_query(&lua, &pool, &sql, &params).await,
            DbPool::Postgres(pool) => postgres_query(&lua, &pool, &sql, &params).await,
        }
    })?;
    db_table.set("query", query_fn)?;

    let execute_fn = lua.create_async_function(|lua, args: mlua::MultiValue| async move {
        let mut args_iter = args.into_iter();

        let pool = extract_db_pool(&args_iter.next(), "db.execute")?;
        let sql = extract_sql_string(&args_iter.next(), "db.execute")?;
        let params = extract_params(&args_iter.next())?;

        let rows_affected = match pool {
            DbPool::Any(pool) => any_execute(&pool, &sql, &params).await?,
            DbPool::Postgres(pool) => postgres_execute(&pool, &sql, &params).await?,
        };

        let tbl = lua.create_table()?;
        tbl.set("rows_affected", rows_affected as i64)?;
        Ok(Value::Table(tbl))
    })?;
    db_table.set("execute", execute_fn)?;

    let close_fn = lua.create_async_function(|_, args: mlua::MultiValue| async move {
        let mut args_iter = args.into_iter();
        match extract_db_pool(&args_iter.next(), "db.close")? {
            DbPool::Any(pool) => pool.close().await,
            DbPool::Postgres(pool) => pool.close().await,
        }
        Ok(())
    })?;
    db_table.set("close", close_fn)?;

    lua.globals().set("db", db_table)?;
    Ok(())
}

fn is_postgres_url(url: &str) -> bool {
    url.starts_with("postgres://") || url.starts_with("postgresql://")
}

fn extract_db_pool(val: &Option<Value>, fn_name: &str) -> mlua::Result<DbPool> {
    match val {
        Some(Value::UserData(ud)) => {
            let db = ud.borrow::<DbPool>().map_err(|_| {
                mlua::Error::runtime(format!("{fn_name}: first argument must be a db connection"))
            })?;
            Ok(db.clone())
        }
        _ => Err(mlua::Error::runtime(format!(
            "{fn_name}: first argument must be a db connection"
        ))),
    }
}

fn extract_sql_string(val: &Option<Value>, fn_name: &str) -> mlua::Result<String> {
    match val {
        Some(Value::String(s)) => Ok(s.to_str()?.to_string()),
        _ => Err(mlua::Error::runtime(format!(
            "{fn_name}: second argument must be a SQL string"
        ))),
    }
}

#[derive(Clone)]
enum DbParam {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

fn extract_params(val: &Option<Value>) -> mlua::Result<Vec<DbParam>> {
    match val {
        Some(Value::Table(t)) => {
            let mut params = Vec::new();
            let len = t.len()?;
            for i in 1..=len {
                let v: Value = t.get(i)?;
                let param = match v {
                    Value::Nil => DbParam::Null,
                    Value::Boolean(b) => DbParam::Bool(b),
                    Value::Integer(n) => DbParam::Int(n),
                    Value::Number(f) => DbParam::Float(f),
                    Value::String(s) => DbParam::Text(s.to_str()?.to_string()),
                    _ => {
                        return Err(mlua::Error::runtime(format!(
                            "db: unsupported parameter type: {}",
                            v.type_name()
                        )));
                    }
                };
                params.push(param);
            }
            Ok(params)
        }
        Some(Value::Nil) | None => Ok(Vec::new()),
        _ => Err(mlua::Error::runtime(
            "db: params must be a table (array) or nil",
        )),
    }
}

async fn any_query(
    lua: &Lua,
    pool: &AnyPool,
    sql: &str,
    params: &[DbParam],
) -> mlua::Result<Value> {
    let mut query = sqlx::query(sql);
    for p in params {
        query = bind_any_param(query, p);
    }

    let rows: Vec<AnyRow> = query
        .fetch_all(pool)
        .await
        .map_err(|e| mlua::Error::runtime(format!("db.query: {e}")))?;

    let result = lua.create_table()?;
    for (i, row) in rows.iter().enumerate() {
        let row_table = any_row_to_lua_table(lua, row)?;
        result.set(i + 1, row_table)?;
    }
    Ok(Value::Table(result))
}

async fn postgres_query(
    lua: &Lua,
    pool: &PgPool,
    sql: &str,
    params: &[DbParam],
) -> mlua::Result<Value> {
    let mut query = sqlx::query(sql);
    for p in params {
        query = bind_postgres_param(query, p);
    }

    let rows: Vec<PgRow> = query
        .fetch_all(pool)
        .await
        .map_err(|e| mlua::Error::runtime(format!("db.query: {e}")))?;

    let result = lua.create_table()?;
    for (i, row) in rows.iter().enumerate() {
        let row_table = postgres_row_to_lua_table(lua, row)?;
        result.set(i + 1, row_table)?;
    }
    Ok(Value::Table(result))
}

async fn any_execute(pool: &AnyPool, sql: &str, params: &[DbParam]) -> mlua::Result<u64> {
    let mut query = sqlx::query(sql);
    for p in params {
        query = bind_any_param(query, p);
    }

    let result = query
        .execute(pool)
        .await
        .map_err(|e| mlua::Error::runtime(format!("db.execute: {e}")))?;
    Ok(result.rows_affected())
}

async fn postgres_execute(pool: &PgPool, sql: &str, params: &[DbParam]) -> mlua::Result<u64> {
    let mut query = sqlx::query(sql);
    for p in params {
        query = bind_postgres_param(query, p);
    }

    let result = query
        .execute(pool)
        .await
        .map_err(|e| mlua::Error::runtime(format!("db.execute: {e}")))?;
    Ok(result.rows_affected())
}

fn bind_any_param<'q>(
    query: sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>>,
    param: &'q DbParam,
) -> sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>> {
    match param {
        DbParam::Null => query.bind(None::<String>),
        DbParam::Bool(b) => query.bind(*b),
        DbParam::Int(n) => query.bind(*n),
        DbParam::Float(f) => query.bind(*f),
        DbParam::Text(s) => query.bind(s.as_str()),
    }
}

fn bind_postgres_param<'q>(
    query: sqlx::query::Query<'q, sqlx::Postgres, PgArguments>,
    param: &'q DbParam,
) -> sqlx::query::Query<'q, sqlx::Postgres, PgArguments> {
    match param {
        DbParam::Null => query.bind(None::<String>),
        DbParam::Bool(b) => query.bind(*b),
        DbParam::Int(n) => query.bind(*n),
        DbParam::Float(f) => query.bind(*f),
        DbParam::Text(s) => query.bind(s.as_str()),
    }
}

fn any_row_to_lua_table(lua: &Lua, row: &AnyRow) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for col in row.columns() {
        let name = col.name();
        let val: Value = any_column_to_lua_value(lua, row, col)?;
        table.set(name.to_string(), val)?;
    }
    Ok(table)
}

fn any_column_to_lua_value<C: Column>(lua: &Lua, row: &AnyRow, col: &C) -> mlua::Result<Value> {
    let ordinal = col.ordinal();
    let type_info = col.type_info();
    let type_name = type_info.to_string();
    let type_name = type_name.to_uppercase();

    if row
        .try_get_raw(ordinal)
        .map(|v| v.is_null())
        .unwrap_or(true)
    {
        return Ok(Value::Nil);
    }

    match type_name.as_str() {
        "BOOLEAN" | "BOOL" => {
            let v: bool = row
                .try_get(ordinal)
                .map_err(|e| mlua::Error::runtime(format!("db: column read error: {e}")))?;
            Ok(Value::Boolean(v))
        }
        "INTEGER" | "INT" | "INT4" | "INT8" | "BIGINT" | "SMALLINT" | "TINYINT" | "INT2" => {
            let v: i64 = row
                .try_get(ordinal)
                .map_err(|e| mlua::Error::runtime(format!("db: column read error: {e}")))?;
            Ok(Value::Integer(v))
        }
        "REAL" | "FLOAT" | "FLOAT4" | "FLOAT8" | "DOUBLE" | "DOUBLE PRECISION" | "NUMERIC" => {
            let v: f64 = row
                .try_get(ordinal)
                .map_err(|e| mlua::Error::runtime(format!("db: column read error: {e}")))?;
            Ok(Value::Number(v))
        }
        _ => {
            let v: String = row
                .try_get(ordinal)
                .map_err(|e| mlua::Error::runtime(format!("db: column read error: {e}")))?;
            Ok(Value::String(lua.create_string(&v)?))
        }
    }
}

fn postgres_row_to_lua_table(lua: &Lua, row: &PgRow) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for col in row.columns() {
        let name = col.name();
        let val: Value = postgres_column_to_lua_value(lua, row, col)?;
        table.set(name.to_string(), val)?;
    }
    Ok(table)
}

fn postgres_column_to_lua_value<C: Column>(lua: &Lua, row: &PgRow, col: &C) -> mlua::Result<Value> {
    let ordinal = col.ordinal();
    let type_info = col.type_info();
    let type_name = type_info.to_string();
    let type_name = type_name.to_uppercase();

    if row
        .try_get_raw(ordinal)
        .map(|v| v.is_null())
        .unwrap_or(true)
    {
        return Ok(Value::Nil);
    }

    match type_name.as_str() {
        "BOOL" | "BOOLEAN" => {
            let v: bool = read_column(row, ordinal)?;
            Ok(Value::Boolean(v))
        }
        "INT2" | "SMALLINT" => {
            let v: i16 = read_column(row, ordinal)?;
            Ok(Value::Integer(i64::from(v)))
        }
        "INT4" | "INTEGER" | "INT" => {
            let v: i32 = read_column(row, ordinal)?;
            Ok(Value::Integer(i64::from(v)))
        }
        "INT8" | "BIGINT" => {
            let v: i64 = read_column(row, ordinal)?;
            Ok(Value::Integer(v))
        }
        "FLOAT4" | "REAL" => {
            let v: f32 = read_column(row, ordinal)?;
            Ok(Value::Number(f64::from(v)))
        }
        "FLOAT8" | "DOUBLE" | "DOUBLE PRECISION" => {
            let v: f64 = read_column(row, ordinal)?;
            Ok(Value::Number(v))
        }
        "NUMERIC" | "DECIMAL" => {
            let v: BigDecimal = read_column(row, ordinal)?;
            Ok(Value::String(lua.create_string(v.to_string())?))
        }
        "JSON" | "JSONB" => {
            let v: sqlx::types::Json<serde_json::Value> = read_column(row, ordinal)?;
            json_value_to_lua(lua, &v.0)
        }
        _ => {
            let v: String = read_column(row, ordinal)?;
            Ok(Value::String(lua.create_string(&v)?))
        }
    }
}

fn read_column<'r, T>(row: &'r PgRow, ordinal: usize) -> mlua::Result<T>
where
    T: sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    row.try_get(ordinal)
        .map_err(|e| mlua::Error::runtime(format!("db: column read error: {e}")))
}
