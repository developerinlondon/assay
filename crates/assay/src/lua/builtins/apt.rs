//! apt builtin — wraps apt-get/dpkg-query for use by the package manager.
//!
//! Functions are simple shell wrappers; the parsers (dpkg-query and
//! `apt list --upgradable -a` output) are pure-string transforms exposed via
//! `apt._parse_dpkg_lines` / `apt._parse_upgradable_lines` for unit testing.
//!
//! Operations that mutate system state (install, remove, update, add_source
//! against /etc paths) require root and are tested manually against a
//! throwaway nspawn machine in knowhere plan 13's smoke.

use mlua::{Lua, Table};

pub fn register_apt(lua: &Lua) -> mlua::Result<()> {
    let t = lua.create_table()?;

    // ── parsers (pure, exported for tests) ────────────────────────────────

    t.set("_parse_dpkg_lines", lua.create_function(parse_dpkg_lines)?)?;
    t.set(
        "_parse_upgradable_lines",
        lua.create_function(parse_upgradable_lines)?,
    )?;

    // ── query / list_installed / list_upgradable ──────────────────────────

    t.set(
        "query",
        lua.create_async_function(|lua, name: String| async move {
            let (status, stdout, _stderr) = run_command(
                "dpkg-query",
                &["-W", "-f=${Package}\t${Version}\t${Status}\n", &name],
            )
            .await?;
            let result = lua.create_table()?;
            if status != 0 {
                result.set("installed", false)?;
                result.set("version", mlua::Value::Nil)?;
                return Ok(result);
            }
            let parsed = parse_dpkg_lines(&lua, stdout)?;
            if let Ok(entry) = parsed.get::<Table>(name.clone()) {
                let installed: bool = entry.get("installed")?;
                let version: String = entry.get("version")?;
                result.set("installed", installed)?;
                if installed {
                    result.set("version", version)?;
                } else {
                    result.set("version", mlua::Value::Nil)?;
                }
            } else {
                result.set("installed", false)?;
                result.set("version", mlua::Value::Nil)?;
            }
            Ok(result)
        })?,
    )?;

    t.set(
        "list_installed",
        lua.create_async_function(|lua, ()| async move {
            let (status, stdout, stderr) = run_command(
                "dpkg-query",
                &["-W", "-f=${Package}\t${Version}\t${Status}\n"],
            )
            .await?;
            if status != 0 {
                return Err(mlua::Error::runtime(format!(
                    "apt.list_installed: dpkg-query exit {status}: {stderr}"
                )));
            }
            parse_dpkg_lines(&lua, stdout)
        })?,
    )?;

    t.set(
        "list_upgradable",
        lua.create_async_function(|lua, ()| async move {
            let (_status, stdout, _stderr) =
                run_command("apt", &["list", "--upgradable", "-a"]).await?;
            parse_upgradable_lines(&lua, stdout)
        })?,
    )?;

    // ── source management ─────────────────────────────────────────────────

    t.set("add_source", lua.create_function(add_source)?)?;

    // ── apt-get wrappers ──────────────────────────────────────────────────

    t.set(
        "update",
        lua.create_async_function(|lua, ()| async move {
            let (status, stdout, stderr) = run_command("apt-get", &["update"]).await?;
            let r = lua.create_table()?;
            r.set("status", status as i64)?;
            r.set("stdout", stdout)?;
            r.set("stderr", stderr)?;
            Ok(r)
        })?,
    )?;

    t.set(
        "install",
        lua.create_async_function(|lua, opts: Table| async move {
            let names_table: Table = opts.get("names")?;
            let only_upgrade: bool = opts.get::<Option<bool>>("only_upgrade")?.unwrap_or(false);
            let mut names: Vec<String> = Vec::new();
            for v in names_table.sequence_values::<String>() {
                names.push(v?);
            }
            if names.is_empty() {
                return Err(mlua::Error::runtime("apt.install: names array is empty"));
            }
            let mut args: Vec<String> = vec![
                "install".into(),
                "-y".into(),
                "--no-install-recommends".into(),
            ];
            if only_upgrade {
                args.push("--only-upgrade".into());
            }
            for n in &names {
                args.push(n.clone());
            }
            let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let (status, stdout, stderr) = run_command("apt-get", &args_refs).await?;
            let r = lua.create_table()?;
            r.set("status", status as i64)?;
            r.set("stdout", stdout)?;
            r.set("stderr", stderr)?;
            Ok(r)
        })?,
    )?;

    t.set(
        "remove",
        lua.create_async_function(|lua, opts: Table| async move {
            let names_table: Table = opts.get("names")?;
            let mut names: Vec<String> = Vec::new();
            for v in names_table.sequence_values::<String>() {
                names.push(v?);
            }
            if names.is_empty() {
                return Err(mlua::Error::runtime("apt.remove: names array is empty"));
            }
            let mut args: Vec<String> = vec!["remove".into(), "-y".into()];
            for n in &names {
                args.push(n.clone());
            }
            let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
            let (status, stdout, stderr) = run_command("apt-get", &args_refs).await?;
            let r = lua.create_table()?;
            r.set("status", status as i64)?;
            r.set("stdout", stdout)?;
            r.set("stderr", stderr)?;
            Ok(r)
        })?,
    )?;

    lua.globals().set("apt", t)?;
    Ok(())
}

