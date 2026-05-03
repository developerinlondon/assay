//! Extract + atomic-install tests for `assay install`.
//!
//! Phase 2 of plan 21. Builds tar.gz archives in memory, writes them to
//! a temp cache dir, then drives the install_* functions and asserts on
//! the resulting filesystem layout / file modes.

use std::io::Write;
use std::path::Path;

use assay::install::extract::{ExtractError, install_extension_binary, install_lib_tree};
use tempfile::TempDir;

// --- archive builders -------------------------------------------------

fn build_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    {
        let mut tb = tar::Builder::new(&mut gz);
        for (name, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_path(name).unwrap();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tb.append(&header, *bytes).unwrap();
        }
        tb.finish().unwrap();
    }
    gz.finish().unwrap()
}

fn write_archive(dir: &Path, name: &str, entries: &[(&str, &[u8])]) -> std::path::PathBuf {
    let p = dir.join(name);
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(&build_tar_gz(entries)).unwrap();
    p
}

// --- install_extension_binary ----------------------------------------

#[test]
fn extension_extracts_named_member_and_chmods_executable() {
    let cache = TempDir::new().unwrap();
    let bin_dir = TempDir::new().unwrap();

    let body = b"fake assay-engine binary";
    let archive = write_archive(
        cache.path(),
        "assay-engine-0.4.1-x86_64.tar.gz",
        &[("assay-engine", body)],
    );

    let installed = install_extension_binary(&archive, bin_dir.path(), "assay-engine").unwrap();

    assert_eq!(installed, bin_dir.path().join("assay-engine"));
    assert_eq!(std::fs::read(&installed).unwrap(), body);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&installed).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }
}

#[test]
fn extension_finds_member_when_archive_has_multiple_files() {
    let cache = TempDir::new().unwrap();
    let bin_dir = TempDir::new().unwrap();
    let archive = write_archive(
        cache.path(),
        "engine.tar.gz",
        &[
            ("README.md", b"docs"),
            ("LICENSE", b"apache"),
            ("assay-engine", b"the actual binary"),
        ],
    );

    let installed = install_extension_binary(&archive, bin_dir.path(), "assay-engine").unwrap();
    assert_eq!(std::fs::read(&installed).unwrap(), b"the actual binary");
}

#[test]
fn extension_errors_when_named_member_not_found() {
    let cache = TempDir::new().unwrap();
    let bin_dir = TempDir::new().unwrap();
    let archive = write_archive(
        cache.path(),
        "engine.tar.gz",
        &[("something-else", b"nope")],
    );

    let err = install_extension_binary(&archive, bin_dir.path(), "assay-engine").unwrap_err();
    assert!(matches!(err, ExtractError::BinaryMemberNotFound { .. }));
    let msg = err.to_string();
    assert!(msg.contains("assay-engine"));
    // and nothing was written
    assert!(!bin_dir.path().join("assay-engine").exists());
    assert!(!bin_dir.path().join(".assay-engine.tmp").exists());
}

#[test]
fn extension_replaces_existing_file_atomically() {
    let cache = TempDir::new().unwrap();
    let bin_dir = TempDir::new().unwrap();

    // pre-existing binary in the bin dir
    let existing = bin_dir.path().join("assay-engine");
    std::fs::write(&existing, b"old binary").unwrap();

    let archive = write_archive(
        cache.path(),
        "engine.tar.gz",
        &[("assay-engine", b"new binary")],
    );

    install_extension_binary(&archive, bin_dir.path(), "assay-engine").unwrap();
    assert_eq!(std::fs::read(&existing).unwrap(), b"new binary");
    // tmp cleaned up
    assert!(!bin_dir.path().join(".assay-engine.tmp").exists());
}

