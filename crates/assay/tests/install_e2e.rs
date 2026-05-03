//! End-to-end test for `assay install`.
//!
//! Phase 2 of plan 21. Boots a `wiremock` server serving fixture
//! tarballs (one extension binary + one lib tree), writes a real
//! `Manifest.lua` referencing them via per-dep `source` overrides, and
//! drives `assay::install::execute` to completion. Asserts on:
//!
//! - extracted extension binary at `<bin_dir>/<name>` with mode 0755
//! - extracted lib tree at `<lib_dir>/<name>/...`
//! - `Manifest.lock` written next to the input manifest, valid Lua

use std::io::Write;
use std::path::Path;

use assay::install::{InstallArgs, execute};
use data_encoding::HEXLOWER;
use mlua::{Lua, Table};
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sha(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    HEXLOWER.encode(&h.finalize())
}

fn build_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    {
        let mut tb = tar::Builder::new(&mut gz);
        for (name, bytes) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_path(name).unwrap();
            h.set_size(bytes.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tb.append(&h, *bytes).unwrap();
        }
        tb.finish().unwrap();
    }
    gz.finish().unwrap()
}

async fn mount(server: &MockServer, route: &str, body: Vec<u8>) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .mount(server)
        .await;
}

fn write_manifest(dir: &Path, content: &str) -> std::path::PathBuf {
    let p = dir.join("Manifest.lua");
    let mut f = std::fs::File::create(&p).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    p
}

#[tokio::test]
async fn full_install_flow_extracts_bin_and_lib_and_writes_lock() {
    let server = MockServer::start().await;

    let bin_bytes = b"fake-engine-binary";
    let bin_archive = build_tar_gz(&[("assay-engine", bin_bytes)]);
    let bin_sha = sha(&bin_archive);
    mount(&server, "/engine.tar.gz", bin_archive).await;

    let lib_archive = build_tar_gz(&[
        ("mount.lua", b"return {}"),
        ("pages/dashboard.lua", b"-- dash"),
        ("VERSION", b"0.1.0\n"),
    ]);
    let lib_sha = sha(&lib_archive);
    mount(&server, "/hostops.tar.gz", lib_archive).await;

    let arch = std::env::consts::ARCH;
    let workspace = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();
    let bin_dir = TempDir::new().unwrap();
    let lib_dir = TempDir::new().unwrap();

    let manifest_text = format!(
        r#"return {{
            assay = "0.15.6",
            extensions = {{
                {{ name = "assay-engine", version = "0.4.1",
                   sha256 = {{ {arch} = "{bin_sha}" }},
                   source = "{server_uri}/engine.tar.gz" }},
            }},
            libs = {{
                {{ name = "hostops", version = "0.1.0", sha256 = "{lib_sha}",
                   source = "{server_uri}/hostops.tar.gz" }},
            }},
        }}"#,
        arch = arch,
        bin_sha = bin_sha,
        lib_sha = lib_sha,
        server_uri = server.uri(),
    );
    let manifest_path = write_manifest(workspace.path(), &manifest_text);

    let args = InstallArgs {
        manifest: manifest_path.clone(),
        cache_dir: Some(cache_dir.path().to_path_buf()),
        bin_dir: Some(bin_dir.path().to_path_buf()),
        lib_dir: Some(lib_dir.path().to_path_buf()),
        offline: false,
        dry_run: false,
        no_progress: true,
    };
    execute(args).await.expect("install should succeed");

    // Extension binary in place + executable.
    let installed_bin = bin_dir.path().join("assay-engine");
    assert_eq!(std::fs::read(&installed_bin).unwrap(), bin_bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&installed_bin).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    // Lib tree in place.
    let lib_root = lib_dir.path().join("hostops");
    assert!(lib_root.is_dir());
    assert_eq!(
        std::fs::read(lib_root.join("mount.lua")).unwrap(),
        b"return {}"
    );
    assert_eq!(
        std::fs::read(lib_root.join("pages/dashboard.lua")).unwrap(),
        b"-- dash"
    );

    // Cache populated.
    assert!(
        cache_dir
            .path()
            .join(format!("assay-engine-0.4.1-{arch}.tar.gz"))
            .exists()
    );
    assert!(
        cache_dir
            .path()
            .join("assay-lib-hostops-0.1.0.tar.gz")
            .exists()
    );

    // Lockfile written next to the manifest, valid Lua, expected shape.
    let lock_path = workspace.path().join("Manifest.lock");
    let lock_src = std::fs::read_to_string(&lock_path).unwrap();
    let lua = Lua::new();
    let lock: Table = lua.load(&lock_src).eval().unwrap();
    assert_eq!(lock.get::<String>("assay").unwrap(), "0.15.6");
    let exts: Table = lock.get("extensions").unwrap();
    let e1: Table = exts.get(1).unwrap();
    assert_eq!(e1.get::<String>("name").unwrap(), "assay-engine");
    assert!(
        e1.get::<String>("url")
            .unwrap()
            .ends_with("/engine.tar.gz")
    );
    let libs: Table = lock.get("libs").unwrap();
    let l1: Table = libs.get(1).unwrap();
    assert_eq!(l1.get::<String>("sha256").unwrap(), lib_sha);
}

