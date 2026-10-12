use std::fs::File;

pub struct PageTable {
    pub file: File,
    pub entries: Vec<PageTableEntry>,
}

pub struct PageTableEntry {
    pub offset: u64,
    pub length: u32,
    pub doc_length: u32,
}