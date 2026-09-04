//! SQLite cell to Postgres bind parameter.
//!
//! SQLite has five storage classes and no column typing, so what a cell
//! holds is a property of the row rather than the schema. Postgres binds
//! are typed, so every cell is read as what SQLite actually stored and
//! then coerced to what the target column declares.

use anyhow::{Result, anyhow, bail};
use sqlx::sqlite::SqliteRow;
use sqlx::{Decode, Row, Sqlite, TypeInfo, ValueRef};

/// One cell as SQLite stored it.
#[derive(Debug, Clone)]
pub enum Cell {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// One bind parameter, already shaped for its Postgres column.
#[derive(Debug, Clone)]
pub enum Bound {
    Text(Option<String>),
    Bool(Option<bool>),
    I32(Option<i32>),
    I64(Option<i64>),
    F64(Option<f64>),
    Bytes(Option<Vec<u8>>),
    TextArray(Option<Vec<String>>),
}

pub fn read_cell(row: &SqliteRow, idx: usize) -> Result<Cell> {
    let raw = row.try_get_raw(idx)?;
    if raw.is_null() {
        return Ok(Cell::Null);
    }
    let kind = raw.type_info().name().to_owned();
    let decode_err = |e: sqlx::error::BoxDynError| anyhow!("decode sqlite {kind} at column {idx}: {e}");
    match kind.as_str() {
        "TEXT" => Ok(Cell::Text(
            <String as Decode<Sqlite>>::decode(raw).map_err(decode_err)?,
        )),
        "INTEGER" => Ok(Cell::Int(
            <i64 as Decode<Sqlite>>::decode(raw).map_err(decode_err)?,
        )),
        "REAL" => Ok(Cell::Real(
            <f64 as Decode<Sqlite>>::decode(raw).map_err(decode_err)?,
        )),
        "BLOB" => Ok(Cell::Blob(
            <Vec<u8> as Decode<Sqlite>>::decode(raw).map_err(decode_err)?,
        )),
        other => bail!("unsupported sqlite storage class {other} at column {idx}"),
    }
}

/// The SQL cast appended to a placeholder for `udt`. Postgres reports
/// array types with a leading underscore.
pub fn pg_cast(udt: &str) -> String {
    match udt.strip_prefix('_') {
        Some(elem) => format!("{elem}[]"),
        None => udt.to_string(),
    }
}

/// Shape `cell` for a Postgres column of type `udt`. `column` names the
/// cell in the error when the stored value cannot be carried across.
pub fn coerce(cell: Cell, udt: &str, column: &str) -> Result<Bound> {
    match udt {
        "text" | "varchar" | "bpchar" | "name" | "json" | "jsonb" | "uuid" => {
            Ok(Bound::Text(as_text(cell, udt, column)?))
        }
        "bool" => Ok(Bound::Bool(as_bool(cell, column)?)),
        "int2" | "int4" => Ok(Bound::I32(as_i32(cell, column)?)),
        "int8" => Ok(Bound::I64(as_i64(cell, column)?)),
        "float4" | "float8" | "numeric" => Ok(Bound::F64(as_f64(cell, column)?)),
        "bytea" => Ok(Bound::Bytes(as_bytes(cell, column)?)),
        "_text" | "_varchar" => Ok(Bound::TextArray(as_text_array(cell, column)?)),
        other => bail!("column {column}: no conversion to postgres type {other}"),
    }
}

fn as_text(cell: Cell, udt: &str, column: &str) -> Result<Option<String>> {
    match cell {
        Cell::Null => Ok(None),
        Cell::Text(s) => Ok(Some(s)),
        Cell::Int(i) => Ok(Some(i.to_string())),
        Cell::Real(f) => Ok(Some(f.to_string())),
        Cell::Blob(_) => bail!("column {column}: sqlite holds a blob, target is {udt}"),
    }
}

fn as_bool(cell: Cell, column: &str) -> Result<Option<bool>> {
    match cell {
        Cell::Null => Ok(None),
        Cell::Int(i) => Ok(Some(i != 0)),
        Cell::Text(s) => match s.as_str() {
            "1" | "t" | "true" | "TRUE" => Ok(Some(true)),
            "0" | "f" | "false" | "FALSE" => Ok(Some(false)),
            other => bail!("column {column}: {other:?} is not a boolean"),
        },
        other => bail!("column {column}: {other:?} is not a boolean"),
    }
}

fn as_i64(cell: Cell, column: &str) -> Result<Option<i64>> {
    match cell {
        Cell::Null => Ok(None),
        Cell::Int(i) => Ok(Some(i)),
        Cell::Real(f) if f.fract() == 0.0 => Ok(Some(f as i64)),
        Cell::Text(s) => s
            .parse::<i64>()
            .map(Some)
            .map_err(|_| anyhow!("column {column}: {s:?} is not an integer")),
        other => bail!("column {column}: {other:?} is not an integer"),
    }
}

fn as_i32(cell: Cell, column: &str) -> Result<Option<i32>> {
    let wide = as_i64(cell, column)?;
    wide.map(|v| {
        i32::try_from(v).map_err(|_| anyhow!("column {column}: {v} does not fit a 32-bit integer"))
    })
    .transpose()
}

fn as_f64(cell: Cell, column: &str) -> Result<Option<f64>> {
    match cell {
        Cell::Null => Ok(None),
        Cell::Real(f) => Ok(Some(f)),
        Cell::Int(i) => Ok(Some(i as f64)),
        Cell::Text(s) => s
            .parse::<f64>()
            .map(Some)
            .map_err(|_| anyhow!("column {column}: {s:?} is not a number")),
        other => bail!("column {column}: {other:?} is not a number"),
    }
}

fn as_bytes(cell: Cell, column: &str) -> Result<Option<Vec<u8>>> {
    match cell {
        Cell::Null => Ok(None),
        Cell::Blob(b) => Ok(Some(b)),
        Cell::Text(s) => Ok(Some(s.into_bytes())),
        other => bail!("column {column}: {other:?} is not bytes"),
    }
}

/// SQLite keeps the Postgres `TEXT[]` columns as a JSON array in a TEXT
/// cell, which is what the SQLite engine schema writes.
fn as_text_array(cell: Cell, column: &str) -> Result<Option<Vec<String>>> {
    match cell {
        Cell::Null => Ok(None),
        Cell::Text(s) => serde_json::from_str::<Vec<String>>(&s)
            .map(Some)
            .map_err(|e| anyhow!("column {column}: {s:?} is not a JSON array of strings: {e}")),
        other => bail!("column {column}: {other:?} is not an array"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn array_columns_cast_with_brackets() {
        assert_eq!(pg_cast("_text"), "text[]");
        assert_eq!(pg_cast("jsonb"), "jsonb");
    }

    #[test]
    fn sqlite_integers_carry_postgres_booleans() {
        let Bound::Bool(v) = coerce(Cell::Int(1), "bool", "enabled").unwrap() else {
            panic!("expected a bool");
        };
        assert_eq!(v, Some(true));
    }

    #[test]
    fn sqlite_json_text_becomes_a_postgres_array() {
        let cell = Cell::Text(r#"["main","other"]"#.to_string());
        let Bound::TextArray(v) = coerce(cell, "_text", "namespaces").unwrap() else {
            panic!("expected an array");
        };
        assert_eq!(v, Some(vec!["main".to_string(), "other".to_string()]));
    }

    #[test]
    fn a_blob_never_silently_becomes_text() {
        let err = coerce(Cell::Blob(vec![1, 2]), "text", "id").unwrap_err();
        assert!(err.to_string().contains("blob"), "{err}");
    }

    #[test]
    fn an_out_of_range_integer_is_refused_rather_than_truncated() {
        let err = coerce(Cell::Int(i64::from(i32::MAX) + 1), "int4", "seq").unwrap_err();
        assert!(err.to_string().contains("32-bit"), "{err}");
    }

    #[test]
    fn an_unknown_postgres_type_is_refused() {
        let err = coerce(Cell::Null, "inet", "addr").unwrap_err();
        assert!(err.to_string().contains("no conversion"), "{err}");
    }
}
