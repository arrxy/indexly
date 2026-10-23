//! Forward-only cursor over one term's compressed posting list.
//! Its block directory stays in memory, but only one posting block is decoded at
//! a time. Galloping over block-ending document IDs skips irrelevant payloads.
//! Each cursor owns its file handle, so separate cursors can seek independently.

use crate::binary::{invalid, read_u32, read_u64};
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use super::lexicon::LexiconEntry;
use crate::compression::IntegerCodec;

#[derive(Debug)]
struct BlockEntry {
    last_doc_id: u32,
    offset: u64,
}

/// One forward-only posting cursor; starts unpositioned and never rewinds.
pub struct PostingsReader<C: IntegerCodec> {
    reader: BufReader<File>,
    codec: C,
    doc_freq: u32,
    blocks: Vec<BlockEntry>,
    // First directory entry not yet consumed; blocks.len() marks exhaustion.
    next_block: usize,
    doc_ids: Vec<u32>,
    frequencies: Vec<u32>,
    // Same index in both decoded vectors; len() means the current block is spent.
    position: usize,
}

impl<C: IntegerCodec> PostingsReader<C> {
    /// Reads trusted indexer output; block ordering and payload metadata are not revalidated.
    pub fn open(index_dir: impl AsRef<Path>, entry: &LexiconEntry, codec: C) -> io::Result<Self> {
        let index_dir = index_dir.as_ref();
        let file = File::open(index_dir.join("postings.bin"))?;
        let mut reader = BufReader::new(file);
        let mut header = [0_u8; 9];
        reader.read_exact(&mut header)?;
        if &header[..4] != b"IDX1" || header[4] != codec.id() {
            return Err(invalid("Invalid postings header or codec"));
        }
        let file = File::open(index_dir.join("blocks.bin"))?;
        let mut directory = BufReader::new(file);
        let mut magic = [0_u8; 4];
        directory.read_exact(&mut magic)?;
        if &magic != b"BLK1" {
            return Err(invalid("Invalid block-directory header"));
        }
        directory.seek(SeekFrom::Start(entry.blocks_offset))?;
        let mut blocks: Vec<BlockEntry> = Vec::with_capacity(entry.block_count as usize);
        for _ in 0..entry.block_count {
            let block = BlockEntry {
                last_doc_id: read_u32(&mut directory)?,
                offset: read_u64(&mut directory)?,
            };
            blocks.push(block);
        }
        Ok(Self {
            reader,
            codec,
            doc_freq: entry.doc_freq,
            blocks,
            next_block: 0,
            doc_ids: Vec::new(),
            frequencies: Vec::new(),
            position: 0,
        })
    }

    /// Total documents in this list, used for IDF and rarest-first AND ordering.
    pub fn doc_freq(&self) -> u32 {
        self.doc_freq
    }

    /// Current document, or None before the first seek and after exhaustion.
    pub fn doc_id(&self) -> Option<u32> {
        self.doc_ids.get(self.position).copied()
    }

    /// Term occurrences in the current document; valid whenever doc_id is Some.
    pub fn frequency(&self) -> Option<u32> {
        self.frequencies.get(self.position).copied()
    }

    /// Seek to the first remaining document at least `target`.
    /// A lower target keeps the current document; an exhausted cursor stays exhausted.
    pub fn next_geq(&mut self, target: u32) -> io::Result<Option<u32>> {
        // Reuse the decoded suffix before touching the directory or disk.
        self.position += self.doc_ids[self.position..].partition_point(|&doc_id| doc_id < target);
        if let Some(doc_id) = self.doc_id() {
            return Ok(Some(doc_id));
        }
        // Choose a block by its last ID; intermediate payloads never need decoding.
        let Some(block_index) = gallop(&self.blocks, self.next_block, target) else {
            self.next_block = self.blocks.len();
            self.doc_ids.clear();
            self.frequencies.clear();
            self.position = 0;
            return Ok(None);
        };
        self.load_block(block_index)?;
        self.position = self.doc_ids.partition_point(|&doc_id| doc_id < target);
        Ok(self.doc_id())
    }

    fn load_block(&mut self, index: usize) -> io::Result<()> {
        let entry = &self.blocks[index];
        self.reader.seek(SeekFrom::Start(entry.offset))?;
        // Five u32 fields: first ID, last ID, posting count, gap bytes, frequency bytes.
        let first = read_u32(&mut self.reader)?;
        let _last = read_u32(&mut self.reader)?;
        let count = read_u32(&mut self.reader)?;
        let gap_bytes = read_u32(&mut self.reader)?;
        let frequency_bytes = read_u32(&mut self.reader)?;
        let mut gaps = vec![0_u8; gap_bytes as usize];
        let mut frequencies = vec![0_u8; frequency_bytes as usize];
        self.reader.read_exact(&mut gaps)?;
        self.reader.read_exact(&mut frequencies)?;
        // The first ID is absolute; the remaining count - 1 values are positive gaps.
        let gaps = self.codec.decode(&gaps, count as usize - 1)?;
        let frequencies = self.codec.decode(&frequencies, count as usize)?;
        let mut doc_ids = Vec::with_capacity(count as usize);
        let mut doc_id = first;
        doc_ids.push(doc_id);
        for gap in gaps {
            doc_id += gap;
            doc_ids.push(doc_id);
        }
        self.doc_ids = doc_ids;
        self.frequencies = frequencies;
        self.position = 0;
        self.next_block = index + 1;
        Ok(())
    }
}

// Check start, start+1, start+2, start+4, ...; binary-search the bracket.
fn gallop(blocks: &[BlockEntry], start: usize, target: u32) -> Option<usize> {
    if start >= blocks.len() {
        return None;
    }
    let mut lower = start;
    let mut upper = start;
    let mut step = 1_usize;
    while blocks[upper].last_doc_id < target {
        lower = upper + 1;
        if lower == blocks.len() {
            return None;
        }
        upper = start.saturating_add(step).min(blocks.len() - 1);
        step = step.saturating_mul(2);
    }
    // Exponential probing found the bracket; binary search finds its first match.
    Some(lower + blocks[lower..=upper].partition_point(|b| b.last_doc_id < target))
}

#[cfg(test)]
#[path = "../../tests/unit/index/postings_reader.rs"]
mod tests;
