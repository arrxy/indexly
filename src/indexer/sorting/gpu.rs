/* Adapt search-engine postings to metal-radix's numeric key-pair API.
Batch-local term ranks preserve the ordering required by the run merger.
The crate owns GPU pipelines, shader execution and result validation. */

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use metal_radix::GpuSorter as RadixSorter;

use super::{IntermediatePosting, PostingOrder, PostingSorter};

#[derive(Default)]
struct Stats {
    batches: u64,
    postings: u64,
    prepare: Duration,
    wait: Duration,
    gpu: Duration,
}

pub struct GpuSorter {
    sorter: Mutex<RadixSorter>,
    description: String,
    stats: Mutex<Stats>,
}

impl GpuSorter {
    pub fn new() -> io::Result<Self> {
        let sorter = RadixSorter::new()?;
        let description = sorter.description().to_owned();
        Ok(Self {
            sorter: Mutex::new(sorter),
            description,
            stats: Mutex::new(Stats::default()),
        })
    }
}

impl PostingSorter for GpuSorter {
    fn sort(&self, postings: &mut [IntermediatePosting]) -> io::Result<PostingOrder> {
        if postings.is_empty() {
            return Ok(PostingOrder::Indices(Vec::new()));
        }
        let start = Instant::now();
        let keys = numeric_keys(postings)?;
        let prepare = start.elapsed();
        let start = Instant::now();
        let sorter = self
            .sorter
            .lock()
            .map_err(|_| io::Error::other("GPU sorter poisoned"))?;
        let wait = start.elapsed();
        let start = Instant::now();
        let indices = sorter.sort_indices(&keys)?;
        let gpu = start.elapsed();
        drop(sorter);
        drop(keys);
        let mut stats = self
            .stats
            .lock()
            .map_err(|_| io::Error::other("GPU sort statistics poisoned"))?;
        stats.batches += 1;
        stats.postings += postings.len() as u64;
        stats.prepare += prepare;
        stats.wait += wait;
        stats.gpu += gpu;
        Ok(PostingOrder::Indices(indices))
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn report(&self) {
        if let Ok(stats) = self.stats.lock() {
            println!(
                "Sort GPU: {} batches, {} postings; aggregate worker times: term IDs {:.2}s, GPU queue wait {:.2}s, GPU key packing/submission/readback/validation {:.2}s; writer follows sorted indices (no posting reorder)",
                stats.batches,
                stats.postings,
                stats.prepare.as_secs_f64(),
                stats.wait.as_secs_f64(),
                stats.gpu.as_secs_f64()
            );
        }
    }
}

fn numeric_keys(postings: &[IntermediatePosting]) -> io::Result<Vec<[u32; 2]>> {
    if postings.len() > u32::MAX as usize {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "GPU batch has more than u32::MAX postings",
        ));
    }
    let mut unique: HashSet<&str> = HashSet::new();
    for posting in postings {
        unique.insert(posting.term.as_str());
    }
    let mut vocabulary: Vec<_> = unique.into_iter().collect();
    vocabulary.sort_unstable();
    let ids: HashMap<_, _> = vocabulary
        .into_iter()
        .enumerate()
        .map(|(id, term)| (term, id as u32))
        .collect();
    Ok(postings
        .iter()
        .map(|posting| [ids[posting.term.as_str()], posting.doc_id])
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_ids_preserve_utf8_lexicographic_order() {
        let postings = vec![
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
        let records = numeric_keys(&postings).unwrap();
        assert_eq!(records, vec![[2, 7], [1, u32::MAX], [2, 0], [0, 8]]);
    }

    #[test]
    #[ignore = "Requires a hardware Metal GPU; run with --features gpu-sort -- --ignored"]
    fn gpu_radix_matches_cpu_oracle_at_block_boundaries_and_full_u32_keys() {
        let radix = RadixSorter::new().unwrap();
        for count in [0, 1, 2, 15, 255, 256, 257, 1025, 65537] {
            let mut state = 0x1234_5678_u32;
            let mut records: Vec<[u32; 2]> = (0..count)
                .map(|index| {
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    let term = if index % 7 == 0 { u32::MAX } else { state % 33 };
                    state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                    [term, if index % 13 == 0 { 0 } else { state }]
                })
                .collect();
            for shape in 0..3 {
                if shape == 1 {
                    for record in &mut records {
                        record[0] = 0;
                        record[1] = 0;
                    }
                } else if shape == 2 {
                    for (i, record) in records.iter_mut().enumerate() {
                        record[0] = (count as usize - i) as u32;
                        record[1] = u32::MAX;
                    }
                }
                let mut expected: Vec<_> = (0..records.len()).collect();
                expected.sort_by_key(|&i| (records[i][0], records[i][1]));
                let indices = radix.sort_indices(&records).unwrap();
                assert_eq!(
                    indices
                        .iter()
                        .map(|&index| index as usize)
                        .collect::<Vec<_>>(),
                    expected,
                    "count={count}, shape={shape}"
                );
            }
        }
        let sorter = GpuSorter::new().unwrap();
        let mut postings: Vec<_> = (0..1031)
            .rev()
            .map(|i| IntermediatePosting {
                term: ["rust", "é", "a", "z", "東京"][i % 5].into(),
                doc_id: i as u32,
                term_frequency: i as u32 % 11 + 1,
            })
            .collect();
        let mut expected = postings.clone();
        expected.sort_by(|a, b| a.term.cmp(&b.term).then(a.doc_id.cmp(&b.doc_id)));
        let original = postings.clone();
        let PostingOrder::Indices(indices) = sorter.sort(&mut postings).unwrap() else {
            panic!("GPU must return sorted indices");
        };
        for (a, b) in indices.iter().map(|&i| &postings[i as usize]).zip(expected) {
            assert_eq!(
                (&a.term, a.doc_id, a.term_frequency),
                (&b.term, b.doc_id, b.term_frequency)
            );
        }
        for (a, b) in postings.iter().zip(&original) {
            assert_eq!(
                (&a.term, a.doc_id, a.term_frequency),
                (&b.term, b.doc_id, b.term_frequency)
            );
        }
        assert!(
            matches!(sorter.sort(&mut []).unwrap(), PostingOrder::Indices(indices) if indices.is_empty())
        );
    }
}
