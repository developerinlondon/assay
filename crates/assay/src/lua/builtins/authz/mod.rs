use assay_authz::action::{ActionCatalogue, ActionDerivation};
use assay_authz::condition::{ConditionContext, ContextValue};
use assay_authz::engine::{CheckOptions, Engine, EngineConfig};
use assay_authz::model::{ScopeEntry, SubjectEntry};
use mlua::{Lua, Table, UserData, UserDataMethods, Value};
use serde::de::DeserializeOwned;

use super::json::{json_value_to_lua, lua_table_to_json, lua_value_to_json};

struct LuaEngine {
    engine: Engine,
}

impl UserData for LuaEngine {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method(
            "check",
            |lua, this, (subjects, action, resource, options): CheckArgs| {
                let subjects: Vec<SubjectEntry> = decode_list(&subjects, "subjects")?;
                let options = check_options(options.as_ref())?;
                let detail = this.engine.check(&subjects, &action, &resource, &options);
                let out = lua.create_table()?;
                out.set("allowed", detail.allowed)?;
                out.set("decision", detail.decision.as_str())?;
                out.set("reason", detail.reason.as_str())?;
                out.set("allowed_by_stored_grants", detail.allowed_by_stored_grants)?;
                Ok(out)
            },
        );

        methods.add_method("validate", |lua, this, statements: Table| {
            let raw = serde_json::Value::Array(list_values(&statements)?);
            match this.engine.validate(&raw) {
                Ok(statements) => {
                    let value = serde_json::to_value(statements).map_err(runtime)?;
                    Ok((json_value_to_lua(lua, &value)?, Value::Nil))
                }
                Err(error) => Ok((Value::Nil, Value::String(lua.create_string(&error)?))),
            }
        });

        methods.add_method("validate_bounds", |lua, this, bounds: Table| {
            let raw = serde_json::Value::Array(list_values(&bounds)?);
            match this.engine.validate_bounds(&raw) {
                Ok(bounds) => {
                    let value = serde_json::to_value(bounds).map_err(runtime)?;
                    Ok((json_value_to_lua(lua, &value)?, Value::Nil))
                }
                Err(error) => Ok((Value::Nil, Value::String(lua.create_string(&error)?))),
            }
        });

        methods.add_method("describe", |lua, this, ()| {
            let value = serde_json::to_value(this.engine.describe()).map_err(runtime)?;
            json_value_to_lua(lua, &value)
        });

        methods.add_method(
            "grants_for",
            |lua, this, (subjects, scope_chain): (Table, Option<Table>)| {
                let subjects: Vec<SubjectEntry> = decode_list(&subjects, "subjects")?;
                let chain: Option<Vec<ScopeEntry>> = match scope_chain.as_ref() {
                    Some(table) => Some(decode_list(table, "scope_chain")?),
                    None => None,
                };
                let grants = this.engine.grants_for(&subjects, chain.as_deref());
                let value = serde_json::to_value(grants).map_err(runtime)?;
                json_value_to_lua(lua, &value)
            },
        );
    }
}

type CheckArgs = (Table, String, String, Option<Table>);

pub fn register_authz(lua: &Lua) -> mlua::Result<()> {
    let authz = lua.create_table()?;
    authz.set(
        "engine",
        lua.create_function(|_, options: Table| build_engine(&options))?,
    )?;
    lua.globals().set("authz", authz)?;
    Ok(())
}

fn build_engine(options: &Table) -> mlua::Result<LuaEngine> {
    let actions = match optional_table(options, "actions")? {
        Some(table) => Some(
            ActionCatalogue::index(decode_list(&table, "actions")?)
                .map_err(|error| runtime(format!("authz.engine: {error}")))?,
        ),
        None => None,
    };
    let config = EngineConfig {
        grants: optional_list(options, "grants")?,
        synthesized_grants: optional_list(options, "synthesized_grants")?,
        condition_keys: optional_map(options, "condition_keys")?,
        scope_kinds: match optional_table(options, "scope_kinds")? {
            Some(table) => Some(decode_list(&table, "scope_kinds")?),
            None => None,
        },
        default_scope_chain: optional_list(options, "default_scope_chain")?,
        action_derivation: match optional_table(options, "action_derivation")? {
            Some(table) => Some(ActionDerivation(decode(
                lua_table_to_json(&table)?,
                "action_derivation",
            )?)),
            None => None,
        },
        actions,
    };
    Ok(LuaEngine {
        engine: Engine::new(config),
    })
}

