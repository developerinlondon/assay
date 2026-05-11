//! Regression guards on the repo's Dockerfiles.
//!
//! All published images are `FROM scratch` with just the relevant
//! binary + a CA bundle:
//!
//! - `ghcr.io/developerinlondon/assay`        — runtime, built from `Dockerfile`
//! - `ghcr.io/developerinlondon/assay-engine` — server,  built from `Dockerfile.assay-engine`
//! - `ghcr.io/developerinlondon/assay:<v>-sh` — runtime + busybox `/bin/sh`,
//!   built from `Dockerfile.sh`. Still `FROM scratch`; only present so
//!   the image works inside GitLab CI's `sh -c "$script"` wrapper.
//!
//! Scratch keeps both images ~10 MB each, reduces the CVE surface to
//! assay's own supply chain, and makes every downstream image that
//! `FROM`'s one of them inherit the same minimal footprint.
//!
//! In Feb 2026 (commit 5c43c83) the runtime stage of `Dockerfile` was
//! briefly flipped to `FROM alpine:3.21` to accommodate a single
//! downstream Deployment that wrapped assay in `command: ["/bin/sh",
//! "-c", …]`. That wrapper has since been removed and assay's stdlib
//! covers the sed/awk use cases that once required a shell. These
//! tests prevent silently reintroducing that regression on either image.
//!
//! Each test runs twice — once per Dockerfile — via a `for` over a
//! small table. Parametrising keeps the assertion bodies identical so
//! a new image variant just means adding a row, not forking the test.

/// Table of `(dockerfile-path-relative-to-repo-root, expected-entrypoint-binary-name)`.
///
/// `CARGO_MANIFEST_DIR` is `crates/assay/` post-0.13.0, so each path is prefixed
/// with `../../` to reach the repo root.
const DOCKERFILES: &[(&str, &str)] = &[
    ("../../Dockerfile", "/assay"),
    ("../../Dockerfile.assay-engine", "/assay-engine"),
    ("../../Dockerfile.sh", "/assay"),
];

fn read_dockerfile(rel_path: &str) -> String {
    let full = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), rel_path);
    std::fs::read_to_string(&full)
        .unwrap_or_else(|e| panic!("Dockerfile must exist at {full}: {e}"))
}

#[test]
fn dockerfile_runtime_stage_is_from_scratch() {
    for (path, _entrypoint) in DOCKERFILES {
        let content = read_dockerfile(path);

        // The runtime stage is the last `FROM` line in a multi-stage
        // build (earlier FROMs are builder/intermediate stages with `AS`).
        // `rfind` on the DoubleEndedIterator is a one-pass reverse
        // search — cheaper than `filter(..).next_back()` (clippy 1.95
        // surfaces this via `clippy::filter_next`) and `Iterator::last`
        // would walk the whole stream forwards.
        let last_from = content
            .lines()
            .map(str::trim_start)
            .rfind(|l| l.starts_with("FROM "))
            .unwrap_or_else(|| panic!("{path} must contain at least one FROM line"));

        assert_eq!(
            last_from.trim(),
            "FROM scratch",
            "{path} runtime stage must be `FROM scratch` — got `{last_from}`.\n\
             This guard exists because commit 5c43c83 (Feb 2026) once flipped \
             the assay runtime to alpine:3.21 and nobody noticed for weeks. \
             Keeping the runtime scratch preserves the ~10 MB image size and \
             excludes the entire Alpine CVE feed from assay's supply chain."
        );
    }
}

#[test]
fn dockerfile_copies_ca_bundle() {
    for (path, _entrypoint) in DOCKERFILES {
        let content = read_dockerfile(path);

        assert!(
            content.contains("ca-certificates.crt"),
            "{path} must COPY a CA bundle into the scratch image at \
             /etc/ssl/certs/ca-certificates.crt — without it, every HTTPS \
             call (reqwest, sqlx TLS, WebSockets) fails with \
             'certificate verify failed: unable to get local issuer \
             certificate'. The pre-Feb-2026 Dockerfile had this line; \
             the Alpine regression silently dropped it."
        );
    }
}

#[test]
fn dockerfile_entrypoint_uses_absolute_path() {
    for (path, entrypoint) in DOCKERFILES {
        let content = read_dockerfile(path);

        // On a scratch image there is no $PATH resolution (no shell, no
        // system PATH). The ENTRYPOINT must be an absolute filesystem
        // path — `/assay`, not bare `assay`.
        let expected = format!(r#"ENTRYPOINT ["{entrypoint}"]"#);
        assert!(
            content.contains(&expected),
            "{path} ENTRYPOINT must be `{expected}` because scratch images \
             have no $PATH resolution. Content did not include that line."
        );
    }
}
