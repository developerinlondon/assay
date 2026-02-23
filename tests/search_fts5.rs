#![cfg(feature = "db")]

use assay::search::SearchEngine;
use assay::search_fts5::FTS5Index;

#[test]
fn test_fts5_basic_ranking() {
    let mut idx = FTS5Index::new();
    idx.add_document(
        "grafana",
        &[
            ("keywords", "grafana monitoring", 3.0),
            ("description", "Grafana dashboards and visualization", 1.0),
        ],
    );
    idx.add_document(
        "prometheus",
        &[
            ("keywords", "prometheus metrics", 3.0),
            ("description", "Prometheus monitoring and alerting", 1.0),
        ],
    );

    let results = idx.search("grafana", 10);
    assert!(!results.is_empty(), "should return results");
    assert_eq!(results[0].id, "grafana", "grafana should rank first");
}

#[test]
fn test_fts5_field_boosting() {
    let mut idx = FTS5Index::new();
    // Doc A: "monitoring" in keywords (weight 3.0)
    idx.add_document("doc_a", &[("keywords", "monitoring", 3.0)]);
    // Doc B: "monitoring" in description (weight 1.0)
    idx.add_document("doc_b", &[("description", "monitoring", 1.0)]);

    let results = idx.search("monitoring", 10);
    assert!(results.len() >= 2, "should return both docs");
    assert_eq!(
        results[0].id, "doc_a",
        "keyword match (weight 3.0) should outrank description match (weight 1.0)"
    );
}

#[test]
fn test_fts5_empty_query() {
    let mut idx = FTS5Index::new();
    idx.add_document("doc1", &[("description", "some content", 1.0)]);

    let results = idx.search("", 10);
    assert!(results.is_empty(), "empty query should return no results");
}

#[test]
fn test_fts5_no_documents() {
    let idx = FTS5Index::new();
    let results = idx.search("grafana", 10);
    assert!(results.is_empty(), "empty index should return no results");
}

#[test]
fn test_fts5_trait_object() {
    let mut idx = FTS5Index::new();
    idx.add_document("doc1", &[("name", "grafana", 2.0)]);

    let engine: Box<dyn SearchEngine> = Box::new(idx);
    let results = engine.search("grafana", 10);
    assert!(!results.is_empty(), "trait object search should work");
    assert_eq!(results[0].id, "doc1");
}

#[test]
fn test_fts5_special_chars() {
    let mut idx = FTS5Index::new();
    idx.add_document("doc1", &[("description", "hello world test", 1.0)]);

    // These should not panic — special FTS5 chars are sanitized
    let results = idx.search("hello OR world", 10);
    assert!(results.len() <= 10);

    let results = idx.search("test*", 10);
    assert!(results.len() <= 10);

    let results = idx.search("\"quoted phrase\"", 10);
    assert!(results.len() <= 10);

    let results = idx.search("col:value", 10);
    assert!(results.len() <= 10);

    let results = idx.search("NOT something", 10);
    assert!(results.len() <= 10);
}
