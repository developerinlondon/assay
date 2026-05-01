//! machinectl wrapper for nspawn machine image management.
//!
//! Surface:
//!   machinectl.pull_tar(url, name, opts?)  → { status, stdout, stderr }
//!   machinectl.pull_raw(url, name, opts?)  → { status, stdout, stderr }
//!   machinectl.remove(name, opts?)         → { status, stdout, stderr }
//!   machinectl.clone(src, dst, opts?)      → { status, stdout, stderr }
//!
//! All subprocess invocations use argv (not /bin/sh -c) so machine names and
//! URLs can never inject shell metacharacters. Names are additionally
//! validated against the systemd-machined character set.
//!
//! These operations require root in v1; the caller is expected to have
//! arranged that (running knowhere as root, or sudo NOPASSWD wrapping).

use mlua::{Lua, Table};

/// Hard cap on a single machinectl invocation. Pull operations can take
/// minutes for large images; remove/clone are fast. We give pulls 30 minutes
/// by default, callers can override via opts.timeout.
const DEFAULT_TIMEOUT_SECS: f64 = 1800.0;

/// Validate a machine/image name. systemd-machined accepts a constrained
/// character set; we intersect with what's safe in filesystem paths and
/// systemctl unit names. Empty names are rejected.
fn validate_name(kind: &str, name: &str) -> mlua::Result<()> {
    if name.is_empty() {
        return Err(mlua::Error::runtime(format!(
            "machinectl.{kind}: name must be non-empty"
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(mlua::Error::runtime(format!(
            "machinectl.{kind}: name {name:?} must match [A-Za-z0-9._-]+"
        )));
    }
    if name.starts_with('-') {
        return Err(mlua::Error::runtime(format!(
            "machinectl.{kind}: name {name:?} must not start with '-' (would be parsed as a flag)"
        )));
    }
    Ok(())
}

/// Validate a URL: must be http(s):// and not contain whitespace or NULs.
/// machinectl pull-tar/raw enforce HTTPS by default but pull from text-listed
/// SHA256SUMS, so a URL with a newline could still confuse the parser; reject
/// any control characters defensively.
fn validate_url(kind: &str, url: &str) -> mlua::Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(mlua::Error::runtime(format!(
            "machinectl.{kind}: url must be http(s)://"
        )));
    }
    if url.chars().any(|c| c.is_control()) {
        return Err(mlua::Error::runtime(format!(
            "machinectl.{kind}: url contains control characters"
        )));
    }
    Ok(())
}

fn parse_timeout(opts: Option<&Table>, kind: &str) -> mlua::Result<std::time::Duration> {
    let secs = match opts {
        Some(t) => t
            .get::<Option<f64>>("timeout")?
            .unwrap_or(DEFAULT_TIMEOUT_SECS),
        None => DEFAULT_TIMEOUT_SECS,
    };
    if !secs.is_finite() || secs <= 0.0 {
        return Err(mlua::Error::runtime(format!(
            "machinectl.{kind}: timeout must be a positive finite number"
        )));
    }
    Ok(std::time::Duration::from_secs_f64(secs))
}

/// True iff the current process is running as root. machinectl/importctl
/// require root or polkit; when running unprivileged we prepend `sudo -n`
/// (passwordless sudo, fail-fast if a password is needed). Cached because
/// uid never changes during a process lifetime.
fn is_root() -> bool {
    static IS_ROOT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *IS_ROOT.get_or_init(|| {
        // SAFETY: getuid() has no preconditions and never fails.
        unsafe { libc::getuid() == 0 }
    })
}

/// Spawn machinectl with the given args, drain stdout/stderr in background
/// tasks, wait with timeout, kill+reap explicitly on timeout.
///
/// When running non-root, prepend `sudo -n` so the operator's NOPASSWD
/// sudoers entry handles polkit elevation. machinectl pull-tar / importctl
/// would otherwise return "Interactive authentication required".
async fn run_machinectl(args: Vec<&str>, timeout: std::time::Duration, op_label: &str) -> mlua::Result<(i32, Vec<u8>, Vec<u8>)> {
    let mut cmd = if is_root() {
        let mut c = tokio::process::Command::new("machinectl");
        c.args(&args);
        c
    } else {
        let mut c = tokio::process::Command::new("sudo");
        c.arg("-n").arg("machinectl");
        c.args(&args);
        c
    };
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| {
        mlua::Error::runtime(format!("machinectl.{op_label}: spawn: {e}"))
    })?;

    let stdout_h = child.stdout.take();
    let stderr_h = child.stderr.take();
    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(mut s) = stdout_h {
            use tokio::io::AsyncReadExt;
            let _ = s.read_to_end(&mut buf).await;
        }
        buf
    });
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(mut s) = stderr_h {
            use tokio::io::AsyncReadExt;
            let _ = s.read_to_end(&mut buf).await;
        }
        buf
    });

    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => Ok((
            status.code().unwrap_or(-1),
            stdout_task.await.unwrap_or_default(),
            stderr_task.await.unwrap_or_default(),
        )),
        Ok(Err(e)) => Err(mlua::Error::runtime(format!(
            "machinectl.{op_label}: wait: {e}"
        ))),
        Err(_elapsed) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            Err(mlua::Error::runtime(format!(
                "machinectl.{op_label}: timed out after {}s",
                timeout.as_secs()
            )))
        }
    }
}

