mod common;

use std::io::Write;

use common::run_lua;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const PACKAGES_FIXTURE: &str = "\
Package: tailscale
Version: 1.84.3
Architecture: amd64
Depends: foo

Package: tailscale
Version: 1.84.2
Architecture: amd64

Package: foo
Version: 1.0
Architecture: amd64
";

fn gzip(body: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(body).unwrap();
    e.finish().unwrap()
}

#[tokio::test]
async fn test_apt_packages_gz() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/dists/noble/main/binary-amd64/Packages.gz"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(gzip(PACKAGES_FIXTURE.as_bytes())))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local apt = require("assay.apt")
        local idx = apt.packages({{
            base_url  = "{}",
            dist      = "noble",
            component = "main",
            arch      = "amd64",
        }})

        local pkg = idx:find("tailscale")
        assert.not_nil(pkg)
        assert.eq(pkg.name, "tailscale")
        assert.eq(pkg.version, "1.84.3")
        assert.eq(pkg.versions[1], "1.84.3")
        assert.eq(pkg.versions[2], "1.84.2")
        assert.eq(pkg.architecture, "amd64")
        assert.eq(pkg.depends, "foo")

        local foo = idx:find("foo")
        assert.eq(foo.version, "1.0")

        local missing = idx:find("nonexistent")
        assert.eq(missing, nil)
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_apt_packages_plain_fallback() {
    let server = MockServer::start().await;
    // .gz, .xz, .zst all 404 — should fall through to plain Packages.
    Mock::given(method("GET"))
        .and(path("/dists/noble/main/binary-amd64/Packages.gz"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/dists/noble/main/binary-amd64/Packages.xz"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/dists/noble/main/binary-amd64/Packages.zst"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/dists/noble/main/binary-amd64/Packages"))
        .respond_with(ResponseTemplate::new(200).set_body_string(PACKAGES_FIXTURE))
        .mount(&server)
        .await;

    let script = format!(
        r#"
        local apt = require("assay.apt")
        local idx = apt.packages({{
            base_url  = "{}",
            dist      = "noble",
            component = "main",
            arch      = "amd64",
        }})

        assert.eq(idx:find("tailscale").version, "1.84.3")
        assert.eq(idx:find("foo").version, "1.0")
        "#,
        server.uri()
    );
    run_lua(&script).await.unwrap();
}

#[tokio::test]
async fn test_apt_packages_all_404_errors() {
    let server = MockServer::start().await;
    for name in ["Packages.gz", "Packages.xz", "Packages.zst", "Packages"] {
        Mock::given(method("GET"))
            .and(path(format!("/dists/noble/main/binary-amd64/{name}")))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
    }

    let script = format!(
        r#"
        local apt = require("assay.apt")
        apt.packages({{
            base_url  = "{}",
            dist      = "noble",
            component = "main",
            arch      = "amd64",
        }})
        "#,
        server.uri()
    );
    let err = run_lua(&script).await.unwrap_err();
    assert!(
        err.to_string().contains("apt.packages"),
        "expected apt.packages error, got: {err}"
    );
}
