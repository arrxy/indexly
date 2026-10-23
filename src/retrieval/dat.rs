//! Exact document-at-a-time retrieval over compressed posting cursors.
//! AND aligns cursors, starting with the rarest term; OR visits their union in
//! increasing document-ID order. A bounded heap retains the ten best hits.
//! Equal scores favor smaller internal document IDs for deterministic results.

use super::{bm25::Bm25, snippet};
use crate::compression::Simple9;
use crate::index::{CollectionStats, Lexicon, PageTable, PassageStore, PostingsReader};
use crate::tokenizer::{SimpleTokenizer, Tokenizer};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

/// Whether every normalized query term or at least one must occur in a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryMode {
    Conjunctive, // AND
    Disjunctive, // OR
}
impl QueryMode {
    pub fn parse(value: &str) -> io::Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "and" | "conjunctive" => Ok(Self::Conjunctive),
            "or" | "disjunctive" => Ok(Self::Disjunctive),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Mode must be 'and' or 'or'",
            )),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Conjunctive => "AND",
            Self::Disjunctive => "OR",
        }
    }
}

/// A query term's frequency and BM25 contribution for one result (zero if absent).
#[derive(Debug)]
pub struct TermScore {
    pub term: String,
    pub frequency: u32,
    pub score: f64,
}
/// One ranked passage, with internal/original IDs, score details, and optional snippet.
#[derive(Debug)]
pub struct SearchHit {
    pub doc_id: u32,
    pub passage_id: u32,
    pub score: f64,
    pub term_scores: Vec<TermScore>,
    pub snippet: String,
}
// BinaryHeap's root is the WORST retained result.
impl PartialEq for SearchHit {
    fn eq(&self, other: &Self) -> bool {
        self.doc_id == other.doc_id && self.score.total_cmp(&other.score) == Ordering::Equal
    }
}
impl Eq for SearchHit {}
impl Ord for SearchHit {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .score
            .total_cmp(&self.score)
            .then(self.doc_id.cmp(&other.doc_id))
    }
}
impl PartialOrd for SearchHit {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Distinct query terms, at most ten ranked hits, and the total number of matched documents.
pub struct SearchResult {
    pub terms: Vec<String>,
    pub hits: Vec<SearchHit>,
    pub matched_documents: u64,
}
struct TermCursor {
    // Maps a reordered cursor back to query-order frequency and IDF arrays.
    term_index: usize,
    reader: PostingsReader<Simple9>,
    current: Option<u32>,
}

/// Reusable loaded dictionaries/statistics; posting cursors are opened per query.
pub struct SearchEngine {
    index_dir: PathBuf,
    lexicon: Lexicon,
    page_table: PageTable,
    stats: CollectionStats,
    bm25: Bm25,
    passages: Option<PassageStore>,
}
impl SearchEngine {
    /// Load a matching set of trusted index files and optionally open the source TSV.
    /// Document counts, tokenizer identity, and posting/page-table consistency are assumed.
    pub fn open(
        index_dir: impl AsRef<Path>,
        dataset: Option<&Path>,
        snippets: bool,
        k1: f64,
        b: f64,
    ) -> io::Result<Self> {
        let index_dir = index_dir.as_ref().to_owned();
        let stats = CollectionStats::load(index_dir.join("metadata.bin"))?;
        let page_table = PageTable::load(index_dir.join("page_table.bin"))?;
        let lexicon = Lexicon::load(index_dir.join("lexicon.bin"))?;
        let passages = if snippets {
            Some(PassageStore::open(&page_table, dataset)?)
        } else {
            None
        };
        let bm25 = Bm25::new(&stats, k1, b)?;
        Ok(Self {
            index_dir,
            lexicon,
            page_table,
            stats,
            bm25,
            passages,
        })
    }

    pub fn document_count(&self) -> u64 {
        self.stats.document_count
    }
    pub fn term_count(&self) -> usize {
        self.lexicon.terms.len()
    }

