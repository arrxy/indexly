/* Sort posting batches in place by term, then document ID. */
use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::{IntermediatePosting, PostingOrder, PostingSorter};

#[derive(Default)]
pub struct CpuSorter {
    // (batches, postings, total sort time)
    stats: Mutex<(u64, u64, Duration)>,
}

impl PostingSorter for CpuSorter {
    fn sort(&self, postings: &mut [IntermediatePosting]) -> io::Result<PostingOrder> {
        let start = Instant::now();
        postings.sort_unstable_by(|a, b| a.term.cmp(&b.term).then_with(|| a.doc_id.cmp(&b.doc_id)));
        let elapsed = start.elapsed();
        let mut stats = self
            .stats
            .lock()
            .map_err(|_| io::Error::other("Sort statistics poisoned"))?;
        stats.0 += 1;
        stats.1 += postings.len() as u64;
        stats.2 += elapsed;
        Ok(PostingOrder::InPlace)
    }

    fn description(&self) -> &str {
        "cpu"
    }

    fn report(&self) {
        if let Ok(stats) = self.stats.lock() {
            println!(
                "Sort CPU: {} batches, {} postings; {:.2}s aggregate worker sort time",
                stats.0,
                stats.1,
                stats.2.as_secs_f64()
            );
        }
    }
}
