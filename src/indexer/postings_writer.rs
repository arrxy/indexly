/*
Consumes sorted postings incrementally and writes compressed blocks plus
lexicon entries. The final merge can call push() directly without writing
an uncompressed merged file. Only the current posting block is buffered.
All integer fields use little-endian byte order.
*/
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::Path;

use super::read_next_posting;
use crate::compression::IntegerCodec;

pub struct PostingsWriter<'a, C: IntegerCodec> {
    postings: BufWriter<File>,
    lexicon: BufWriter<File>,
    codec: &'a C,
    block_size: usize,
    position: u64,
    list_start: u64,
    current_term: Option<String>,
    previous_doc: Option<u32>,
    doc_freq: u32,
    term_count: u64,
    doc_ids: Vec<u32>,
    frequencies: Vec<u32>,
}

impl<'a, C: IntegerCodec> PostingsWriter<'a, C> {
    pub fn new(output_dir: &Path, block_size: usize, codec: &'a C) -> io::Result<Self> {
        let block_size_u32 = to_u32(block_size)?;
        if block_size == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Block size must be positive",
            ));
        }
        fs::create_dir_all(output_dir)?;
        let mut postings = BufWriter::new(File::create(output_dir.join("postings.bin"))?);
        let mut lexicon = BufWriter::new(File::create(output_dir.join("lexicon.bin"))?);
        postings.write_all(b"IDX1")?;
        postings.write_all(&[codec.id()])?;
        postings.write_all(&block_size_u32.to_le_bytes())?;
        lexicon.write_all(b"LEX1")?;

        Ok(Self {
            postings,
            lexicon,
            codec,
            block_size,
            position: 9,
            list_start: 9,
            current_term: None,
            previous_doc: None,
            doc_freq: 0,
            term_count: 0,
            doc_ids: Vec::with_capacity(block_size),
            frequencies: Vec::with_capacity(block_size),
        })
    }

    pub fn push(&mut self, term: &str, doc_id: u32, frequency: u32) -> io::Result<()> {
        if term.is_empty() || frequency == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Term must be nonempty and frequency positive",
            ));
        }
        if self
            .current_term
            .as_deref()
            .is_some_and(|current| term < current)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Terms are not sorted",
            ));
        }
        if self.current_term.as_deref() != Some(term) {
            self.finish_term()?;
            self.current_term = Some(term.to_owned());
            self.list_start = self.position;
            self.previous_doc = None;
            self.doc_freq = 0;
        }
        if self.previous_doc.is_some_and(|previous| doc_id <= previous) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Doc IDs must strictly increase within a term",
            ));
        }
        self.previous_doc = Some(doc_id);
        self.doc_ids.push(doc_id);
        self.frequencies.push(frequency);
        self.doc_freq = self.doc_freq.checked_add(1).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "Document frequency exceeds u32")
        })?;
        if self.doc_ids.len() == self.block_size {
            self.flush_block()?;
        }
        Ok(())
    }

    // Must succeed before temporary merge inputs are deleted.
    pub fn finish(mut self) -> io::Result<u64> {
        self.finish_term()?;
        self.postings.flush()?;
        self.lexicon.flush()?;
        Ok(self.term_count)
    }

    fn flush_block(&mut self) -> io::Result<()> {
        self.position += write_block(
            &mut self.postings,
            &mut self.doc_ids,
            &mut self.frequencies,
            self.codec,
        )?;
        Ok(())
    }

    fn finish_term(&mut self) -> io::Result<()> {
        if let Some(term) = self.current_term.take() {
            self.flush_block()?;
            write_term(
                &mut self.lexicon,
                &term,
                self.list_start,
                self.position - self.list_start,
                self.doc_freq,
            )?;
            self.term_count += 1;
        }
        Ok(())
    }
}

// Retained for callers that already have a merged binary file.
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

