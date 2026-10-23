//! Choose the posting sort backend without changing the serialized run format.
//! Both backends order by term, then document ID. GPU mode uses numeric term ranks
//! and returns original indices instead of moving the postings' owned strings.

use std::io;

use super::builder::IntermediatePosting;

mod cpu;

#[cfg(feature = "gpu-sort")]
mod gpu;

pub use cpu::CpuSorter;

// CPU sorting leaves the batch ordered in place. GPU sorting instead
// returns original indices in sorted order, leaving String ownership alone.
pub enum PostingOrder {
    InPlace,
    #[cfg(any(feature = "gpu-sort", test))]
    Indices(Vec<u32>),
}

/// Shared sorting interface for indexing workers.
/// Implementations must produce ascending (term, doc_id) order. An index-based
/// result must be a complete permutation; the run writer does not revalidate it.
pub trait PostingSorter: Sync {
    fn sort(&self, postings: &mut [IntermediatePosting]) -> io::Result<PostingOrder>;
    fn description(&self) -> &str;
}

/// Select the requested backend; an unavailable GPU is an error, not a CPU fallback.
pub fn create_sorter(mode: &str) -> io::Result<Box<dyn PostingSorter>> {
    match mode {
        "cpu" => Ok(Box::new(CpuSorter)),
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
