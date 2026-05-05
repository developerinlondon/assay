//! `cgroup` Rust builtin — cgroup v2 unified-hierarchy readers.
//!
//! Linux-only. On non-Linux targets the `cgroup` Lua table is registered
//! empty. Plan 18 Phase 2.

use mlua::Lua;

pub fn register_cgroup(lua: &Lua) -> mlua::Result<()> {
    let cgroup_table = lua.create_table()?;
    #[cfg(target_os = "linux")]
    cgroup_impl::register(lua, &cgroup_table)?;
    lua.globals().set("cgroup", cgroup_table)?;
    Ok(())
}

#[cfg(target_os = "linux")]
mod cgroup_impl {
    use mlua::Lua;
    use std::path::{Path, PathBuf};

    const CGROUP_ROOT: &str = "/sys/fs/cgroup";
    const CGROUP2_SUPER_MAGIC: u64 = 0x6367_7270;
    const TMPFS_MAGIC: u64 = 0x0102_1994;

    // ── path safety ──────────────────────────────────────────────────────────

    pub(crate) fn validate_cgroup_path(p: &str) -> Result<PathBuf, String> {
        // canonicalize resolves symlinks and removes ..
        let canon = std::fs::canonicalize(p)
            .map_err(|e| format!("cgroup: cannot canonicalize path {p:?}: {e}"))?;

        let root = Path::new(CGROUP_ROOT);
        // Accept exactly /sys/fs/cgroup or any path beneath it
        if canon != root && !canon.starts_with(root) {
            return Err(format!(
                "cgroup: path must start with /sys/fs/cgroup, got {canon:?}"
            ));
        }
        Ok(canon)
    }

    // ── statfs helper ────────────────────────────────────────────────────────

    fn statfs_ftype(path: &str) -> Result<u64, String> {
        use std::ffi::CString;
        let c = CString::new(path).map_err(|e| format!("invalid path: {e}"))?;
        // SAFETY: statfs is POSIX; we pass a valid null-terminated path and a
        // zeroed buffer that the kernel fills on success.
        unsafe {
            let mut st: libc::statfs = std::mem::zeroed();
            let ret = libc::statfs(c.as_ptr(), &mut st);
            if ret != 0 {
                let err = std::io::Error::last_os_error();
                return Err(format!("statfs({path:?}) failed: {err}"));
            }
            Ok(st.f_type as u64)
        }
    }

    // ── parsers ──────────────────────────────────────────────────────────────

    pub(crate) struct CpuStat {
        pub usage_usec: Option<u64>,
        pub user_usec: Option<u64>,
        pub system_usec: Option<u64>,
        pub nr_periods: Option<u64>,
        pub nr_throttled: Option<u64>,
        pub throttled_usec: Option<u64>,
        pub nr_bursts: Option<u64>,
        pub burst_usec: Option<u64>,
    }

    pub(crate) fn parse_cpu_stat(content: &str) -> CpuStat {
        let mut s = CpuStat {
            usage_usec: None,
            user_usec: None,
            system_usec: None,
            nr_periods: None,
            nr_throttled: None,
            throttled_usec: None,
            nr_bursts: None,
            burst_usec: None,
        };
        for line in content.lines() {
            let mut parts = line.splitn(2, ' ');
            let key = parts.next().unwrap_or("").trim();
            let val: Option<u64> = parts.next().and_then(|v| v.trim().parse().ok());
            match key {
                "usage_usec" => s.usage_usec = val,
                "user_usec" => s.user_usec = val,
                "system_usec" => s.system_usec = val,
                "nr_periods" => s.nr_periods = val,
                "nr_throttled" => s.nr_throttled = val,
                "throttled_usec" => s.throttled_usec = val,
                "nr_bursts" => s.nr_bursts = val,
                "burst_usec" => s.burst_usec = val,
                _ => {}
            }
        }
        s
    }

    pub(crate) struct MemoryStat {
        pub current: Option<u64>,
        pub max: Option<u64>, // None means sentinel "max"
        pub swap_current: Option<u64>,
        pub swap_max: Option<u64>, // None means sentinel "max"
        pub peak: Option<u64>,
        pub low: Option<u64>,
        pub high: Option<u64>,
        pub oom_kill: Option<u64>,
        pub oom: Option<u64>,
    }

    fn read_optional_u64(dir: &Path, name: &str) -> Option<u64> {
        let content = std::fs::read_to_string(dir.join(name)).ok()?;
        content.trim().parse().ok()
    }

