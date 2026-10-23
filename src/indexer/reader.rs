//! Stream TSV rows as original passage IDs and text; assign no internal IDs here.
//! Only the first tab separates the numeric ID from text, which may contain tabs.
//! Row parsing errors are yielded lazily when the indexing producer reaches them.

use std::io::{self, BufRead, BufReader};

/// Open a buffered row iterator; each item carries its own parsing or I/O error.
pub fn read_passages_from_file(
    file_path: &str,
) -> io::Result<impl Iterator<Item = io::Result<(u32, String)>>> {
    let file = std::fs::File::open(file_path)?;
    let reader = BufReader::new(file);
    Ok(reader.lines().map(|line| {
        let line = line?;
        let (id, text) = line
            .split_once('\t')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Invalid line format"))?;
        let passage_id: u32 = id
            .parse()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid passage id"))?;
        Ok((passage_id, text.to_owned()))
    }))
}
