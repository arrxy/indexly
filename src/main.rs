use std::io;
fn main() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("search") => indexly::retrieval::run_cli(args),
        Some("prepare-snippets") => indexly::app::run_prepare(args),
        _ => indexly::app::run_index(std::env::args().skip(1)),
    }
}