fn write_block<C: IntegerCodec>(
    output: &mut impl Write,
    doc_ids: &mut Vec<u32>,
    frequencies: &mut Vec<u32>,
    codec: &C,
) -> io::Result<u64> {
    if doc_ids.is_empty() {
        return Ok(0);
    }

    let gaps: Vec<u32> = doc_ids.windows(2).map(|pair| pair[1] - pair[0]).collect();

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

    let written = 20 + gap_bytes.len() as u64 + frequency_bytes.len() as u64;

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
    u32::try_from(value)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Value exceeds u32"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compression::Simple9, index::Lexicon};
    use std::io::{Read, Seek, SeekFrom};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "stream-writer-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn finish_flushes_tail_blocks_and_lexicon_entries() {
        let root = temp_dir("decode");
        let codec = Simple9;
        let mut writer = PostingsWriter::new(&root, 2, &codec).unwrap();
        let expected = [
            ("rust", vec![(0, 1), (1, 2), (4, 2)]),
            ("search", vec![(0, 1), (1, 1), (2, 1), (3, 1), (4, 1)]),
        ];
        for (term, postings) in &expected {
            for &(doc_id, frequency) in postings {
                writer.push(term, doc_id, frequency).unwrap();
            }
        }
        assert_eq!(writer.finish().unwrap(), 2);
        let lexicon = Lexicon::load(root.join("lexicon.bin")).unwrap();
        let mut file = File::open(root.join("postings.bin")).unwrap();
        let mut end = 9;
        for (term, expected_postings) in expected {
            let entry = &lexicon.terms[term];
            assert_eq!(entry.offset, end);
            assert_eq!(entry.doc_freq as usize, expected_postings.len());
            file.seek(SeekFrom::Start(entry.offset)).unwrap();
            let mut actual = Vec::new();
            while file.stream_position().unwrap() < entry.offset + entry.length {
                let mut bytes = [0; 20];
                file.read_exact(&mut bytes).unwrap();
                let fields: Vec<_> = bytes
                    .chunks_exact(4)
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                    .collect();
                let count = fields[2] as usize;
                let mut gaps = vec![0; fields[3] as usize];
                let mut frequencies = vec![0; fields[4] as usize];
                file.read_exact(&mut gaps).unwrap();
                file.read_exact(&mut frequencies).unwrap();
                let gaps = codec.decode(&gaps, count - 1).unwrap();
                let frequencies = codec.decode(&frequencies, count).unwrap();
                let mut ids = vec![fields[0]];
                for gap in gaps {
                    ids.push(ids.last().unwrap() + gap);
                }
                assert_eq!(*ids.last().unwrap(), fields[1]);
                actual.extend(ids.into_iter().zip(frequencies));
            }
            end = entry.offset + entry.length;
            assert_eq!(file.stream_position().unwrap(), end);
            assert_eq!(actual, expected_postings);
        }
        assert_eq!(end, file.metadata().unwrap().len());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn streamed_and_file_based_writer_match() {
        let root = temp_dir("compatibility");
        let input = root.join("merged.bin");
        let mut data = Vec::new();
        let records = [("rust", 0_u32, 1_u32), ("rust", 3, 2), ("search", 1, 1)];
        for (term, id, frequency) in records {
            data.extend_from_slice(&(term.len() as u32).to_le_bytes());
            data.extend_from_slice(term.as_bytes());
            data.extend_from_slice(&id.to_le_bytes());
            data.extend_from_slice(&frequency.to_le_bytes());
        }
        fs::write(&input, data).unwrap();
        let from_file = root.join("from_file");
        assert_eq!(write_index(&input, &from_file, 2, &Simple9).unwrap(), 2);
        let streamed = root.join("streamed");
        let mut writer = PostingsWriter::new(&streamed, 2, &Simple9).unwrap();
        for (term, id, frequency) in records {
            writer.push(term, id, frequency).unwrap();
        }
        assert_eq!(writer.finish().unwrap(), 2);
        for name in ["postings.bin", "lexicon.bin"] {
            assert_eq!(
                fs::read(from_file.join(name)).unwrap(),
                fs::read(streamed.join(name)).unwrap()
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_posting_order_and_zero_frequency_are_rejected() {
        let root = temp_dir("invalid");
        let cases = [
            vec![("z", 0, 1), ("a", 1, 1)],
            vec![("a", 1, 1), ("a", 1, 1)],
            vec![("a", 2, 1), ("a", 1, 1)],
            vec![("a", 0, 0)],
            vec![("", 0, 1)],
        ];
        for (number, records) in cases.into_iter().enumerate() {
            let mut writer =
                PostingsWriter::new(&root.join(number.to_string()), 2, &Simple9).unwrap();
            let result: io::Result<()> = records
                .into_iter()
                .try_for_each(|(term, id, frequency)| writer.push(term, id, frequency));
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
        }
        fs::remove_dir_all(root).unwrap();
    }
}
