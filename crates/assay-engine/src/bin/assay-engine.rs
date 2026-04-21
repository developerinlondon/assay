//! Standalone assay-engine binary. Full wiring lands in plan 12e Phase 8.

fn main() {
    eprintln!(
        "assay-engine {} — binary wiring deferred to phase 8 (see plan 12)",
        env!("CARGO_PKG_VERSION")
    );
    std::process::exit(2);
}
