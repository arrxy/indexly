mod reader;
mod builder;
mod writer;

use std::fs::{self, File};
use std::io;
use std::io::{BufWriter, Write};
use std::path::Path;
use crate::tokenizer::Tokenizer;
use builder::IndexBuilder;

pub fn build_runs<T: Tokenizer> (input_path: &str,
                                output_path: &str,
                                max_postings: usize,
                                tokenizer: &T) -> io::Result<usize> {

    if max_postings == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput, "max_postings must be greater than 0")
        );
    }
    fs::create_dir_all(output_path)?;
    let page_file = File::create(Path::new(output_path).join("page_table.bin"))?;
    let mut page_writer = BufWriter::new(page_file);
    let mut builder = IndexBuilder::new();
    let mut run_count = 0;
    let passages = reader::read_passages_from_file(input_path)?;
    for (number, passage) in passages.enumerate() {
        let (passage_id, text) = passage?;
        let doc_id = u32::try_from(number).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData, "Too many documents",
            )
        })?;
        let tokens = tokenizer.tokenize(&text);
        let doc_length = builder.add_document(doc_id, tokens);
        page_writer.write_all(&passage_id.to_le_bytes())?;
        page_writer.write_all(&doc_length.to_le_bytes())?;
        if builder.postings_count() >= max_postings {
            run_count += 1;
            let path = format!("{output_path}/run_{run_count:04}.bin");
            builder.flush_to_file(&path)?;
        }
    }
    if builder.postings_count() > 0 {
        run_count += 1;
        let path = format!("{output_path}/run_{run_count:04}.bin");
        builder.flush_to_file(&path)?;
    }
    page_writer.flush()?;
    Ok(run_count)
}