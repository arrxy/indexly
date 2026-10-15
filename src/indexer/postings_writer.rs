/*
Reads globally sorted intermediate postings and writes compressed blocks
plus a lexicon. Only one block of postings is buffered at a time.

All integer fields use little-endian byte order.
*/

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::Path;

use crate::compression::IntegerCodec;
use super::read_next_posting;

pub fn write_index<C: IntegerCodec>(
    merged_path: &Path,
    output_dir: &Path,
    block_size: usize,
    codec: &C,
) -> io::Result<u64> {
    let block_size_u32 = to_u32(block_size)?;

    if block_size == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Block size must be positive"));
    }

    let mut input = BufReader::new(File::open(merged_path)?);

    fs::create_dir_all(output_dir)?;

    let mut postings =
        BufWriter::new(File::create(output_dir.join("postings.bin"))?);
    let mut lexicon =
        BufWriter::new(File::create(output_dir.join("lexicon.bin"))?);

    // File header: magic/version (4), codec ID (1), block size (4).
    postings.write_all(b"IDX1")?;
    postings.write_all(&[codec.id()])?;
    postings.write_all(&block_size_u32.to_le_bytes())?;

    lexicon.write_all(b"LEX1")?;

    // Track byte positions without seeking or flushing per term.
    let mut position = 9_u64;
    let mut list_start = position;
    let mut current_term: Option<String> = None;
    let mut previous_doc = None;
    let mut doc_freq = 0_u32;
    let mut term_count = 0_u64;

    let mut doc_ids = Vec::with_capacity(block_size);
    let mut frequencies = Vec::with_capacity(block_size);

    while let Some(posting) = read_next_posting(&mut input)? {
        if let Some(term) = &current_term {
            if posting.term.as_str() < term.as_str() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Terms are not sorted",
                ));
            }
        }

        // Finish the previous term when a new term begins.
        if current_term.as_deref() != Some(posting.term.as_str()) {
            if let Some(term) = current_term.take() {
                position += write_block(
                    &mut postings,
                    &mut doc_ids,
                    &mut frequencies,
                    codec,
                )?;

                write_term(
                    &mut lexicon,
                    &term,
                    list_start,
                    position - list_start,
                    doc_freq,
                )?;

                term_count += 1;
            }

            current_term = Some(posting.term);
            list_start = position;
            previous_doc = None;
            doc_freq = 0;
        }

        if previous_doc.is_some_and(|id| posting.doc_id <= id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Doc IDs must strictly increase within a term",
            ));
        }

        if posting.term_frequency == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Term frequency must be positive",
            ));
        }

        previous_doc = Some(posting.doc_id);
        doc_ids.push(posting.doc_id);
        frequencies.push(posting.term_frequency);

        doc_freq = doc_freq.checked_add(1).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Document frequency exceeds u32",
            )
        })?;

        if doc_ids.len() == block_size {
            position += write_block(
                &mut postings,
                &mut doc_ids,
                &mut frequencies,
                codec,
            )?;
        }
    }

    // Finish the final term at EOF.
    if let Some(term) = current_term {
        position += write_block(
            &mut postings,
            &mut doc_ids,
            &mut frequencies,
            codec,
        )?;

        write_term(
            &mut lexicon,
            &term,
            list_start,
            position - list_start,
            doc_freq,
        )?;

        term_count += 1;
    }

    postings.flush()?;
    lexicon.flush()?;

    Ok(term_count)
}

fn write_block<C: IntegerCodec>(
    output: &mut impl Write,
    doc_ids: &mut Vec<u32>,
    frequencies: &mut Vec<u32>,
    codec: &C,
) -> io::Result<u64> {
    if doc_ids.is_empty() {
        return Ok(0);
    }

    let gaps: Vec<u32> = doc_ids
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .collect();

    let gap_bytes = codec.encode(&gaps)?;
    let frequency_bytes = codec.encode(frequencies)?;

    // Five u32 fields: 20 bytes total.
    let header = [
        doc_ids[0],
        *doc_ids.last().unwrap(),
        to_u32(doc_ids.len())?,
        to_u32(gap_bytes.len())?,
        to_u32(frequency_bytes.len())?,
    ];

    for value in header {
        output.write_all(&value.to_le_bytes())?;
    }

    output.write_all(&gap_bytes)?;
    output.write_all(&frequency_bytes)?;

    let written =
        20 + gap_bytes.len() as u64 + frequency_bytes.len() as u64;

    doc_ids.clear();
    frequencies.clear();

    Ok(written)
}

fn write_term(
    output: &mut impl Write,
    term: &str,
    offset: u64,
    length: u64,
    doc_freq: u32,
) -> io::Result<()> {
    output.write_all(&to_u32(term.len())?.to_le_bytes())?;
    output.write_all(term.as_bytes())?;
    output.write_all(&offset.to_le_bytes())?;
    output.write_all(&length.to_le_bytes())?;
    output.write_all(&doc_freq.to_le_bytes())
}

fn to_u32(value: usize) -> io::Result<u32> {
    u32::try_from(value).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidInput, "Value exceeds u32")
    })
}