#[tokio::test]
async fn dry_run_does_not_write_anything() {
    let server = MockServer::start().await; // no mocks; dry-run shouldn't hit it
    let workspace = TempDir::new().unwrap();
    let cache_dir = TempDir::new().unwrap();
    let bin_dir = TempDir::new().unwrap();
    let lib_dir = TempDir::new().unwrap();

    let arch = std::env::consts::ARCH;
    let manifest_text = format!(
        r#"return {{
            extensions = {{
                {{ name = "x", version = "1",
                   sha256 = {{ {arch} = "abc" }},
                   source = "{}/x.tar.gz" }},
            }},
        }}"#,
        server.uri(),
    );
    let manifest_path = write_manifest(workspace.path(), &manifest_text);

    let args = InstallArgs {
        manifest: manifest_path.clone(),
        cache_dir: Some(cache_dir.path().to_path_buf()),
        bin_dir: Some(bin_dir.path().to_path_buf()),
        lib_dir: Some(lib_dir.path().to_path_buf()),
        offline: false,
        dry_run: true,
        no_progress: true,
    };
    execute(args).await.expect("dry-run should succeed");

    // Nothing fetched, nothing extracted, no lock.
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(std::fs::read_dir(cache_dir.path()).unwrap().next().is_none());
    assert!(std::fs::read_dir(bin_dir.path()).unwrap().next().is_none());
    assert!(std::fs::read_dir(lib_dir.path()).unwrap().next().is_none());
    assert!(!workspace.path().join("Manifest.lock").exists());
}

#[tokio::test]
async fn errors_propagate_when_one_dep_fails() {
    let server = MockServer::start().await;
    // One dep available, one returns 404.
    let good_archive = build_tar_gz(&[("good-bin", b"good")]);
    let good_sha = sha(&good_archive);
    mount(&server, "/good.tar.gz", good_archive).await;
    Mock::given(method("GET"))
        .and(path("/missing.tar.gz"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let arch = std::env::consts::ARCH;
    let workspace = TempDir::new().unwrap();
    let cache = TempDir::new().unwrap();
    let bin = TempDir::new().unwrap();
    let lib = TempDir::new().unwrap();

    let manifest_text = format!(
        r#"return {{
            extensions = {{
                {{ name = "good-bin", version = "1",
                   sha256 = {{ {arch} = "{good_sha}" }},
                   source = "{base}/good.tar.gz" }},
                {{ name = "missing", version = "1",
                   sha256 = {{ {arch} = "0000" }},
                   source = "{base}/missing.tar.gz" }},
            }},
        }}"#,
        arch = arch,
        good_sha = good_sha,
        base = server.uri(),
    );
    let manifest_path = write_manifest(workspace.path(), &manifest_text);

    let args = InstallArgs {
        manifest: manifest_path.clone(),
        cache_dir: Some(cache.path().to_path_buf()),
        bin_dir: Some(bin.path().to_path_buf()),
        lib_dir: Some(lib.path().to_path_buf()),
        offline: false,
        dry_run: false,
        no_progress: true,
    };
    let err = execute(args).await.unwrap_err();
    assert!(matches!(
        err,
        assay::install::InstallError::FetchFailed { count: 1 }
    ));

    // The good dep's cache entry was preserved (so re-runs resume),
    // but no extraction happened (we abort on fetch failure before
    // extract).
    assert!(cache.path().join("good-bin-1-".to_string() + arch + ".tar.gz").exists());
    assert!(!bin.path().join("good-bin").exists());
    assert!(!workspace.path().join("Manifest.lock").exists());
}
