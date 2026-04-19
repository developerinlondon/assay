//! Regression guards on the root `Dockerfile`.
//!
//! The published `ghcr.io/developerinlondon/assay` image is meant to
//! be a scratch image containing only the assay binary and a CA bundle.
//! That keeps it ~10 MB (vs ~25 MB on Alpine), reduces the CVE surface
//! to assay's own supply chain, and makes every downstream image that
//! `FROM`'s assay inherit the same minimal footprint.
//!
//! In Feb 2026 (commit 5c43c83) the runtime stage was briefly flipped
//! to `FROM alpine:3.21` to accommodate a single downstream Deployment
//! that wrapped assay in `command: ["/bin/sh", "-c", …]`. That wrapper
//! has since been removed and assay's stdlib covers the sed/awk use
//! cases that once required a shell. These tests prevent silently
//! reintroducing the regression.

#[test]
fn dockerfile_runtime_stage_is_from_scratch() {
    let content =
        std::fs::read_to_string("Dockerfile").expect("Dockerfile must exist at repo root");

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
        .expect("Dockerfile must contain at least one FROM line");

    assert_eq!(
        last_from.trim(),
        "FROM scratch",
        "Dockerfile runtime stage must be `FROM scratch` — got `{last_from}`.\n\
         This guard exists because commit 5c43c83 (Feb 2026) once flipped \
         the runtime to alpine:3.21 and nobody noticed for weeks. Keeping \
         the runtime scratch preserves the ~10 MB image size and excludes \
         the entire Alpine CVE feed from assay's supply chain."
    );
}

#[test]
fn dockerfile_copies_ca_bundle() {
    let content =
        std::fs::read_to_string("Dockerfile").expect("Dockerfile must exist at repo root");

    assert!(
        content.contains("ca-certificates.crt"),
        "Dockerfile must COPY a CA bundle into the scratch image at \
         /etc/ssl/certs/ca-certificates.crt — without it, every HTTPS \
         call from assay (reqwest, sqlx TLS, WebSockets) fails with \
         'certificate verify failed: unable to get local issuer \
         certificate'. The pre-Feb-2026 Dockerfile had this line; \
         the Alpine regression silently dropped it."
    );
}

#[test]
fn dockerfile_entrypoint_uses_absolute_path() {
    let content =
        std::fs::read_to_string("Dockerfile").expect("Dockerfile must exist at repo root");

    // On a scratch image there is no $PATH resolution (no shell, no
    // system PATH). The ENTRYPOINT must be an absolute filesystem
    // path — `/assay`, not bare `assay`.
    assert!(
        content.contains(r#"ENTRYPOINT ["/assay"]"#),
        "Dockerfile ENTRYPOINT must be an absolute path (`/assay`) \
         because scratch images have no $PATH resolution. Found content \
         did not include the absolute-path ENTRYPOINT."
    );
}