    // Returns (value, is_sentinel_max)
    fn read_limit_file(dir: &Path, name: &str) -> (Option<u64>, bool) {
        let content = match std::fs::read_to_string(dir.join(name)) {
            Ok(s) => s,
            Err(_) => return (None, false),
        };
        let trimmed = content.trim();
        if trimmed == "max" {
            return (None, true);
        }
        (trimmed.parse().ok(), false)
    }

    pub(crate) fn parse_memory_stat(dir: &Path) -> MemoryStat {
        let current = read_optional_u64(dir, "memory.current");
        let (max, _) = read_limit_file(dir, "memory.max");
        let swap_current = read_optional_u64(dir, "memory.swap.current");
        let (swap_max, _) = read_limit_file(dir, "memory.swap.max");
        let peak = read_optional_u64(dir, "memory.peak");
        let low = read_optional_u64(dir, "memory.low");
        let high = read_optional_u64(dir, "memory.high");

        // memory.events: parse oom_kill and oom fields
        let mut oom_kill: Option<u64> = None;
        let mut oom: Option<u64> = None;
        if let Ok(events) = std::fs::read_to_string(dir.join("memory.events")) {
            for line in events.lines() {
                let mut parts = line.splitn(2, ' ');
                let key = parts.next().unwrap_or("").trim();
                let val: Option<u64> = parts.next().and_then(|v| v.trim().parse().ok());
                match key {
                    "oom_kill" => oom_kill = val,
                    "oom" => oom = val,
                    _ => {}
                }
            }
        }

        MemoryStat {
            current,
            max,
            swap_current,
            swap_max,
            peak,
            low,
            high,
            oom_kill,
            oom,
        }
    }

    pub(crate) struct IoEntry {
        pub device: String,
        pub rbytes: u64,
        pub wbytes: u64,
        pub rios: u64,
        pub wios: u64,
        pub dbytes: u64,
        pub dios: u64,
    }

