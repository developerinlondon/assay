//! Shared catalog of assay's mutating builtin surface. Both the read-only
//! gate (`readonly.rs`) and the approval gate (`approval.rs`) consume this
//! one list so the two modes classify the same operations as mutating.

/// Tables whose entire function surface is gated.
pub(crate) const BLOCKED_TABLES: &[&str] = &["shell", "process", "machinectl"];

/// Individual functions gated inside otherwise-usable tables.
pub(crate) const BLOCKED_FUNCTIONS: &[&str] = &[
    "http.post",
    "http.put",
    "http.patch",
    "http.delete",
    "http.serve",
    "http.serve_with_extra",
    "http.download",
    "ws.connect",
    "fs.write",
    "fs.write_bytes",
    "fs.remove",
    "fs.rename",
    "fs.copy",
    "fs.chmod",
    "fs.mkdir",
    "fs.tempdir",
    "fs.sub_in_file",
    "env.set",
    "db.execute",
    "oci.copy",
    "oci.tag",
    "oci.mutate",
    "systemd.start",
    "systemd.stop",
    "systemd.restart",
    "systemd.reload",
    "systemd.unit_action",
    "systemd.machine_start",
    "systemd.machine_poweroff",
    "systemd.machine_reboot",
    "systemd.machine_terminate",
    "systemd.machine_exec",
    "apt.update",
    "apt.install",
    "apt.remove",
    "apt.add_source",
    "compress.untar",
    "tar.create",
    "tar.extract",
    "io.popen",
];

/// `http.client(...)` wrappers route every verb through
/// `http._client_request(ud, method, ...)`. Only `get` is a read; every
/// other verb is a mutating (gated) request.
pub(crate) fn is_gated_http_verb(method: &str) -> bool {
    method != "get"
}
