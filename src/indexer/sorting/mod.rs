/* Sort a posting batch before it is written as a run. GPU is the default
when compiled in and uses batch-local lexicographic numeric term IDs.
CPU sorting is available through --sort cpu or a CPU-only build. */
use std::io;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::builder::IntermediatePosting;

#[cfg(feature = "gpu-sort")]
mod gpu;

// CPU sorting leaves the batch ordered in place. GPU sorting instead
// returns original indices in sorted order, leaving String ownership alone.
pub enum PostingOrder {
    InPlace,
    #[cfg(any(feature = "gpu-sort", test))]
    Indices(Vec<u32>),
}

pub trait PostingSorter: Sync {
    fn sort(&self, postings: &mut [IntermediatePosting]) -> io::Result<PostingOrder>;
    fn description(&self) -> &str;
    fn report(&self);
}

#[derive(Default)]
pub struct CpuSorter {
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

pub fn create_sorter(mode: &str) -> io::Result<Box<dyn PostingSorter>> {
    match mode {
        "cpu" => Ok(Box::new(CpuSorter::default())),
        #[cfg(feature = "gpu-sort")]
        "gpu" => Ok(Box::new(gpu::GpuSorter::new()?)),
        #[cfg(not(feature = "gpu-sort"))]
        "gpu" => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "GPU sorting requires rebuilding with: cargo build --release --features gpu-sort",
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--sort must be cpu or gpu",
        )),
    }
}
