use std::fs::File;
use std::io;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

#[derive(Debug)]
pub struct PageTable {
    pub entries: Vec<PageTableEntry>,
}

#[derive(Debug)]
pub struct PageTableEntry {
    pub passage_id: u32,
    pub doc_length: u32,
}

impl PageTable {
    pub fn load (path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut entries = Vec::new();
        while !reader.fill_buf()?.is_empty() {
            let mut bytes = [0_u8; 8];
            reader.read_exact(&mut bytes)?;
            entries.push(PageTableEntry {
                passage_id: u32::from_le_bytes(bytes[..4].try_into().unwrap()),
                doc_length: u32::from_le_bytes(bytes[4..].try_into().unwrap()),
            });
        }
        Ok(Self { entries })
    }

    pub fn get(&self, doc_id: u32) -> Option<&PageTableEntry> {
        self.entries.get(doc_id as usize)
    }
}