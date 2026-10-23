//! Combined entry point for complete index construction and interactive search.

use std::io;
fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("search") => search_engine::retrieval::run_cli(args),
        _ => search_engine::app::run_index(std::env::args().skip(1)),
    }
}
