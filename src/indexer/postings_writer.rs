//! Stream sorted postings into postings.bin, blocks.bin, and lexicon.bin.
//! Only the current block is buffered; the final merge can feed `push` directly.
//! IDX1 contains a codec byte and block size after its magic; BLK1 starts fixed
//! 12-byte directory records (last_doc_id:u32, postings_offset:u64). LEX2 stores
//! one entry per term. All integer fields are little-endian.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::Path;

use super::read_next_posting;
use crate::compression::IntegerCodec;

/// State for a single sorted stream; counters describe the current term and block.
pub struct PostingsWriter<'a, C: IntegerCodec> {
    postings: BufWriter<File>,
    lexicon: BufWriter<File>,
    blocks: BufWriter<File>,
    // Absolute byte positions include the magic/header already written to each file.
    blocks_position: u64,
    list_blocks_start: u64,
    block_count: u32,
    codec: &'a C,
    block_size: usize,
    position: u64,
    list_start: u64,
    current_term: Option<String>,
    doc_freq: u32,
    term_count: u64,
    doc_ids: Vec<u32>,
    frequencies: Vec<u32>,
}

impl<'a, C: IntegerCodec> PostingsWriter<'a, C> {
    /// Create index files with empty headers; block_size must be positive and fit u32.
    pub fn new(output_dir: &Path, block_size: usize, codec: &'a C) -> io::Result<Self> {
        let block_size_u32 = u32::try_from(block_size)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Block size exceeds u32"))?;
        if block_size == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Block size must be positive",
            ));
        }
        fs::create_dir_all(output_dir)?;
        let mut postings = BufWriter::new(File::create(output_dir.join("postings.bin"))?);
        let mut lexicon = BufWriter::new(File::create(output_dir.join("lexicon.bin"))?);
        let mut blocks = BufWriter::new(File::create(output_dir.join("blocks.bin"))?);
        postings.write_all(b"IDX1")?;
        postings.write_all(&[codec.id()])?;
        postings.write_all(&block_size_u32.to_le_bytes())?;
        lexicon.write_all(b"LEX2")?;
        blocks.write_all(b"BLK1")?;

        Ok(Self {
            postings,
            lexicon,
            blocks,
            blocks_position: 4,
            list_blocks_start: 4,
            block_count: 0,
            codec,
            block_size,
            position: 9,
            list_start: 9,
            current_term: None,
            doc_freq: 0,
            term_count: 0,
            doc_ids: Vec::with_capacity(block_size),
            frequencies: Vec::with_capacity(block_size),
        })
    }

    /// Accepts sorted, unique postings with nonempty terms and positive frequencies.
    /// Terms are lexicographically ordered, and IDs strictly increase within a term.
    /// Counts/lengths must fit u32; gaps and frequencies must fit the chosen codec.
    /// These trusted-input invariants are not checked again here.
    pub fn push(&mut self, term: &str, doc_id: u32, frequency: u32) -> io::Result<()> {
        // Close the old list before recording the new term's byte ranges.
        if self.current_term.as_deref() != Some(term) {
            self.finish_term()?;
            self.current_term = Some(term.to_owned());
            self.list_start = self.position;
            self.list_blocks_start = self.blocks_position;
            self.block_count = 0;
            self.doc_freq = 0;
        }
        self.doc_ids.push(doc_id);
        self.frequencies.push(frequency);
        self.doc_freq += 1;
        if self.doc_ids.len() == self.block_size {
            self.flush_block()?;
        }
        Ok(())
    }

    /// Flush the tail block and all three files, returning the number of terms.
    /// Callers must wait for success before deleting temporary merge inputs.
    pub fn finish(mut self) -> io::Result<u64> {
        self.finish_term()?;
        self.postings.flush()?;
        self.lexicon.flush()?;
        self.blocks.flush()?;
        Ok(self.term_count)
    }

    fn flush_block(&mut self) -> io::Result<()> {
        let Some(&last) = self.doc_ids.last() else {
            return Ok(());
        };
        let gaps: Vec<_> = self
            .doc_ids
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .collect();
        let gaps = self.codec.encode(&gaps)?;
        let frequencies = self.codec.encode(&self.frequencies)?;
        // A block header is five u32 fields, followed by separate gap and frequency payloads.
        for value in [
            self.doc_ids[0],
            last,
            self.doc_ids.len() as u32,
            gaps.len() as u32,
            frequencies.len() as u32,
        ] {
            self.postings.write_all(&value.to_le_bytes())?;
        }
        self.postings.write_all(&gaps)?;
        self.postings.write_all(&frequencies)?;
        // Record the header's offset before advancing it or clearing the decoded block.
        self.blocks.write_all(&last.to_le_bytes())?;
        self.blocks.write_all(&self.position.to_le_bytes())?;
        self.position += 20 + gaps.len() as u64 + frequencies.len() as u64;
        self.blocks_position += 12;
        self.block_count += 1;
        self.doc_ids.clear();
        self.frequencies.clear();
        Ok(())
    }

    fn finish_term(&mut self) -> io::Result<()> {
        if let Some(term) = self.current_term.take() {
            self.flush_block()?;
            // Variable-length LEX2 entry: term bytes, posting range, df, and directory range.
            self.lexicon.write_all(&(term.len() as u32).to_le_bytes())?;
            self.lexicon.write_all(term.as_bytes())?;
            self.lexicon.write_all(&self.list_start.to_le_bytes())?;
            self.lexicon
                .write_all(&(self.position - self.list_start).to_le_bytes())?;
            self.lexicon.write_all(&self.doc_freq.to_le_bytes())?;
            self.lexicon
                .write_all(&self.list_blocks_start.to_le_bytes())?;
            self.lexicon.write_all(&self.block_count.to_le_bytes())?;
            self.term_count += 1;
        }
        Ok(())
    }
}

/// Compress an already-merged run through the same writer used by the streaming merge.
pub fn write_index<C: IntegerCodec>(
    merged_path: &Path,
    output_dir: &Path,
    block_size: usize,
    codec: &C,
) -> io::Result<u64> {
    let mut input = BufReader::new(File::open(merged_path)?);
    let mut writer = PostingsWriter::new(output_dir, block_size, codec)?;
    while let Some(posting) = read_next_posting(&mut input)? {
        writer.push(&posting.term, posting.doc_id, posting.term_frequency)?;
    }
    writer.finish()
}

#[cfg(test)]
#[path = "../../tests/unit/indexer/postings_writer.rs"]
mod tests;
