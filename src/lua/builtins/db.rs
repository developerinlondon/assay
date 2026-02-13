use mlua::{Lua, Table, UserData, Value};
use sqlx::any::AnyRow;
use sqlx::{AnyPool, Column, Row, ValueRef};
use std::sync::Arc;

struct DbPool(Arc<AnyPool>);
impl UserData for DbPool {}

pub fn register_db(lua: &Lua) -> mlua::Result<()> {
    sqlx::any::install_default_drivers();

    let db_table = lua.create_table()?;

    let connect_fn = lua.create_async_function(|lua, url: String| async move {
        let pool = sqlx::any::AnyPoolOptions::new()
            .max_connections(if url.starts_with("sqlite:") { 1 } else { 5 })
            .connect(&url)
            .await
            .map_err(|e| mlua::Error::runtime(format!("db.connect: {e}")))?;
        lua.create_any_userdata(DbPool(Arc::new(pool)))
    })?;
    db_table.set("connect", connect_fn)?;

    let query_fn = lua.create_async_function(|lua, args: mlua::MultiValue| async move {
        let mut args_iter = args.into_iter();

        let pool = extract_db_pool(&args_iter.next(), "db.query")?;
        let sql = extract_sql_string(&args_iter.next(), "db.query")?;
        let params = extract_params(&args_iter.next())?;

        let mut query = sqlx::query(&sql);
        for p in &params {
            query = bind_param(query, p);
        }

        let rows: Vec<AnyRow> = query
            .fetch_all(&*pool)
            .await
            .map_err(|e| mlua::Error::runtime(format!("db.query: {e}")))?;

        let result = lua.create_table()?;
        for (i, row) in rows.iter().enumerate() {
            let row_table = any_row_to_lua_table(&lua, row)?;
            result.set(i + 1, row_table)?;
        }
        Ok(Value::Table(result))
    })?;
    db_table.set("query", query_fn)?;

    let execute_fn = lua.create_async_function(|lua, args: mlua::MultiValue| async move {
        let mut args_iter = args.into_iter();

        let pool = extract_db_pool(&args_iter.next(), "db.execute")?;
        let sql = extract_sql_string(&args_iter.next(), "db.execute")?;
        let params = extract_params(&args_iter.next())?;

        let mut query = sqlx::query(&sql);
        for p in &params {
            query = bind_param(query, p);
        }

        let result = query
            .execute(&*pool)
            .await
            .map_err(|e| mlua::Error::runtime(format!("db.execute: {e}")))?;

        let tbl = lua.create_table()?;
        tbl.set("rows_affected", result.rows_affected() as i64)?;
        Ok(Value::Table(tbl))
    })?;
    db_table.set("execute", execute_fn)?;

    let close_fn = lua.create_async_function(|_, args: mlua::MultiValue| async move {
        let mut args_iter = args.into_iter();
        let pool = extract_db_pool(&args_iter.next(), "db.close")?;
        pool.close().await;
        Ok(())
    })?;
    db_table.set("close", close_fn)?;

    lua.globals().set("db", db_table)?;
    Ok(())
}

fn extract_db_pool(val: &Option<Value>, fn_name: &str) -> mlua::Result<Arc<AnyPool>> {
    match val {
        Some(Value::UserData(ud)) => {
            let db = ud.borrow::<DbPool>().map_err(|_| {
                mlua::Error::runtime(format!("{fn_name}: first argument must be a db connection"))
            })?;
            Ok(db.0.clone())
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

fn bind_param<'q>(
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
