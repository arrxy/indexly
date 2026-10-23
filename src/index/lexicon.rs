use crate::binary::{invalid, read_u32, read_u64};
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;

#[derive(Debug)]
pub struct Lexicon {
    pub terms: HashMap<String, LexiconEntry>,
}

#[derive(Debug)]
pub struct LexiconEntry {
    pub offset: u64,
    pub length: u64,
    pub doc_freq: u32,
    pub blocks_offset: u64,
    pub block_count: u32,
}

impl Lexicon {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);

        let mut magic = [0; 4];
        reader.read_exact(&mut magic)?;

        if &magic != b"LEX2" {
            return Err(invalid(
                "Expected LEX2 lexicon; rebuild older indexes to add block directories",
            ));
        }

        let mut terms = HashMap::new();

        while !reader.fill_buf()?.is_empty() {
            let term_length = read_u32(&mut reader)?;
            let mut term_bytes = vec![0; term_length as usize];
            reader.read_exact(&mut term_bytes)?;

            let term = String::from_utf8(term_bytes)
                .map_err(|_| invalid("Invalid UTF-8 in lexicon term"))?;

            let entry = LexiconEntry {
                offset: read_u64(&mut reader)?,
                length: read_u64(&mut reader)?,
                doc_freq: read_u32(&mut reader)?,
                blocks_offset: read_u64(&mut reader)?,
                block_count: read_u32(&mut reader)?,
            };

            terms.insert(term, entry);
        }

        Ok(Self { terms })
    }
}
