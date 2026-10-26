fn main() -> std::io::Result<()> {
    indexly::retrieval::run_cli(std::env::args().skip(1))
}
