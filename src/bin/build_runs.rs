//! Stage 1: write sorted posting runs, a page table, and the source manifest.

fn main() -> std::io::Result<()> {
    search_engine::app::run_build(std::env::args().skip(1))
}