fn check_options(options: Option<&Table>) -> mlua::Result<CheckOptions> {
    let Some(options) = options else {
        return Ok(CheckOptions::default());
    };
    let now: Option<String> = options.get("now")?;
    Ok(CheckOptions {
        scope_chain: match optional_table(options, "scope_chain")? {
            Some(table) => Some(decode_list(&table, "scope_chain")?),
            None => None,
        },
        context: context_values(options)?,
        source_ip: options.get("source_ip")?,
        now: now
            .as_deref()
            .map(assay_authz::parse_rfc3339)
            .transpose()
            .map_err(|error| runtime(format!("authz: `now` — {error}")))?,
        bypass: options.get::<Option<bool>>("bypass")?.unwrap_or(false),
    })
}

/// Context values are read by shape, never through the JSON encoder's
/// array-vs-object heuristic: an empty table there becomes an object, which is
/// no context value at all, and a check that cannot encode its context would
/// abort instead of deciding.
fn context_values(options: &Table) -> mlua::Result<ConditionContext> {
    let Some(table) = optional_table(options, "context")? else {
        return Ok(ConditionContext::new());
    };
    let mut context = ConditionContext::new();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let key = match key {
            Value::String(text) => text.to_str()?.to_string(),
            Value::Integer(index) => index.to_string(),
            other => {
                return Err(runtime(format!(
                    "authz: `context` key of type {}",
                    other.type_name()
                )));
            }
        };
        context.insert(key, context_value(&value)?);
    }
    Ok(context)
}

fn context_value(value: &Value) -> mlua::Result<ContextValue> {
    match value {
        Value::Table(table) => Ok(ContextValue::List(
            table
                .clone()
                .sequence_values::<Value>()
                .map(|item| Ok(scalar_text(&item?)))
                .collect::<mlua::Result<Vec<String>>>()?,
        )),
        other => Ok(ContextValue::from(lua_value_to_json(other)?)),
    }
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.to_string_lossy().to_string(),
        Value::Integer(number) => number.to_string(),
        Value::Number(number) => assay_authz::condition::format_number(*number),
        Value::Boolean(flag) => flag.to_string(),
        Value::Nil => "null".to_string(),
        other => other.type_name().to_string(),
    }
}

/// A Lua sequence, read positionally rather than through the JSON encoder's
/// array heuristic — an empty table is an empty list here, never an object.
fn list_values(table: &Table) -> mlua::Result<Vec<serde_json::Value>> {
    table
        .clone()
        .sequence_values::<Value>()
        .map(|item| lua_value_to_json(&item?))
        .collect()
}

fn optional_table(options: &Table, field: &str) -> mlua::Result<Option<Table>> {
    options.get(field)
}

fn optional_list<T: DeserializeOwned>(options: &Table, field: &str) -> mlua::Result<Vec<T>> {
    match optional_table(options, field)? {
        Some(table) => decode_list(&table, field),
        None => Ok(Vec::new()),
    }
}

fn optional_map<T: DeserializeOwned + Default>(options: &Table, field: &str) -> mlua::Result<T> {
    match optional_table(options, field)? {
        Some(table) => decode(lua_table_to_json(&table)?, field),
        None => Ok(T::default()),
    }
}

fn decode_list<T: DeserializeOwned>(table: &Table, field: &str) -> mlua::Result<Vec<T>> {
    decode(serde_json::Value::Array(list_values(table)?), field)
}

fn decode<T: DeserializeOwned>(value: serde_json::Value, field: &str) -> mlua::Result<T> {
    serde_json::from_value(value).map_err(|error| runtime(format!("authz: `{field}` — {error}")))
}

fn runtime(error: impl std::fmt::Display) -> mlua::Error {
    mlua::Error::runtime(error.to_string())
}
