//! Stage 2: merge saved runs into a searchable compressed index.

fn main() -> std::io::Result<()> {
    search_engine::app::run_merge(std::env::args().skip(1))
}
