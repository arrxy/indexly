/*
This module sorts intermediate postings by term, then by doc_id,
and writes each batch to a binary run file. CPU sorting reorders the
batch; GPU sorting returns indices that the writer follows directly.

Each record is stored as:
[term byte length: u32][UTF-8 term bytes][doc_id: u32][term_frequency: u32]

Integer fields use little-endian byte order. Writes are buffered
for efficient disk access.

These sorted run files are uncompressed. They will later be merged
to create the final compressed inverted index.
*/
use super::sorting::{PostingOrder, PostingSorter};
use crate::indexer::builder::IntermediatePosting;
use std::fs::File;
use std::io::{self, BufWriter, Write};
pub fn write_run(
    file_path: &str,
    postings: &mut [IntermediatePosting],
    sorter: &dyn PostingSorter,
) -> io::Result<()> {
    let order = sorter.sort(postings)?;
    #[cfg(any(feature = "gpu-sort", test))]
    if let PostingOrder::Indices(indices) = &order {
        if indices.len() != postings.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Posting order length does not match batch",
            ));
        }
    }
    let file = File::create(file_path)?;
    let mut writer = BufWriter::new(file);
    match order {
        PostingOrder::InPlace => {
            for posting in postings {
                write_posting(&mut writer, posting)?;
            }
        }
        #[cfg(any(feature = "gpu-sort", test))]
        PostingOrder::Indices(indices) => {
            for index in indices {
                let posting = postings.get(index as usize).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "Posting index outside batch")
                })?;
                write_posting(&mut writer, posting)?;
            }
        }
    }
    writer.flush()
}

fn write_posting(writer: &mut impl Write, posting: &IntermediatePosting) -> io::Result<()> {
    let term_bytes = posting.term.as_bytes();
    let term_length = u32::try_from(term_bytes.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Term exceeds the maximum byte length",
        )
    })?;
    writer.write_all(&term_length.to_le_bytes())?;
    writer.write_all(term_bytes)?;
    writer.write_all(&posting.doc_id.to_le_bytes())?;
    writer.write_all(&posting.term_frequency.to_le_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::sorting::CpuSorter;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct IndexedSorter(Vec<u32>);
    impl PostingSorter for IndexedSorter {
        fn sort(&self, _: &mut [IntermediatePosting]) -> io::Result<PostingOrder> {
            Ok(PostingOrder::Indices(self.0.clone()))
        }
        fn description(&self) -> &str {
            "test indexed sorter"
        }
        fn report(&self) {}
    }

    fn temp_dir() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "indexed-writer-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        root
    }

    fn snapshot(postings: &[IntermediatePosting]) -> Vec<(String, u32, u32)> {
        postings
            .iter()
            .map(|p| (p.term.clone(), p.doc_id, p.term_frequency))
            .collect()
    }

    #[test]
    fn indexed_writer_matches_cpu_bytes_without_moving_postings() {
        let root = temp_dir();
        let mut postings = vec![
            IntermediatePosting {
                term: "é".into(),
                doc_id: 7,
                term_frequency: 1,
            },
            IntermediatePosting {
                term: "z".into(),
                doc_id: u32::MAX,
                term_frequency: 2,
            },
            IntermediatePosting {
                term: "é".into(),
                doc_id: 0,
                term_frequency: 3,
            },
            IntermediatePosting {
                term: "a".into(),
                doc_id: 8,
                term_frequency: 4,
            },
        ];
        let original = snapshot(&postings);
        let mut cpu = postings.clone();
        let indexed_path = root.join("indexed.bin");
        let cpu_path = root.join("cpu.bin");
        write_run(
            indexed_path.to_str().unwrap(),
            &mut postings,
            &IndexedSorter(vec![3, 1, 2, 0]),
        )
        .unwrap();
        write_run(cpu_path.to_str().unwrap(), &mut cpu, &CpuSorter::default()).unwrap();
        assert_eq!(snapshot(&postings), original);
        assert_eq!(fs::read(indexed_path).unwrap(), fs::read(cpu_path).unwrap());
        let empty_path = root.join("empty.bin");
        write_run(
            empty_path.to_str().unwrap(),
            &mut [],
            &IndexedSorter(vec![]),
        )
        .unwrap();
        assert_eq!(fs::metadata(empty_path).unwrap().len(), 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_index_count_and_out_of_bounds_return_errors() {
        let root = temp_dir();
        let mut postings = vec![IntermediatePosting {
            term: "rust".into(),
            doc_id: 0,
            term_frequency: 1,
        }];
        for (label, indices) in [("short", vec![]), ("out-of-bounds", vec![1])] {
            let path = root.join(label);
            let result = write_run(
                path.to_str().unwrap(),
                &mut postings,
                &IndexedSorter(indices),
            );
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
            if label == "short" {
                assert!(!path.exists());
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}
