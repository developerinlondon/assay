use std::collections::HashMap;

/// Result of a search query, containing the document ID and relevance score.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub id: String,
    pub score: f64,
}

/// Trait for search backends. Implemented by `BM25Index` (in-memory fallback)
/// and can be implemented by FTS5-backed stores when the `db` feature is enabled.
pub trait SearchEngine {
    /// Add a document with weighted fields.
    /// Each field is `(field_name, field_value, field_weight)`.
    fn add_document(&mut self, id: &str, fields: &[(&str, &str, f64)]);

    /// Search for documents matching the query, returning at most `limit` results
    /// sorted by descending relevance score.
    fn search(&self, query: &str, limit: usize) -> Vec<SearchResult>;
}

/// Per-field data stored for a single document.
#[derive(Debug)]
struct FieldData {
    tokens: Vec<String>,
    weight: f64,
}

/// Per-document data.
#[derive(Debug)]
struct Document {
    fields: HashMap<String, FieldData>,
}

/// Zero-dependency BM25 search index.
///
/// Uses the Okapi BM25 ranking function with configurable field weights.
/// This is the fallback backend when no database/FTS5 is available.
#[derive(Debug)]
pub struct BM25Index {
    documents: HashMap<String, Document>,
    /// For each field name, tracks the total token count across all documents
    /// (used to compute average field length).
    field_total_tokens: HashMap<String, usize>,
    /// For each field name, tracks how many documents have that field.
    field_doc_count: HashMap<String, usize>,
    /// Document frequency: for each term, the set of document IDs containing it.
    doc_freq: HashMap<String, Vec<String>>,
}

impl Default for BM25Index {
    fn default() -> Self {
        Self::new()
    }
}

impl BM25Index {
    /// BM25 term frequency saturation parameter.
    const K1: f64 = 1.2;
    /// BM25 document length normalization parameter.
    const B: f64 = 0.75;

    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
            field_total_tokens: HashMap::new(),
            field_doc_count: HashMap::new(),
            doc_freq: HashMap::new(),
        }
    }

    /// Tokenize text: split on non-alphanumeric/underscore, lowercase, filter len <= 1.
    fn tokenize(text: &str) -> Vec<String> {
        text.split(|c: char| !c.is_alphanumeric() && c != '_')
            .map(|s| s.to_lowercase())
            .filter(|s| s.len() > 1)
            .collect()
    }

    /// Compute IDF for a term.
    /// IDF(t) = ln((N - df + 0.5) / (df + 0.5) + 1.0)
    fn idf(&self, term: &str) -> f64 {
        let n = self.documents.len() as f64;
        let df = self.doc_freq.get(term).map_or(0, |docs| docs.len()) as f64;
        f64::ln((n - df + 0.5) / (df + 0.5) + 1.0)
    }

    /// Average field length for a given field name.
    fn avg_field_len(&self, field_name: &str) -> f64 {
        let total = *self.field_total_tokens.get(field_name).unwrap_or(&0) as f64;
        let count = *self.field_doc_count.get(field_name).unwrap_or(&0) as f64;
        if count == 0.0 {
            return 0.0;
        }
        total / count
    }
}

impl SearchEngine for BM25Index {
    fn add_document(&mut self, id: &str, fields: &[(&str, &str, f64)]) {
        let mut doc_fields = HashMap::new();
        let mut seen_terms: HashMap<String, bool> = HashMap::new();

        for &(field_name, field_value, weight) in fields {
            let tokens = Self::tokenize(field_value);

            // Track field statistics
            *self
                .field_total_tokens
                .entry(field_name.to_string())
                .or_insert(0) += tokens.len();
            *self
                .field_doc_count
                .entry(field_name.to_string())
                .or_insert(0) += 1;

            // Track unique terms in this document for document frequency
            for token in &tokens {
                seen_terms.entry(token.clone()).or_insert(true);
            }

            doc_fields.insert(field_name.to_string(), FieldData { tokens, weight });
        }

        // Update document frequency: each term gets this doc ID once
        for term in seen_terms.keys() {
            self.doc_freq
                .entry(term.clone())
                .or_default()
                .push(id.to_string());
        }

        self.documents
            .insert(id.to_string(), Document { fields: doc_fields });
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchResult> {
        let query_tokens = Self::tokenize(query);
        if query_tokens.is_empty() || self.documents.is_empty() {
            return Vec::new();
        }

        let mut scores: HashMap<&str, f64> = HashMap::new();

        for term in &query_tokens {
            let idf = self.idf(term);

            for (doc_id, doc) in &self.documents {
                let mut doc_term_score = 0.0;

                for (field_name, field_data) in &doc.fields {
                    let tf = field_data.tokens.iter().filter(|t| *t == term).count() as f64;

                    if tf == 0.0 {
                        continue;
                    }

                    let field_len = field_data.tokens.len() as f64;
                    let avg_fl = self.avg_field_len(field_name);

                    let tf_norm = if avg_fl == 0.0 {
                        0.0
                    } else {
                        (tf * (Self::K1 + 1.0))
                            / (tf + Self::K1 * (1.0 - Self::B + Self::B * field_len / avg_fl))
                    };

                    doc_term_score += idf * tf_norm * field_data.weight;
                }

                if doc_term_score > 0.0 {
                    *scores.entry(doc_id.as_str()).or_insert(0.0) += doc_term_score;
                }
            }
        }

        let mut results: Vec<SearchResult> = scores
            .into_iter()
            .map(|(id, score)| SearchResult {
                id: id.to_string(),
                score,
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        results
    }
}