// ── Parsers ──────────────────────────────────────────────────────────────────

/// Parse `dpkg-query -W -f='${Package}\t${Version}\t${Status}\n'` output.
/// Returns a table keyed by package name with { version=string, installed=bool }.
fn parse_dpkg_lines(lua: &Lua, input: String) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.len() < 3 {
            continue;
        }
        let name = parts[0].trim();
        let version = parts[1].trim();
        let status = parts[2].trim();
        // dpkg Status format: "<wanted> <flag> <status>"; we want the third field == "installed".
        let installed = status
            .split_whitespace()
            .nth(2)
            .map(|s| s == "installed")
            .unwrap_or(false);

        let entry = lua.create_table()?;
        entry.set("version", version)?;
        entry.set("installed", installed)?;
        out.set(name, entry)?;
    }
    Ok(out)
}

/// Parse `apt list --upgradable -a` output. Returns array of
/// { name=string, current=string, candidate=string, suite=string }.
fn parse_upgradable_lines(lua: &Lua, input: String) -> mlua::Result<Table> {
    let out = lua.create_table()?;
    let mut idx = 1usize;
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("Listing...")
            || line.starts_with("WARNING:")
            || line.starts_with("N:")
        {
            continue;
        }
        // Format: "name/suite candidate arch [upgradable from: current]"
        let upgradable_marker = "[upgradable from:";
        if !line.contains(upgradable_marker) {
            continue;
        }
        let (head, tail) = match line.split_once(upgradable_marker) {
            Some(p) => p,
            None => continue,
        };
        let head_parts: Vec<&str> = head.split_whitespace().collect();
        if head_parts.len() < 2 {
            continue;
        }
        let name_suite = head_parts[0];
        let candidate = head_parts[1];
        let (name, suite) = match name_suite.split_once('/') {
            Some((n, s)) => (n, s),
            None => (name_suite, ""),
        };
        let current = tail.trim_end_matches(']').trim();

        let entry = lua.create_table()?;
        entry.set("name", name)?;
        entry.set("current", current)?;
        entry.set("candidate", candidate)?;
        entry.set("suite", suite)?;
        out.set(idx, entry)?;
        idx += 1;
    }
    Ok(out)
}

// ── Shell helper ─────────────────────────────────────────────────────────────

async fn run_command(program: &str, args: &[&str]) -> mlua::Result<(i32, String, String)> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args);
    cmd.env("DEBIAN_FRONTEND", "noninteractive");
    cmd.env("LC_ALL", "C");
    cmd.env_remove("APT_CONFIG");
    cmd.env_remove("APT_LISTCHANGES_FRONTEND");
    cmd.env_remove("DEBCONF_NONINTERACTIVE_SEEN");
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let output = cmd
        .output()
        .await
        .map_err(|e| mlua::Error::runtime(format!("apt: spawn {program}: {e}")))?;
    Ok((
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    ))
}