fn build_result(lua: &Lua, status: i32, stdout: Vec<u8>, stderr: Vec<u8>) -> mlua::Result<Table> {
    let r = lua.create_table()?;
    r.set("status", status as i64)?;
    r.set("stdout", String::from_utf8_lossy(&stdout).to_string())?;
    r.set("stderr", String::from_utf8_lossy(&stderr).to_string())?;
    Ok(r)
}

pub fn register_machinectl(lua: &Lua) -> mlua::Result<()> {
    let t = lua.create_table()?;

    // pull_tar(url, name, opts?) — fetch a .tar.xz / .tar.gz / .raw.xz rootfs.
    // Wraps `machinectl pull-tar [--verify=no] <url> <name>`.
    t.set(
        "pull_tar",
        lua.create_async_function(|lua, args: mlua::MultiValue| async move {
            let mut iter = args.into_iter();
            let url: String = iter
                .next()
                .ok_or_else(|| mlua::Error::runtime("machinectl.pull_tar: url required"))
                .and_then(|v| lua.unpack(v))?;
            let name: String = iter
                .next()
                .ok_or_else(|| mlua::Error::runtime("machinectl.pull_tar: name required"))
                .and_then(|v| lua.unpack(v))?;
            let opts: Option<Table> = match iter.next() {
                Some(mlua::Value::Table(t)) => Some(t),
                _ => None,
            };
            validate_url("pull_tar", &url)?;
            validate_name("pull_tar", &name)?;
            let timeout = parse_timeout(opts.as_ref(), "pull_tar")?;

            // Default --verify=no — operators using TLS-protected URLs from
            // trusted sources don't need machinectl's signature check, which
            // requires Tukaani signatures rarely available outside Debian
            // archive. Caller sets verify=true to opt in.
            let verify: bool = opts
                .as_ref()
                .and_then(|t| t.get::<Option<bool>>("verify").ok().flatten())
                .unwrap_or(false);

            // `--` ends option parsing so a name beginning with '-' doesn't
            // become a flag. (validate_name forbids that already, but defense
            // in depth.)
            let mut argv: Vec<&str> = vec!["pull-tar"];
            if !verify {
                argv.push("--verify=no");
            }
            argv.push("--");
            argv.push(&url);
            argv.push(&name);

            let (status, stdout, stderr) = run_machinectl(argv, timeout, "pull_tar").await?;
            build_result(&lua, status, stdout, stderr)
        })?,
    )?;

    // pull_raw(url, name, opts?) — same shape but for .raw images.
    t.set(
        "pull_raw",
        lua.create_async_function(|lua, args: mlua::MultiValue| async move {
            let mut iter = args.into_iter();
            let url: String = iter
                .next()
                .ok_or_else(|| mlua::Error::runtime("machinectl.pull_raw: url required"))
                .and_then(|v| lua.unpack(v))?;
            let name: String = iter
                .next()
                .ok_or_else(|| mlua::Error::runtime("machinectl.pull_raw: name required"))
                .and_then(|v| lua.unpack(v))?;
            let opts: Option<Table> = match iter.next() {
                Some(mlua::Value::Table(t)) => Some(t),
                _ => None,
            };
            validate_url("pull_raw", &url)?;
            validate_name("pull_raw", &name)?;
            let timeout = parse_timeout(opts.as_ref(), "pull_raw")?;
            let verify: bool = opts
                .as_ref()
                .and_then(|t| t.get::<Option<bool>>("verify").ok().flatten())
                .unwrap_or(false);

            let mut argv: Vec<&str> = vec!["pull-raw"];
            if !verify {
                argv.push("--verify=no");
            }
            argv.push("--");
            argv.push(&url);
            argv.push(&name);

            let (status, stdout, stderr) = run_machinectl(argv, timeout, "pull_raw").await?;
            build_result(&lua, status, stdout, stderr)
        })?,
    )?;

    // remove(name, opts?) — `machinectl remove <name>`. Removes the image
    // from /var/lib/machines. Errors if the machine is currently running
    // (caller should poweroff first).
    t.set(
        "remove",
        lua.create_async_function(|lua, args: mlua::MultiValue| async move {
            let mut iter = args.into_iter();
            let name: String = iter
                .next()
                .ok_or_else(|| mlua::Error::runtime("machinectl.remove: name required"))
                .and_then(|v| lua.unpack(v))?;
            let opts: Option<Table> = match iter.next() {
                Some(mlua::Value::Table(t)) => Some(t),
                _ => None,
            };
            validate_name("remove", &name)?;
            let timeout = parse_timeout(opts.as_ref(), "remove")?;

            let argv: Vec<&str> = vec!["remove", "--", &name];
            let (status, stdout, stderr) = run_machinectl(argv, timeout, "remove").await?;
            build_result(&lua, status, stdout, stderr)
        })?,
    )?;

    // clone(src, dst, opts?) — `machinectl clone <src> <dst>`. Snapshot or
    // copy depending on storage backend. Near-instant on btrfs.
    t.set(
        "clone",
        lua.create_async_function(|lua, args: mlua::MultiValue| async move {
            let mut iter = args.into_iter();
            let src: String = iter
                .next()
                .ok_or_else(|| mlua::Error::runtime("machinectl.clone: src required"))
                .and_then(|v| lua.unpack(v))?;
            let dst: String = iter
                .next()
                .ok_or_else(|| mlua::Error::runtime("machinectl.clone: dst required"))
                .and_then(|v| lua.unpack(v))?;
            let opts: Option<Table> = match iter.next() {
                Some(mlua::Value::Table(t)) => Some(t),
                _ => None,
            };
            validate_name("clone(src)", &src)?;
            validate_name("clone(dst)", &dst)?;
            let timeout = parse_timeout(opts.as_ref(), "clone")?;

            let argv: Vec<&str> = vec!["clone", "--", &src, &dst];
            let (status, stdout, stderr) = run_machinectl(argv, timeout, "clone").await?;
            build_result(&lua, status, stdout, stderr)
        })?,
    )?;

    lua.globals().set("machinectl", t)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_accepts_normal_names() {
        validate_name("test", "apex").unwrap();
        validate_name("test", "agent-x").unwrap();
        validate_name("test", "node_01").unwrap();
        validate_name("test", "host.local").unwrap();
    }

    #[test]
    fn validate_name_rejects_empty() {
        assert!(validate_name("test", "").is_err());
    }

    #[test]
    fn validate_name_rejects_path_traversal() {
        assert!(validate_name("test", "../../../etc/passwd").is_err());
        assert!(validate_name("test", "name/with/slash").is_err());
    }

    #[test]
    fn validate_name_rejects_shell_meta() {
        assert!(validate_name("test", "name;rm -rf /").is_err());
        assert!(validate_name("test", "$(whoami)").is_err());
        assert!(validate_name("test", "name with space").is_err());
    }

    #[test]
    fn validate_name_rejects_leading_dash() {
        assert!(validate_name("test", "-rf").is_err());
    }

    #[test]
    fn validate_url_accepts_http_and_https() {
        validate_url("test", "https://example.com/image.tar.xz").unwrap();
        validate_url("test", "http://example.com/image.raw").unwrap();
    }

    #[test]
    fn validate_url_rejects_other_schemes() {
        assert!(validate_url("test", "ftp://example.com/file").is_err());
        assert!(validate_url("test", "file:///etc/passwd").is_err());
    }

    #[test]
    fn validate_url_rejects_control_chars() {
        assert!(validate_url("test", "https://example.com/\nfoo").is_err());
        assert!(validate_url("test", "https://example.com/\rfoo").is_err());
    }
}
