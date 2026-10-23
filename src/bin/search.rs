//! Standalone interactive search entry point; reuses the library's query loop.

fn main() -> std::io::Result<()> {
    search_engine::retrieval::run_cli(std::env::args().skip(1))
}