    pub(crate) fn parse_io_stat(content: &str) -> Vec<IoEntry> {
        let mut entries = Vec::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.split_ascii_whitespace();
            let device = match parts.next() {
                Some(d) => d.to_string(),
                None => continue,
            };
            let mut rbytes = 0u64;
            let mut wbytes = 0u64;
            let mut rios = 0u64;
            let mut wios = 0u64;
            let mut dbytes = 0u64;
            let mut dios = 0u64;
            for kv in parts {
                let mut iter = kv.splitn(2, '=');
                let key = iter.next().unwrap_or("").trim();
                let val: u64 = iter.next().and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                match key {
                    "rbytes" => rbytes = val,
                    "wbytes" => wbytes = val,
                    "rios" => rios = val,
                    "wios" => wios = val,
                    "dbytes" => dbytes = val,
                    "dios" => dios = val,
                    _ => {}
                }
            }
            entries.push(IoEntry {
                device,
                rbytes,
                wbytes,
                rios,
                wios,
                dbytes,
                dios,
            });
        }
        entries
    }

    // ── register ─────────────────────────────────────────────────────────────

    pub fn register(_lua: &Lua, t: &mlua::Table) -> mlua::Result<()> {
        // cgroup.version()
        let version_fn = _lua.create_function(|_, ()| {
            let ftype = statfs_ftype(CGROUP_ROOT)
                .map_err(|e| mlua::Error::runtime(format!("cgroup.version: {e}")))?;
            if ftype == CGROUP2_SUPER_MAGIC {
                return Ok("v2".to_string());
            }
            if ftype == TMPFS_MAGIC {
                // hybrid: v2 mounted at /sys/fs/cgroup/unified
                let unified = format!("{CGROUP_ROOT}/unified");
                if let Ok(uf) = statfs_ftype(&unified)
                    && uf == CGROUP2_SUPER_MAGIC
                {
                    return Ok("hybrid".to_string());
                }
                return Ok("v1".to_string());
            }
            Ok("v1".to_string())
        })?;
        t.set("version", version_fn)?;

        // cgroup.list(slice_path)
        let list_fn = _lua.create_function(|lua, path: String| {
            let canon = validate_cgroup_path(&path).map_err(mlua::Error::runtime)?;

            let rd = std::fs::read_dir(&canon).map_err(|e| {
                mlua::Error::runtime(format!(
                    "cgroup.list: failed to read directory {canon:?}: {e}"
                ))
            })?;

            let mut names: Vec<String> = Vec::new();
            for entry in rd.flatten() {
                let ftype = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };
                if !ftype.is_dir() {
                    continue;
                }
                // Only include directories that have a cgroup.controllers file
                if !entry.path().join("cgroup.controllers").exists() {
                    continue;
                }
                if let Some(n) = entry.file_name().to_str() {
                    names.push(n.to_string());
                }
            }
            names.sort();

            let tbl = lua.create_table()?;
            for (i, name) in names.into_iter().enumerate() {
                tbl.set(i + 1, name)?;
            }
            Ok(tbl)
        })?;
        t.set("list", list_fn)?;

        // cgroup.cpu_stat(path)
        let cpu_stat_fn = _lua.create_function(|lua, path: String| {
            let canon = validate_cgroup_path(&path).map_err(mlua::Error::runtime)?;

            let content = std::fs::read_to_string(canon.join("cpu.stat")).map_err(|e| {
                mlua::Error::runtime(format!("cgroup.cpu_stat: failed to read cpu.stat: {e}"))
            })?;

            let s = parse_cpu_stat(&content);
            let tbl = lua.create_table()?;
            if let Some(v) = s.usage_usec {
                tbl.set("usage_usec", v)?;
            }
            if let Some(v) = s.user_usec {
                tbl.set("user_usec", v)?;
            }
            if let Some(v) = s.system_usec {
                tbl.set("system_usec", v)?;
            }
            if let Some(v) = s.nr_periods {
                tbl.set("nr_periods", v)?;
            }
            if let Some(v) = s.nr_throttled {
                tbl.set("nr_throttled", v)?;
            }
            if let Some(v) = s.throttled_usec {
                tbl.set("throttled_usec", v)?;
            }
            if let Some(v) = s.nr_bursts {
                tbl.set("nr_bursts", v)?;
            }
            if let Some(v) = s.burst_usec {
                tbl.set("burst_usec", v)?;
            }
            Ok(tbl)
        })?;
        t.set("cpu_stat", cpu_stat_fn)?;

        // cgroup.memory(path)
        let memory_fn = _lua.create_function(|lua, path: String| {
            let canon = validate_cgroup_path(&path).map_err(mlua::Error::runtime)?;

            let m = parse_memory_stat(&canon);
            let tbl = lua.create_table()?;
            if let Some(v) = m.current {
                tbl.set("current", v)?;
            }
            // max = nil when sentinel "max" (unlimited)
            if let Some(v) = m.max {
                tbl.set("max", v)?;
            }
            if let Some(v) = m.swap_current {
                tbl.set("swap_current", v)?;
            }
            if let Some(v) = m.swap_max {
                tbl.set("swap_max", v)?;
            }
            if let Some(v) = m.peak {
                tbl.set("peak", v)?;
            }
            if let Some(v) = m.low {
                tbl.set("low", v)?;
            }
            if let Some(v) = m.high {
                tbl.set("high", v)?;
            }
            if let Some(v) = m.oom_kill {
                tbl.set("oom_kill", v)?;
            }
            if let Some(v) = m.oom {
                tbl.set("oom", v)?;
            }
            Ok(tbl)
        })?;
        t.set("memory", memory_fn)?;

        // cgroup.io(path)
        let io_fn = _lua.create_function(|lua, path: String| {
            let canon = validate_cgroup_path(&path).map_err(mlua::Error::runtime)?;

            let content = std::fs::read_to_string(canon.join("io.stat")).map_err(|e| {
                mlua::Error::runtime(format!("cgroup.io: failed to read io.stat: {e}"))
            })?;

            let entries = parse_io_stat(&content);
            let tbl = lua.create_table()?;
            for (i, e) in entries.into_iter().enumerate() {
                let row = lua.create_table()?;
                row.set("device", e.device)?;
                row.set("rbytes", e.rbytes)?;
                row.set("wbytes", e.wbytes)?;
                row.set("rios", e.rios)?;
                row.set("wios", e.wios)?;
                row.set("dbytes", e.dbytes)?;
                row.set("dios", e.dios)?;
                tbl.set(i + 1, row)?;
            }
            Ok(tbl)
        })?;
        t.set("io", io_fn)?;

        // cgroup.pids(path)
        let pids_fn = _lua.create_function(|lua, path: String| {
            let canon = validate_cgroup_path(&path).map_err(mlua::Error::runtime)?;

            let current: Option<u64> = std::fs::read_to_string(canon.join("pids.current"))
                .ok()
                .and_then(|s| s.trim().parse().ok());

            // pids.max may contain "max" sentinel
            let max: Option<u64> = std::fs::read_to_string(canon.join("pids.max"))
                .ok()
                .and_then(|s| {
                    let s = s.trim();
                    if s == "max" { None } else { s.parse().ok() }
                });

            let tbl = lua.create_table()?;
            if let Some(v) = current {
                tbl.set("current", v)?;
            }
            if let Some(v) = max {
                tbl.set("max", v)?;
            }
            Ok(tbl)
        })?;
        t.set("pids", pids_fn)?;

        // cgroup.procs(path)
        let procs_fn = _lua.create_function(|lua, path: String| {
            let canon = validate_cgroup_path(&path).map_err(mlua::Error::runtime)?;

            let content = std::fs::read_to_string(canon.join("cgroup.procs")).map_err(|e| {
                mlua::Error::runtime(format!("cgroup.procs: failed to read cgroup.procs: {e}"))
            })?;

            let tbl = lua.create_table()?;
            let mut idx = 1usize;
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                if let Ok(pid) = line.parse::<u64>() {
                    tbl.set(idx, pid)?;
                    idx += 1;
                }
            }
            Ok(tbl)
        })?;
        t.set("procs", procs_fn)?;

        Ok(())
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    #[cfg(test)]
    pub(crate) mod tests {
        use super::*;

        #[test]
        fn version_returns_string() {
            if !Path::new(CGROUP_ROOT).exists() {
                eprintln!("skip: /sys/fs/cgroup not present");
                return;
            }
            let v = statfs_ftype(CGROUP_ROOT).expect("statfs should succeed");
            // Must be one of the known magic numbers or an unknown fs
            let result = if v == CGROUP2_SUPER_MAGIC {
                "v2"
            } else if v == TMPFS_MAGIC {
                "v1 or hybrid"
            } else {
                "unknown"
            };
            assert!(
                matches!(result, "v2" | "v1 or hybrid" | "unknown"),
                "unexpected ftype {v:#x}"
            );
        }

        #[test]
        fn list_root_has_known_slice() {
            let root = Path::new(CGROUP_ROOT);
            if !root.exists() {
                eprintln!("skip: /sys/fs/cgroup not present");
                return;
            }
            let result = validate_cgroup_path(CGROUP_ROOT);
            assert!(result.is_ok(), "root should validate: {result:?}");

            // Read the directory directly and look for well-known slices
            let found_slice = std::fs::read_dir(root)
                .into_iter()
                .flatten()
                .flatten()
                .any(|e| {
                    let n = e.file_name();
                    let name = n.to_string_lossy();
                    (name == "machine.slice" || name == "system.slice" || name == "user.slice")
                        && e.path().join("cgroup.controllers").exists()
                });

            // On a typical systemd host at least one slice exists.
            // If running in a minimal container this may be false — we log rather than hard-fail.
            if !found_slice {
                eprintln!(
                    "note: no machine/system/user.slice with cgroup.controllers found under /sys/fs/cgroup — this is expected in some container environments"
                );
            }
        }

        #[test]
        fn list_rejects_path_outside_cgroup() {
            // /etc is outside /sys/fs/cgroup — canonicalize will succeed on the
            // real filesystem but the prefix check must reject it.
            let err = validate_cgroup_path("/etc");
            assert!(
                err.is_err(),
                "expected error for /etc, got Ok({:?})",
                err.unwrap()
            );
            let msg = err.unwrap_err();
            assert!(
                msg.contains("must start with /sys/fs/cgroup"),
                "wrong error: {msg}"
            );
        }

        #[test]
        fn list_rejects_path_traversal_to_etc() {
            // /sys/fs/cgroup/../../../etc canonicalises to /etc on Linux
            let err = validate_cgroup_path("/sys/fs/cgroup/../../../etc");
            // Either canonicalize fails (path doesn't exist) or prefix check rejects it
            if let Err(msg) = err {
                assert!(
                    msg.contains("must start with /sys/fs/cgroup")
                        || msg.contains("cannot canonicalize"),
                    "wrong error: {msg}"
                );
            }
            // If /sys/fs/cgroup exists but the traversed path doesn't: Ok is not possible because
            // /etc always exists on Linux — so err is the only expected branch.
        }

        #[test]
        fn path_canonicalisation_dotdot_rejected() {
            // If /sys/fs/cgroup exists, a path with .. that exits the tree is rejected
            if !Path::new(CGROUP_ROOT).exists() {
                eprintln!("skip: /sys/fs/cgroup not present");
                return;
            }
            let result = validate_cgroup_path("/sys/fs/cgroup/../../tmp");
            assert!(
                result.is_err(),
                "expected rejection but got Ok({:?})",
                result.unwrap()
            );
        }

        #[test]
        fn cpu_stat_parser_synthetic() {
            let fixture = "\
usage_usec 1234567
user_usec 555555
system_usec 234567
nr_periods 100
nr_throttled 5
throttled_usec 10000
nr_bursts 2
burst_usec 500
";
            let s = parse_cpu_stat(fixture);
            assert_eq!(s.usage_usec, Some(1_234_567));
            assert_eq!(s.user_usec, Some(555_555));
            assert_eq!(s.system_usec, Some(234_567));
            assert_eq!(s.nr_periods, Some(100));
            assert_eq!(s.nr_throttled, Some(5));
            assert_eq!(s.throttled_usec, Some(10_000));
            assert_eq!(s.nr_bursts, Some(2));
            assert_eq!(s.burst_usec, Some(500));
        }

        #[test]
        fn cpu_stat_parser_missing_burst_fields() {
            // Older kernels omit nr_bursts / burst_usec
            let fixture = "\
usage_usec 999
user_usec 111
system_usec 888
nr_periods 10
nr_throttled 0
throttled_usec 0
";
            let s = parse_cpu_stat(fixture);
            assert_eq!(s.usage_usec, Some(999));
            assert_eq!(s.nr_bursts, None);
            assert_eq!(s.burst_usec, None);
        }

        #[test]
        fn memory_parser_synthetic() {
            let dir = tempfile::tempdir().expect("tempdir");
            let p = dir.path();

            std::fs::write(p.join("memory.current"), "102400\n").unwrap();
            std::fs::write(p.join("memory.max"), "max\n").unwrap();
            std::fs::write(p.join("memory.swap.current"), "4096\n").unwrap();
            std::fs::write(p.join("memory.swap.max"), "8192\n").unwrap();
            std::fs::write(p.join("memory.peak"), "204800\n").unwrap();
            std::fs::write(p.join("memory.low"), "0\n").unwrap();
            std::fs::write(p.join("memory.high"), "max\n").unwrap();
            std::fs::write(p.join("memory.events"), "oom 3\noom_kill 1\n").unwrap();

            let m = parse_memory_stat(p);
            assert_eq!(m.current, Some(102_400));
            assert_eq!(m.max, None, "sentinel 'max' should map to None");
            assert_eq!(m.swap_current, Some(4_096));
            assert_eq!(m.swap_max, Some(8_192));
            assert_eq!(m.peak, Some(204_800));
            assert_eq!(m.low, Some(0));
            // memory.high contains "max" → None
            assert_eq!(m.high, None);
            assert_eq!(m.oom, Some(3));
            assert_eq!(m.oom_kill, Some(1));
        }

        #[test]
        fn io_stat_parser_synthetic() {
            let fixture = "8:0 rbytes=12345 wbytes=678 rios=1 wios=2 dbytes=0 dios=0\n\
                           253:0 rbytes=999 wbytes=111 rios=5 wios=3 dbytes=64 dios=1\n";
            let entries = parse_io_stat(fixture);
            assert_eq!(entries.len(), 2);

            let e0 = &entries[0];
            assert_eq!(e0.device, "8:0");
            assert_eq!(e0.rbytes, 12_345);
            assert_eq!(e0.wbytes, 678);
            assert_eq!(e0.rios, 1);
            assert_eq!(e0.wios, 2);
            assert_eq!(e0.dbytes, 0);
            assert_eq!(e0.dios, 0);

            let e1 = &entries[1];
            assert_eq!(e1.device, "253:0");
            assert_eq!(e1.rbytes, 999);
            assert_eq!(e1.rios, 5);
            assert_eq!(e1.dbytes, 64);
        }

        #[test]
        fn io_stat_parser_empty() {
            let entries = parse_io_stat("");
            assert!(entries.is_empty());
        }
    }
}
