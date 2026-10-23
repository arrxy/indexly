//! BM25 term scoring with collection statistics cached across queries.
//! Compute inverse document frequency once per distinct query term, then sum
//! its contribution with those of the other terms for each candidate document.

use crate::index::CollectionStats;
use std::io;

/// Cached corpus statistics and scoring parameters shared across query terms.
pub struct Bm25 {
    document_count: u64,
    average_doc_length: f64,
    k1: f64,
    b: f64,
}

impl Bm25 {
    /// Validate k1 (term-frequency saturation) and b (document-length normalization).
    pub fn new(stats: &CollectionStats, k1: f64, b: f64) -> io::Result<Self> {
        if !k1.is_finite() || k1 < 0.0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "k1 must be finite and nonnegative",
            ));
        }
        if !b.is_finite() || !(0.0..=1.0).contains(&b) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "b must be between 0 and 1",
            ));
        }
        if stats.document_count == 0 && stats.total_doc_length != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "An empty collection cannot have indexed tokens",
            ));
        }
        Ok(Self {
            document_count: stats.document_count,
            average_doc_length: stats.average_doc_length(),
            k1,
            b,
        })
    }

    /// Compute ln(1 + (N - df + 0.5) / (df + 0.5)); absent terms contribute zero.
    pub fn idf(&self, doc_freq: u32) -> io::Result<f64> {
        if u64::from(doc_freq) > self.document_count {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Document frequency exceeds document count",
            ));
        }
        if doc_freq == 0 {
            return Ok(0.0);
        }
        let n = self.document_count as f64;
        let df = f64::from(doc_freq);
        Ok(((n - df + 0.5) / (df + 0.5)).ln_1p())
    }

    /// Score one term using its precomputed IDF and the document's full token count.
    pub fn score(&self, term_frequency: u32, doc_length: u32, idf: f64) -> f64 {
        // OR queries include absent terms; an all-empty corpus has no meaningful length ratio.
        if term_frequency == 0 || self.average_doc_length == 0.0 {
            return 0.0;
        }
        let tf = f64::from(term_frequency);
        let length_normalization =
            1.0 - self.b + self.b * f64::from(doc_length) / self.average_doc_length;
        idf * (tf * (self.k1 + 1.0)) / (tf + self.k1 * length_normalization)
    }
}
