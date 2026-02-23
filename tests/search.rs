use assay::search::{BM25Index, SearchEngine};

#[test]
fn test_basic_ranking() {
    let mut idx = BM25Index::new();
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
fn test_field_boosting() {
    let mut idx = BM25Index::new();
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
    assert!(
        results[0].score > results[1].score,
        "doc_a score ({}) should be greater than doc_b score ({})",
        results[0].score,
        results[1].score
    );
}

#[test]
fn test_idf_document_frequency() {
    // "grafana" appears 5 times in doc A, 1 time in doc B
    // df should be 2 (distinct docs), NOT 6 (total occurrences)
    let mut idx = BM25Index::new();
    idx.add_document(
        "doc_a",
        &[(
            "description",
            "grafana grafana grafana grafana grafana",
            1.0,
        )],
    );
    idx.add_document("doc_b", &[("description", "grafana is great", 1.0)]);

    let results = idx.search("grafana", 10);
    assert_eq!(results.len(), 2, "both docs contain grafana");

    // With df=2 and N=2: IDF = ln((2-2+0.5)/(2+0.5)+1) = ln(0.5/2.5+1) = ln(1.2)
    // If df were incorrectly 6: IDF = ln((2-6+0.5)/(6+0.5)+1) which would be different
    // Both docs should have positive scores
    for result in &results {
        assert!(
            result.score > 0.0,
            "score for {} should be positive, got {}",
            result.id,
            result.score
        );
    }

    // Doc A has higher TF for "grafana" so it should score higher
    assert_eq!(
        results[0].id, "doc_a",
        "doc with more occurrences should rank higher"
    );
}

#[test]
fn test_multi_term_query() {
    let mut idx = BM25Index::new();
    idx.add_document(
        "both",
        &[("description", "grafana health check endpoint", 1.0)],
    );
    idx.add_document(
        "grafana_only",
        &[("description", "grafana dashboards and panels", 1.0)],
    );
    idx.add_document(
        "health_only",
        &[("description", "health monitoring system", 1.0)],
    );

    let results = idx.search("grafana health", 10);
    assert!(!results.is_empty(), "should return results");
    assert_eq!(
        results[0].id, "both",
        "doc containing both terms should rank highest"
    );
}

#[test]
fn test_empty_query() {
    let mut idx = BM25Index::new();
    idx.add_document("doc1", &[("description", "some content", 1.0)]);

    let results = idx.search("", 10);
    assert!(results.is_empty(), "empty query should return no results");
}

#[test]
fn test_empty_index() {
    let idx = BM25Index::new();
    let results = idx.search("grafana", 10);
    assert!(results.is_empty(), "empty index should return no results");
}

#[test]
fn test_no_matches() {
    let mut idx = BM25Index::new();
    idx.add_document("doc1", &[("description", "grafana monitoring", 1.0)]);

    let results = idx.search("nonexistent", 10);
    assert!(
        results.is_empty(),
        "query with no matches should return empty"
    );
}

#[test]
fn test_scores_non_negative() {
    let mut idx = BM25Index::new();
    idx.add_document(
        "doc1",
        &[
            ("keywords", "grafana monitoring dashboards", 3.0),
            ("description", "Grafana visualization tool", 1.0),
        ],
    );
    idx.add_document(
        "doc2",
        &[
            ("keywords", "prometheus metrics alerting", 3.0),
            ("description", "Prometheus monitoring system", 1.0),
        ],
    );
    idx.add_document(
        "doc3",
        &[
            ("keywords", "loki logging", 3.0),
            ("description", "Loki log aggregation", 1.0),
        ],
    );

    // Search for a common term
    let results = idx.search("monitoring", 10);
    for result in &results {
        assert!(
            result.score >= 0.0,
            "score for {} should be non-negative, got {}",
            result.id,
            result.score
        );
    }

    // Search for a rare term
    let results = idx.search("loki", 10);
    for result in &results {
        assert!(
            result.score >= 0.0,
            "score for {} should be non-negative, got {}",
            result.id,
            result.score
        );
    }
}

#[test]
fn test_limit() {
    let mut idx = BM25Index::new();
    idx.add_document("doc1", &[("description", "search engine ranking", 1.0)]);
    idx.add_document("doc2", &[("description", "search optimization tips", 1.0)]);
    idx.add_document(
        "doc3",
        &[("description", "search algorithms explained", 1.0)],
    );

    let results = idx.search("search", 2);
    assert!(
        results.len() <= 2,
        "should return at most 2 results, got {}",
        results.len()
    );
}