    /// Return exact top-ten BM25 results for distinct normalized terms.
    /// Repeated query terms count once; an empty query returns no matches.
    pub fn search(&mut self, query: &str, mode: QueryMode) -> io::Result<SearchResult> {
        let mut seen = HashSet::new();
        let terms: Vec<String> = SimpleTokenizer
            .tokenize(query)
            .into_iter()
            .filter(|t| seen.insert(t.clone()))
            .collect();
        let mut result = SearchResult {
            terms,
            hits: Vec::new(),
            matched_documents: 0,
        };
        let mut cursors = Vec::new();
        let mut idfs = vec![0.0; result.terms.len()];
        for (term_index, term) in result.terms.iter().enumerate() {
            let Some(entry) = self.lexicon.terms.get(term) else {
                // A missing term makes AND impossible; OR can still match the remaining terms.
                if mode == QueryMode::Conjunctive {
                    return Ok(result);
                }
                continue;
            };
            idfs[term_index] = self.bm25.idf(entry.doc_freq)?;
            let reader = PostingsReader::open(&self.index_dir, entry, Simple9)?;
            cursors.push(TermCursor {
                term_index,
                reader,
                current: None,
            });
        }
        if cursors.is_empty() {
            return Ok(result);
        }
        let mut heap = BinaryHeap::with_capacity(11);
        let mut frequencies = vec![0; result.terms.len()];
        if mode == QueryMode::Conjunctive {
            // The shortest list usually rules out candidates earliest.
            cursors.sort_by_key(|c| c.reader.doc_freq());
            let mut target = 0;
            'align: loop {
                for cursor in &mut cursors {
                    let Some(doc_id) = cursor.reader.next_geq(target)? else {
                        break 'align;
                    };
                    if doc_id > target {
                        // This list skipped the target; restart alignment at its next possible candidate.
                        target = doc_id;
                        continue 'align;
                    }
                }
                // Every cursor now points to target, so each has a valid frequency.
                frequencies.fill(0);
                for cursor in &cursors {
                    frequencies[cursor.term_index] = cursor.reader.frequency().unwrap();
                }
                self.consider(target, &result.terms, &idfs, &frequencies, &mut heap);
                result.matched_documents += 1;
                let Some(next) = target.checked_add(1) else {
                    break;
                };
                target = next;
            }
        } else {
            for cursor in &mut cursors {
                cursor.current = cursor.reader.next_geq(0)?;
            }
            // Visit the smallest pending document once, combining all lists currently on it.
            while let Some(doc_id) = cursors.iter().filter_map(|c| c.current).min() {
                frequencies.fill(0);
                for cursor in &cursors {
                    if cursor.current == Some(doc_id) {
                        frequencies[cursor.term_index] = cursor.reader.frequency().unwrap();
                    }
                }
                self.consider(doc_id, &result.terms, &idfs, &frequencies, &mut heap);
                result.matched_documents += 1;
                // Advance only matching lists; later IDs remain pending for the next iteration.
                for cursor in &mut cursors {
                    if cursor.current == Some(doc_id) {
                        cursor.current = match doc_id.checked_add(1) {
                            Some(next) => cursor.reader.next_geq(next)?,
                            None => None,
                        };
                    }
                }
            }
        }
        // Heap order exposes the worst hit, not display order; sort the retained hits here.
        result.hits = heap.into_vec();
        result
            .hits
            .sort_by(|a, b| b.score.total_cmp(&a.score).then(a.doc_id.cmp(&b.doc_id)));
        // Defer source reads and snippet generation until only the top ten remain.
        if let Some(passages) = &mut self.passages {
            let generator = snippet::SnippetGenerator::new(&result.terms);
            for hit in &mut result.hits {
                let entry = &self.page_table.entries[hit.doc_id as usize];
                hit.snippet = generator.generate(&passages.text(entry)?);
            }
        }
        Ok(result)
    }

    fn consider(
        &self,
        doc_id: u32,
        terms: &[String],
        idfs: &[f64],
        frequencies: &[u32],
        heap: &mut BinaryHeap<SearchHit>,
    ) {
        // Dense posting IDs index the page table directly under the trusted-index contract.
        let entry = &self.page_table.entries[doc_id as usize];

        let score: f64 = frequencies
            .iter()
            .zip(idfs)
            .map(|(&tf, &idf)| self.bm25.score(tf, entry.doc_length, idf))
            .sum();
        // Reject noncompetitive candidates before cloning terms and allocating score details.
        if heap.len() == 10
            && heap.peek().is_some_and(|worst| {
                score < worst.score || (score == worst.score && doc_id >= worst.doc_id)
            })
        {
            return;
        }
        let term_scores = terms
            .iter()
            .zip(frequencies)
            .zip(idfs)
            .map(|((term, &frequency), &idf)| TermScore {
                term: term.clone(),
                frequency,
                score: self.bm25.score(frequency, entry.doc_length, idf),
            })
            .collect();
        heap.push(SearchHit {
            doc_id,
            passage_id: entry.passage_id,
            score,
            term_scores,
            snippet: String::new(),
        });
        // A candidate may temporarily grow the heap to eleven; discard its worst member.
        if heap.len() > 10 {
            heap.pop();
        }
    }
}
