//! `Manifest.lua` parser. Reads a Lua return-table declaring extension
//! binaries + Lua libraries to install.
//!
//! Manifest.lua is evaluated in a heavily restricted mlua VM:
//! - StdLib loaded: `BASE` (always, by mlua) + `STRING` + `TABLE` + `MATH`.
//! - StdLib withheld: `IO`, `OS`, `COROUTINE`, `UTF8`, `DEBUG`, `PACKAGE`.
//! - Code-loading primitives nilled: `load`, `loadfile`, `loadstring`,
//!   `dofile`, `require`.
//!
//! A malicious or accidental `os.execute("...")` therefore fails with
//! "attempt to index a nil value (global 'os')" before any side effect.

use std::collections::HashMap;
use std::path::Path;

use mlua::{Lua, LuaOptions, LuaSerdeExt, StdLib, Value};
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("manifest {path}: lua error: {source}")]
    Lua {
        path: String,
        #[source]
        source: mlua::Error,
    },

    #[error("manifest {path}: top-level value is {got}, expected a table")]
    NotATable { path: String, got: String },

    #[error("manifest {path}: failed to decode: {source}")]
    Decode {
        path: String,
        #[source]
        source: mlua::Error,
    },
}

/// Parsed `Manifest.lua` contents.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Self-version pin for the assay binary (advisory). Install warns
    /// — does not abort — if the running binary differs.
    pub assay: Option<String>,

    /// Compiled separate binaries (e.g. `assay-engine`).
    #[serde(default)]
    pub extensions: Vec<Extension>,

    /// Lua libraries shipped as tarballs (e.g. `sysops`).
    #[serde(default)]
    pub libs: Vec<Lib>,
}

/// One declared extension binary. Per-arch sha256 because the binary
/// differs per target triple.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extension {
    pub name: String,
    pub version: String,
    /// Map from arch token (e.g. `x86_64`, `aarch64`) to sha256 hex.
    pub sha256: HashMap<String, String>,
    /// Optional full-URL override. If absent, install resolves to the
    /// assay-release URL convention.
    #[serde(default)]
    pub source: Option<String>,
}

/// One declared Lua library. Single sha256 because libs are arch-neutral
/// (pure Lua).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lib {
    pub name: String,
    pub version: String,
    pub sha256: String,
    #[serde(default)]
    pub source: Option<String>,
}

/// Parse a `Manifest.lua` source string.
///
/// `path` is used only for error messages; the source is evaluated as an
/// in-memory chunk and the parser performs no filesystem access.
pub fn parse(source: &str, path: impl AsRef<Path>) -> Result<Manifest, ManifestError> {
    let path_str = path.as_ref().display().to_string();

    let lua = sandboxed_vm().map_err(|e| ManifestError::Lua {
        path: path_str.clone(),
        source: e,
    })?;

    let chunk = lua.load(source).set_name(path_str.clone());
    let value: Value = chunk.eval().map_err(|e| ManifestError::Lua {
        path: path_str.clone(),
        source: e,
    })?;

    if !matches!(value, Value::Table(_)) {
        return Err(ManifestError::NotATable {
            path: path_str,
            got: value.type_name().to_string(),
        });
    }

    lua.from_value::<Manifest>(value)
        .map_err(|e| ManifestError::Decode {
            path: path_str,
            source: e,
        })
}

fn sandboxed_vm() -> mlua::Result<Lua> {
    let libs = StdLib::STRING | StdLib::TABLE | StdLib::MATH;
    let lua = Lua::new_with(libs, LuaOptions::default())?;

    // BASE is always loaded by mlua. Strip the primitives that allow
    // loading arbitrary Lua code from strings or disk.
    let globals = lua.globals();
    for name in ["load", "loadfile", "loadstring", "dofile", "require"] {
        globals.set(name, mlua::Nil)?;
    }

    Ok(lua)
}
