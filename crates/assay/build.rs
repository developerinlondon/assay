//! Invalidate the crate whenever an embedded stdlib file changes.
//!
//! `include_dir!("$CARGO_MANIFEST_DIR/stdlib")` embeds the stdlib at compile
//! time, but Cargo does not track proc-macro file reads, so a stdlib-only
//! change (no source `.rs` change) would reuse a cached build with a stale
//! embed. Emitting `rerun-if-changed` for the tree fixes that on every build
//! cache, including persistent CI caches.

use std::path::Path;

fn track(dir: &Path) {
    println!("cargo:rerun-if-changed={}", dir.display());
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            track(&path);
        } else {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

fn main() {
    track(Path::new("stdlib"));
}