// ── source management ────────────────────────────────────────────────────────

/// apt.add_source({ id, source_list, key_path, _sources_dir?, _keyrings_dir? }) ->
///   { changed = bool, list_path = string, key_path = string }
///
/// Idempotent. `_sources_dir` and `_keyrings_dir` overrides exist for tests so
/// we don't write into /etc during cargo test. In production they default to
/// /etc/apt/sources.list.d and /usr/share/keyrings.
fn add_source(lua: &Lua, opts: Table) -> mlua::Result<Table> {
    let id: String = opts.get("id")?;
    let source_list: String = opts.get("source_list")?;
    let key_path: String = opts.get("key_path")?;
    let sources_dir: String = opts
        .get::<Option<String>>("_sources_dir")?
        .unwrap_or_else(|| "/etc/apt/sources.list.d".into());
    let keyrings_dir: String = opts
        .get::<Option<String>>("_keyrings_dir")?
        .unwrap_or_else(|| "/usr/share/keyrings".into());

    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(mlua::Error::runtime(format!(
            "apt.add_source: id {id:?} must match [a-z0-9-]+"
        )));
    }

    if source_list.contains('\n') || source_list.contains('\r') {
        return Err(mlua::Error::runtime(
            "apt.add_source: source_list must be a single line (no \\n or \\r)",
        ));
    }

    std::fs::create_dir_all(&sources_dir)
        .map_err(|e| mlua::Error::runtime(format!("apt.add_source: mkdir {sources_dir:?}: {e}")))?;
    std::fs::create_dir_all(&keyrings_dir).map_err(|e| {
        mlua::Error::runtime(format!("apt.add_source: mkdir {keyrings_dir:?}: {e}"))
    })?;

    let list_dst = format!("{sources_dir}/{id}.list");
    let key_dst = format!("{keyrings_dir}/{id}.gpg");

    let mut changed = false;

    // Write key BEFORE list: a stranded keyring is harmless, but a stranded
    // .list referring to a missing keyring breaks `apt-get update`.
    //
    // Idempotent key copy: read key_path, compare to dst, copy if different.
    let want_key = std::fs::read(&key_path)
        .map_err(|e| mlua::Error::runtime(format!("apt.add_source: read key {key_path:?}: {e}")))?;
    let cur_key = std::fs::read(&key_dst).ok();
    if cur_key.as_deref() != Some(&want_key) {
        let tmp = format!("{key_dst}.tmp.{}", std::process::id());
        std::fs::write(&tmp, &want_key)
            .map_err(|e| mlua::Error::runtime(format!("apt.add_source: write key {tmp:?}: {e}")))?;
        std::fs::rename(&tmp, &key_dst)
            .map_err(|e| mlua::Error::runtime(format!("apt.add_source: rename key: {e}")))?;
        changed = true;
    }

    // Idempotent list write: only write if missing or content differs.
    let want_list = format!("{}\n", source_list.trim_end());
    let cur_list = std::fs::read_to_string(&list_dst).ok();
    if cur_list.as_deref() != Some(&want_list) {
        let tmp = format!("{list_dst}.tmp.{}", std::process::id());
        std::fs::write(&tmp, &want_list)
            .map_err(|e| mlua::Error::runtime(format!("apt.add_source: write {tmp:?}: {e}")))?;
        std::fs::rename(&tmp, &list_dst)
            .map_err(|e| mlua::Error::runtime(format!("apt.add_source: rename: {e}")))?;
        changed = true;
    }

    let result = lua.create_table()?;
    result.set("changed", changed)?;
    result.set("list_path", list_dst)?;
    result.set("key_path", key_dst)?;
    Ok(result)
}