#[test]
fn extension_creates_bin_dir_when_missing() {
    let cache = TempDir::new().unwrap();
    let bin_root = TempDir::new().unwrap();
    let bin_dir = bin_root.path().join("nested/dir");
    assert!(!bin_dir.exists());

    let archive = write_archive(
        cache.path(),
        "engine.tar.gz",
        &[("assay-engine", b"bin")],
    );

    install_extension_binary(&archive, &bin_dir, "assay-engine").unwrap();
    assert!(bin_dir.join("assay-engine").exists());
}

// --- install_lib_tree -------------------------------------------------

#[test]
fn lib_extracts_tarball_tree_into_named_subdir() {
    let cache = TempDir::new().unwrap();
    let lib_dir = TempDir::new().unwrap();

    let archive = write_archive(
        cache.path(),
        "assay-lib-hostops-0.1.0.tar.gz",
        &[
            ("mount.lua", b"return {}"),
            ("pages/dashboard.lua", b"-- dash"),
            ("VERSION", b"0.1.0\n"),
        ],
    );

    let installed = install_lib_tree(&archive, lib_dir.path(), "hostops").unwrap();
    assert_eq!(installed, lib_dir.path().join("hostops"));
    assert_eq!(
        std::fs::read(installed.join("mount.lua")).unwrap(),
        b"return {}"
    );
    assert_eq!(
        std::fs::read(installed.join("pages/dashboard.lua")).unwrap(),
        b"-- dash"
    );
    assert_eq!(std::fs::read(installed.join("VERSION")).unwrap(), b"0.1.0\n");
}

#[test]
fn lib_replaces_existing_tree() {
    let cache = TempDir::new().unwrap();
    let lib_dir = TempDir::new().unwrap();
    let target = lib_dir.path().join("hostops");

    // Pre-existing lib tree with a stale file the new tarball doesn't include.
    std::fs::create_dir_all(target.join("pages")).unwrap();
    std::fs::write(target.join("STALE_FILE"), b"obsolete").unwrap();
    std::fs::write(target.join("mount.lua"), b"-- old").unwrap();

    let archive = write_archive(
        cache.path(),
        "assay-lib-hostops-0.1.1.tar.gz",
        &[("mount.lua", b"-- new"), ("VERSION", b"0.1.1\n")],
    );

    install_lib_tree(&archive, lib_dir.path(), "hostops").unwrap();

    assert_eq!(std::fs::read(target.join("mount.lua")).unwrap(), b"-- new");
    // stale file from prior install must be gone — we replace the whole tree.
    assert!(!target.join("STALE_FILE").exists());
    // staging dir cleaned up
    assert!(!lib_dir.path().join(".hostops.new").exists());
}

#[test]
fn lib_recovers_from_leftover_staging_dir() {
    let cache = TempDir::new().unwrap();
    let lib_dir = TempDir::new().unwrap();

    // Simulate a previously-crashed install: stale staging dir present.
    let stale_staging = lib_dir.path().join(".hostops.new");
    std::fs::create_dir_all(&stale_staging).unwrap();
    std::fs::write(stale_staging.join("garbage.lua"), b"crashed").unwrap();

    let archive = write_archive(
        cache.path(),
        "assay-lib-hostops-0.1.0.tar.gz",
        &[("mount.lua", b"return {}")],
    );

    let installed = install_lib_tree(&archive, lib_dir.path(), "hostops").unwrap();
    assert!(installed.join("mount.lua").exists());
    // and the staging dir is gone after a successful install
    assert!(!stale_staging.exists());
}

#[test]
fn lib_creates_lib_dir_when_missing() {
    let cache = TempDir::new().unwrap();
    let lib_root = TempDir::new().unwrap();
    let lib_dir = lib_root.path().join("opt/assay/libs");
    assert!(!lib_dir.exists());

    let archive = write_archive(
        cache.path(),
        "assay-lib-hostops-0.1.0.tar.gz",
        &[("mount.lua", b"return {}")],
    );

    install_lib_tree(&archive, &lib_dir, "hostops").unwrap();
    assert!(lib_dir.join("hostops/mount.lua").exists());
}
