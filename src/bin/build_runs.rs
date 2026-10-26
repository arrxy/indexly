fn main() -> std::io::Result<()> {
    indexly::app::run_build(std::env::args().skip(1))
}
