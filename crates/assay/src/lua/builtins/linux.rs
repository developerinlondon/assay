//! `linux` Rust builtin — `/proc` and `/sys/fs/...` readers.
//!
//! Linux-only. On non-Linux targets the `linux` Lua table is registered
//! empty; no methods are exposed. This matches the `process` builtin's
//! posture (it has Linux-fast and macOS-fallback paths) but goes further:
//! /proc has no analogue on macOS / Windows, so there's nothing to fall
//! back to. Callers can probe `linux.kernel ~= nil` to detect support.

use mlua::Lua;

/// Register the `linux` global table in the given Lua VM.
///
/// On Linux, all functions listed in the module-level doc are populated.
/// On other platforms the table is registered but empty.
pub fn register_linux(lua: &Lua) -> mlua::Result<()> {
    let linux_table = lua.create_table()?;
    #[cfg(target_os = "linux")]
    linux_impl::register(lua, &linux_table)?;
    lua.globals().set("linux", linux_table)?;
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux_impl {
    use mlua::Lua;
    use procfs::prelude::*;

    pub fn register(lua: &Lua, t: &mlua::Table) -> mlua::Result<()> {
        t.set("kernel", lua.create_function(kernel)?)?;
        t.set("uptime", lua.create_function(uptime)?)?;
        t.set("loadavg", lua.create_function(loadavg)?)?;
        t.set("cpu_stat", lua.create_function(cpu_stat)?)?;
        t.set("cpu_stat_per_core", lua.create_function(cpu_stat_per_core)?)?;
        t.set("cpu_percent", lua.create_function(cpu_percent)?)?;
        t.set("meminfo", lua.create_function(meminfo)?)?;
        t.set("netdev", lua.create_function(netdev)?)?;
        t.set("diskstats", lua.create_function(diskstats)?)?;
        t.set("proc_stat", lua.create_function(proc_stat)?)?;
        t.set("proc_status", lua.create_function(proc_status)?)?;
        Ok(())
    }

    // -- helpers ----------------------------------------------------------

    fn cpu_time_to_table(lua: &Lua, ct: &procfs::CpuTime) -> mlua::Result<mlua::Table> {
        let t = lua.create_table()?;
        t.set("user", ct.user)?;
        t.set("nice", ct.nice)?;
        t.set("system", ct.system)?;
        t.set("idle", ct.idle)?;
        t.set("iowait", ct.iowait)?;
        t.set("irq", ct.irq)?;
        t.set("softirq", ct.softirq)?;
        t.set("steal", ct.steal)?;
        t.set("guest", ct.guest)?;
        t.set("guest_nice", ct.guest_nice)?;
        Ok(t)
    }

    // -- function implementations -----------------------------------------

    fn kernel(lua: &Lua, _: ()) -> mlua::Result<mlua::Table> {
        // /proc/version string
        let version = std::fs::read_to_string("/proc/version")
            .map(|s| s.trim().to_string())
            .map_err(|e| mlua::Error::runtime(format!("linux.kernel: /proc/version: {e}")))?;

        // hostname from /proc/sys/kernel/hostname (always available on Linux)
        let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .map(|s| s.trim().to_string())
            .map_err(|e| {
                mlua::Error::runtime(format!("linux.kernel: /proc/sys/kernel/hostname: {e}"))
            })?;

        // btime from /proc/stat
        let btime = procfs::KernelStats::current()
            .map(|ks| ks.btime)
            .map_err(|e| mlua::Error::runtime(format!("linux.kernel: /proc/stat: {e}")))?;

        // /etc/os-release parsed as key=value pairs
        let os_release_table = lua.create_table()?;
        if let Ok(contents) = std::fs::read_to_string("/etc/os-release") {
            for line in contents.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some((key, val)) = line.split_once('=') {
                    let val = val.trim_matches('"');
                    os_release_table.set(key, val)?;
                }
            }
        }

        let t = lua.create_table()?;
        t.set("version", version)?;
        t.set("hostname", hostname)?;
        t.set("os_release", os_release_table)?;
        t.set("btime", btime)?;
        Ok(t)
    }

    fn uptime(lua: &Lua, _: ()) -> mlua::Result<mlua::Table> {
        let u = procfs::Uptime::current()
            .map_err(|e| mlua::Error::runtime(format!("linux.uptime: {e}")))?;
        let t = lua.create_table()?;
        t.set("uptime_secs", u.uptime)?;
        t.set("idle_secs", u.idle)?;
        Ok(t)
    }

    fn loadavg(lua: &Lua, _: ()) -> mlua::Result<mlua::Table> {
        let la = procfs::LoadAverage::current()
            .map_err(|e| mlua::Error::runtime(format!("linux.loadavg: {e}")))?;
        let t = lua.create_table()?;
        t.set("one", la.one)?;
        t.set("five", la.five)?;
        t.set("fifteen", la.fifteen)?;
        t.set("running", la.cur)?;
        t.set("total", la.max)?;
        t.set("last_pid", la.latest_pid)?;
        Ok(t)
    }

    fn cpu_stat(lua: &Lua, _: ()) -> mlua::Result<mlua::Table> {
        let ks = procfs::KernelStats::current()
            .map_err(|e| mlua::Error::runtime(format!("linux.cpu_stat: {e}")))?;
        cpu_time_to_table(lua, &ks.total)
    }

    fn cpu_stat_per_core(lua: &Lua, _: ()) -> mlua::Result<mlua::Table> {
        let ks = procfs::KernelStats::current()
            .map_err(|e| mlua::Error::runtime(format!("linux.cpu_stat_per_core: {e}")))?;
        let list = lua.create_table()?;
        for (i, ct) in ks.cpu_time.iter().enumerate() {
            let row = cpu_time_to_table(lua, ct)?;
            row.set("cpu", i as u32)?;
            list.set(i + 1, row)?;
        }
        Ok(list)
    }

    /// Pull a u64 jiffy field from a Lua table; missing key or nil → 0.
    fn get_jiffy(t: &mlua::Table, key: &str) -> mlua::Result<u64> {
        match t.get::<mlua::Value>(key)? {
            mlua::Value::Integer(n) => Ok(n as u64),
            mlua::Value::Number(n) => Ok(n as u64),
            _ => Ok(0),
        }
    }

    fn cpu_percent(lua: &Lua, (prev, curr): (mlua::Table, mlua::Table)) -> mlua::Result<mlua::Table> {
        let fields = ["user", "nice", "system", "idle", "iowait", "irq", "softirq", "steal", "guest", "guest_nice"];

        let mut delta_total: u64 = 0;
        let mut delta_busy: u64 = 0;
        for field in &fields {
            let p = get_jiffy(&prev, field)?;
            let c = get_jiffy(&curr, field)?;
            let d = c.saturating_sub(p);
            delta_total += d;
            if *field != "idle" {
                delta_busy += d;
            }
        }

        let total_pct = if delta_total == 0 {
            0.0_f64
        } else {
            100.0 * delta_busy as f64 / delta_total as f64
        };

        let t = lua.create_table()?;
        t.set("total_pct", total_pct)?;
        Ok(t)
    }

    fn meminfo(lua: &Lua, _: ()) -> mlua::Result<mlua::Table> {
        let m = procfs::Meminfo::current()
            .map_err(|e| mlua::Error::runtime(format!("linux.meminfo: {e}")))?;

        // procfs Meminfo fields are in kB; multiply by 1024 to get bytes.
        // Option fields expose nil to Lua when None.
        let t = lua.create_table()?;

        macro_rules! set_kb {
            ($field:expr, $name:expr) => {
                t.set($name, $field * 1024)?;
            };
        }
        macro_rules! set_kb_opt {
            ($field:expr, $name:expr) => {
                if let Some(v) = $field {
                    t.set($name, v * 1024)?;
                }
            };
        }

        set_kb!(m.mem_total, "total");
        set_kb!(m.mem_free, "free");
        set_kb_opt!(m.mem_available, "available");
        set_kb!(m.buffers, "buffers");
        set_kb!(m.cached, "cached");
        set_kb!(m.swap_cached, "swap_cached");
        set_kb!(m.active, "active");
        set_kb!(m.inactive, "inactive");
        set_kb_opt!(m.active_anon, "active_anon");
        set_kb_opt!(m.inactive_anon, "inactive_anon");
        set_kb_opt!(m.active_file, "active_file");
        set_kb_opt!(m.inactive_file, "inactive_file");
        set_kb_opt!(m.unevictable, "unevictable");
        set_kb_opt!(m.mlocked, "mlocked");
        set_kb_opt!(m.high_total, "high_total");
        set_kb_opt!(m.high_free, "high_free");
        set_kb_opt!(m.low_total, "low_total");
        set_kb_opt!(m.low_free, "low_free");
        set_kb!(m.swap_total, "swap_total");
        set_kb!(m.swap_free, "swap_free");
        set_kb!(m.dirty, "dirty");
        set_kb!(m.writeback, "writeback");
        set_kb_opt!(m.anon_pages, "anon_pages");
        set_kb!(m.mapped, "mapped");
        set_kb_opt!(m.shmem, "shmem");
        set_kb!(m.slab, "slab");
        set_kb_opt!(m.s_reclaimable, "s_reclaimable");
        set_kb_opt!(m.s_unreclaim, "s_unreclaim");
        set_kb_opt!(m.kernel_stack, "kernel_stack");
        set_kb_opt!(m.page_tables, "page_tables");
        set_kb_opt!(m.nfs_unstable, "nfs_unstable");
        set_kb_opt!(m.bounce, "bounce");
        set_kb_opt!(m.writeback_tmp, "writeback_tmp");
        set_kb_opt!(m.commit_limit, "commit_limit");
        set_kb!(m.committed_as, "committed_as");
        set_kb!(m.vmalloc_total, "vmalloc_total");
        set_kb!(m.vmalloc_used, "vmalloc_used");
        set_kb!(m.vmalloc_chunk, "vmalloc_chunk");
        set_kb_opt!(m.hardware_corrupted, "hardware_corrupted");
        set_kb_opt!(m.anon_hugepages, "anon_hugepages");
        set_kb_opt!(m.shmem_hugepages, "shmem_hugepages");
        set_kb_opt!(m.shmem_pmd_mapped, "shmem_pmd_mapped");
        set_kb_opt!(m.cma_total, "cma_total");
        set_kb_opt!(m.cma_free, "cma_free");
        set_kb_opt!(m.hugepages_total, "hugepages_total");
        set_kb_opt!(m.hugepages_free, "hugepages_free");
        set_kb_opt!(m.hugepages_rsvd, "hugepages_rsvd");
        set_kb_opt!(m.hugepages_surp, "hugepages_surp");
        set_kb_opt!(m.hugepagesize, "hugepagesize");
        set_kb_opt!(m.direct_map_4k, "direct_map_4k");
        set_kb_opt!(m.direct_map_2M, "direct_map_2m");
        set_kb_opt!(m.direct_map_1G, "direct_map_1g");
        set_kb_opt!(m.hugetlb, "hugetlb");
        set_kb_opt!(m.per_cpu, "per_cpu");
        set_kb_opt!(m.k_reclaimable, "k_reclaimable");
        set_kb_opt!(m.z_swap, "z_swap");
        set_kb_opt!(m.z_swapped, "z_swapped");

        Ok(t)
    }

    fn netdev(lua: &Lua, _: ()) -> mlua::Result<mlua::Table> {
        let devs = procfs::net::dev_status()
            .map_err(|e| mlua::Error::runtime(format!("linux.netdev: {e}")))?;

        let mut entries: Vec<_> = devs.into_values().collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        let list = lua.create_table()?;
        for (i, dev) in entries.iter().enumerate() {
            let row = lua.create_table()?;
            row.set("name", dev.name.as_str())?;
            row.set("rx_bytes", dev.recv_bytes)?;
            row.set("rx_packets", dev.recv_packets)?;
            row.set("rx_errs", dev.recv_errs)?;
            row.set("rx_drop", dev.recv_drop)?;
            row.set("tx_bytes", dev.sent_bytes)?;
            row.set("tx_packets", dev.sent_packets)?;
            row.set("tx_errs", dev.sent_errs)?;
            row.set("tx_drop", dev.sent_drop)?;
            list.set(i + 1, row)?;
        }
        Ok(list)
    }

    fn diskstats(lua: &Lua, _: ()) -> mlua::Result<mlua::Table> {
        let disks = procfs::diskstats()
            .map_err(|e| mlua::Error::runtime(format!("linux.diskstats: {e}")))?;

        let list = lua.create_table()?;
        for (i, d) in disks.iter().enumerate() {
            let row = lua.create_table()?;
            row.set("device", d.name.as_str())?;
            row.set("reads", d.reads)?;
            row.set("reads_merged", d.merged)?;
            row.set("sectors_read", d.sectors_read)?;
            row.set("ms_read", d.time_reading)?;
            row.set("writes", d.writes)?;
            row.set("writes_merged", d.writes_merged)?;
            row.set("sectors_written", d.sectors_written)?;
            row.set("ms_write", d.time_writing)?;
            row.set("in_progress", d.in_progress)?;
            row.set("ms_io", d.time_in_progress)?;
            list.set(i + 1, row)?;
        }
        Ok(list)
    }

    fn proc_stat(lua: &Lua, pid: i32) -> mlua::Result<mlua::Table> {
        let process = procfs::process::Process::new(pid)
            .map_err(|e| mlua::Error::runtime(format!("linux.proc_stat({pid}): {e}")))?;
        let stat = process
            .stat()
            .map_err(|e| mlua::Error::runtime(format!("linux.proc_stat({pid}): {e}")))?;

        let t = lua.create_table()?;
        t.set("pid", stat.pid)?;
        t.set("comm", stat.comm.as_str())?;
        t.set("state", stat.state.to_string())?;
        t.set("ppid", stat.ppid)?;
        t.set("pgrp", stat.pgrp)?;
        t.set("session", stat.session)?;
        t.set("utime", stat.utime)?;
        t.set("stime", stat.stime)?;
        t.set("vsize", stat.vsize)?;
        t.set("rss_pages", stat.rss)?;
        t.set("num_threads", stat.num_threads)?;
        t.set("starttime", stat.starttime)?;
        t.set("priority", stat.priority)?;
        t.set("nice", stat.nice)?;
        Ok(t)
    }

    fn proc_status(lua: &Lua, pid: i32) -> mlua::Result<mlua::Table> {
        let process = procfs::process::Process::new(pid)
            .map_err(|e| mlua::Error::runtime(format!("linux.proc_status({pid}): {e}")))?;
        let status = process
            .status()
            .map_err(|e| mlua::Error::runtime(format!("linux.proc_status({pid}): {e}")))?;

        let t = lua.create_table()?;
        t.set("name", status.name.as_str())?;
        t.set("state", status.state.as_str())?;
        t.set("pid", status.pid)?;
        t.set("ppid", status.ppid)?;
        t.set("uid", status.ruid)?;
        t.set("gid", status.rgid)?;
        t.set("threads", status.threads)?;
        if let Some(v) = status.vmrss {
            // procfs gives kB; convert to bytes
            t.set("vm_rss", v * 1024)?;
        }
        if let Some(v) = status.vmsize {
            t.set("vm_size", v * 1024)?;
        }
        Ok(t)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn new_lua_with_linux() -> mlua::Result<mlua::Lua> {
        let lua = mlua::Lua::new();
        register_linux(&lua)?;
        Ok(lua)
    }

    #[test]
    fn register_linux_smoke() {
        let lua = new_lua_with_linux().unwrap();
        let result: mlua::Table = lua
            .load("linux.kernel()")
            .eval()
            .expect("linux.kernel() failed");
        let version: String = result.get("version").expect("no version field");
        assert!(!version.is_empty(), "version string should be non-empty");
        let hostname: String = result.get("hostname").expect("no hostname field");
        assert!(!hostname.is_empty(), "hostname should be non-empty");
    }

    #[test]
    fn meminfo_total_positive() {
        let lua = new_lua_with_linux().unwrap();
        let total: u64 = lua
            .load("linux.meminfo().total")
            .eval()
            .expect("linux.meminfo().total failed");
        assert!(total > 0, "total memory should be positive");
    }

    #[test]
    fn loadavg_shape() {
        let lua = new_lua_with_linux().unwrap();
        let result: mlua::Table = lua
            .load("linux.loadavg()")
            .eval()
            .expect("linux.loadavg() failed");
        let _one: f64 = result.get("one").expect("loadavg: no 'one' field");
        let _five: f64 = result.get("five").expect("loadavg: no 'five' field");
        let _fifteen: f64 = result.get("fifteen").expect("loadavg: no 'fifteen' field");
    }

    #[test]
    fn cpu_percent_zero_for_same_snapshot() {
        let lua = new_lua_with_linux().unwrap();
        let total_pct: f64 = lua
            .load(
                r#"
                local snap = linux.cpu_stat()
                local r = linux.cpu_percent(snap, snap)
                return r.total_pct
                "#,
            )
            .eval()
            .expect("cpu_percent with same snapshot failed");
        assert!(
            (total_pct - 0.0).abs() < f64::EPSILON,
            "expected 0.0, got {total_pct}"
        );
    }

    #[test]
    fn proc_stat_self() {
        let lua = new_lua_with_linux().unwrap();
        let pid = std::process::id();
        let result: mlua::Table = lua
            .load(format!("linux.proc_stat({pid})"))
            .eval()
            .expect("linux.proc_stat(self) failed");

        let state: String = result.get("state").expect("no state field");
        assert!(
            state == "R" || state == "S" || state == "D",
            "unexpected state: {state}"
        );
        let comm: String = result.get("comm").expect("no comm field");
        assert!(!comm.is_empty(), "comm should be non-empty");
    }
}
