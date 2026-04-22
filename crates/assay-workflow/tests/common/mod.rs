pub mod harness;
// Each integration test binary sees this module independently. Some tests
// use the whole re-export (`smoke_backends`), others only pull specific
// items (`postgres_store`) — silence dead-import warnings in the latter.
#[allow(dead_code, unused_imports)]
pub use harness::*;
