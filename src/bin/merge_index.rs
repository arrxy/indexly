fn main() -> std::io::Result<()> {
    indexly::app::run_merge(std::env::args().skip(1))
}
