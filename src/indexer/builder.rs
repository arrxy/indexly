//! Accumulate one posting per distinct term/document pair in a reusable batch.
//! Repeated tokens become a frequency, while document length counts all tokens.
//! A flushed batch is sorted and serialized before its reusable buffer is cleared.

use std::collections::HashMap;

/// One distinct term/document pair, with occurrences already aggregated.
#[derive(Default, Debug, Clone)]
pub struct IntermediatePosting {
    pub term: String,
    pub doc_id: u32,
    pub term_frequency: u32,
}

/// A reusable posting buffer whose threshold is based on serialized record sizes.
#[derive(Default, Debug)]
pub struct IndexBuilder {
    postings: Vec<IntermediatePosting>,
    pub(crate) buffered_bytes: usize,
}

impl IndexBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Aggregate token frequencies and return the full token count for BM25.
    pub fn add_document(&mut self, doc_id: u32, tokens: Vec<String>) -> u32 {
        let doc_len = tokens.len() as u32;
        let mut term_counts: HashMap<String, u32> = HashMap::new();
        for term in tokens {
            *term_counts.entry(term).or_insert(0) += 1;
        }
        for (term, frequency) in term_counts {
            // A run record has three u32 fields (12 bytes) plus the UTF-8 term.
            self.buffered_bytes += 12 + term.len();
            self.postings.push(IntermediatePosting {
                term,
                doc_id,
                term_frequency: frequency,
            });
        }
        doc_len
    }

    pub fn postings_count(&self) -> usize {
        self.postings.len()
    }

    /// Clear buffered postings only after the sorted run has been written successfully.
    pub fn flush_to_file(
        &mut self,
        file_path: &str,
        sorter: &dyn super::sorting::PostingSorter,
    ) -> std::io::Result<()> {
        crate::indexer::writer::write_run(file_path, &mut self.postings, sorter)?;
        self.postings.clear();
        self.buffered_bytes = 0;
        Ok(())
    }
}
