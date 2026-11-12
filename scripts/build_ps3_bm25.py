import argparse
import subprocess
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--passages", type=Path, default=REPO / "data/ps3/passages.tsv",
        help="Filtered passage TSV from prepare_ps3_subset.py (default: %(default)s)",
    )
    parser.add_argument(
        "--output", type=Path, default=REPO / "output/ps3/bm25",
        help="New index directory (default: %(default)s)",
    )
    args = parser.parse_args()

    command = [
        "cargo", "run", "--release", "--locked", "--bin", "search-engine", "--",
        str(args.passages.resolve()), "--output", str(args.output.resolve()), "--sort", "cpu",
    ]
    try:
        return subprocess.run(command, cwd=REPO, check=False).returncode
    except OSError as error:
        parser.exit(1, f"{parser.prog}: could not run Cargo: {error}\n")


if __name__ == "__main__":
    raise SystemExit(main())
