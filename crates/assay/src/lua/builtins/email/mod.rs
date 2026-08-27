//! `smtp_probe` — recipient probing over the SMTP envelope.

mod probe;
mod reply;

use mlua::{Lua, Table};
use probe::{Params, Probe};
use std::time::Duration;

const DEFAULT_PORT: u16 = 25;
const DEFAULT_CONNECT_TIMEOUT_MS: u64 = 8_000;
const DEFAULT_OP_TIMEOUT_MS: u64 = 8_000;
const DEFAULT_GREYLIST_DELAY_MS: u64 = 3_000;

fn ms(opts: &Table, key: &str, default: u64) -> mlua::Result<Duration> {
    Ok(Duration::from_millis(
        opts.get::<Option<u64>>(key)?.unwrap_or(default),
    ))
}

fn hosts_of(opts: &Table) -> mlua::Result<Vec<String>> {
    let list = opts
        .get::<Option<Table>>("mx")?
        .ok_or_else(|| mlua::Error::runtime("smtp_probe.check: mx list is required"))?;
    let mut hosts = Vec::new();
    for host in list.sequence_values::<String>() {
        hosts.push(host?);
    }
    if hosts.is_empty() {
        return Err(mlua::Error::runtime("smtp_probe.check: mx list is empty"));
    }
    Ok(hosts)
}

fn params_of(opts: &Table) -> mlua::Result<Params> {
    let email: String = opts.get("email")?;
    let domain = email
        .rsplit_once('@')
        .map(|(_, d)| d.to_lowercase())
        .ok_or_else(|| mlua::Error::runtime("smtp_probe.check: email must contain '@'"))?;

    let from: String = opts.get("from")?;
    let from_domain = from
        .rsplit_once('@')
        .map(|(_, d)| d.to_string())
        .ok_or_else(|| {
            mlua::Error::runtime(
                "smtp_probe.check: from must be a full address — servers judge the envelope sender",
            )
        })?;

    Ok(Params {
        email,
        domain,
        hosts: hosts_of(opts)?,
        from,
        helo: opts.get::<Option<String>>("helo")?.unwrap_or(from_domain),
        port: opts.get::<Option<u16>>("port")?.unwrap_or(DEFAULT_PORT),
        connect: ms(opts, "connect_timeout_ms", DEFAULT_CONNECT_TIMEOUT_MS)?,
        op: ms(opts, "op_timeout_ms", DEFAULT_OP_TIMEOUT_MS)?,
        catch_all_check: opts.get::<Option<bool>>("catch_all")?.unwrap_or(true),
        greylist_delay: ms(opts, "greylist_delay_ms", DEFAULT_GREYLIST_DELAY_MS)?,
    })
}

fn to_table(lua: &Lua, p: Probe) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set("host_exists", p.host_exists)?;
    t.set("mx_host", p.mx_host)?;
    t.set("catch_all", p.catch_all)?;
    t.set("deliverable", p.deliverable)?;
    t.set("full_inbox", p.full_inbox)?;
    t.set("disabled", p.disabled)?;
    t.set("blocked", p.blocked)?;
    t.set("greylisted", p.greylisted)?;
    t.set("code", p.code)?;
    t.set("reason", p.reason)?;
    t.set("detail", p.detail)?;
    t.set("stage", p.stage)?;
    Ok(t)
}

pub fn register_smtp_probe(lua: &Lua) -> mlua::Result<()> {
    let table = lua.create_table()?;
    let check = lua.create_async_function(|lua, opts: Table| async move {
        let params = params_of(&opts)?;
        to_table(&lua, probe::run(params).await)
    })?;
    table.set("check", check)?;
    lua.globals().set("smtp_probe", table)?;
    Ok(())
}
