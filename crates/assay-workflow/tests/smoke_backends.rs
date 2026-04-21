mod common;
use common::Backend;
use rstest::rstest;

#[rstest]
#[cfg_attr(feature = "backend-postgres", case::pg(Backend::Postgres))]
#[cfg_attr(feature = "backend-sqlite", case::sqlite(Backend::Sqlite))]
#[cfg_attr(feature = "backend-surrealdb", case::surreal(Backend::Surreal))]
#[tokio::test(flavor = "multi_thread")]
async fn namespace_roundtrip(#[case] backend: Backend) {
    let h = backend.setup().await.expect("setup");
    // "main" namespace is created during harness setup — it must appear in the list.
    let list = h.list_namespaces().await.unwrap();
    assert!(list.iter().any(|n| n.name == "main"));
}
