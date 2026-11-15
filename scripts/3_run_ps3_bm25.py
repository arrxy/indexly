import argparse
import subprocess
from pathlib import Path
from tempfile import TemporaryDirectory


REPO = Path(__file__).resolve().parents[1]
QUERY_FILES = {
    "eval.one": "queries.eval.tsv",
    "eval.two": "queries.eval.tsv",
    "dev": "queries.dev.tsv",
}


def select_queries(qrels: Path, queries: Path, output: Path) -> int:
    with qrels.open(encoding="utf-8") as source:
        wanted = {line.split(maxsplit=1)[0] for line in source if line.strip()}
    if not wanted:
        raise ValueError(f"{qrels}: no judged query IDs")

    missing = wanted.copy()
    with queries.open(encoding="utf-8") as source, output.open("w", encoding="utf-8") as target:
        for line_number, line in enumerate(source, start=1):
            query_id, separator, _ = line.rstrip("\n").partition("\t")
            if query_id not in wanted:
                continue
            if not separator:
                raise ValueError(f"{queries}:{line_number}: missing query text field")
            if query_id not in missing:
                raise ValueError(f"{queries}:{line_number}: duplicate query ID {query_id}")
            target.write(line)
            missing.remove(query_id)

    if missing:
        examples = ", ".join(sorted(missing)[:5])
        raise ValueError(f"{queries}: missing text for {len(missing):,} judged queries ({examples})")
    return len(wanted)


def run_split(split: str, data_dir: Path, index: Path, output_dir: Path) -> None:
    result_path = output_dir / f"bm25.{split}.tsv"
    with TemporaryDirectory(prefix=f".{split}-", dir=output_dir) as staging:
        query_subset = Path(staging) / "queries.tsv"
        count = select_queries(
            data_dir / f"qrels.{split}.tsv",
            data_dir / QUERY_FILES[split],
            query_subset,
        )
        print(f"Running {split} with {count:,} judged queries", flush=True)

        staged_result = Path(staging) / result_path.name
        with staged_result.open("wb") as output:
            subprocess.run(
                [
                    "cargo", "run", "--release", "--locked", "--bin", "search", "--",
                    "--index", str(index),
                    "--queries", str(query_subset),
                    "--mode", "or",
                    "--top-k", "1000",
                ],
                cwd=REPO,
                stdout=output,
                check=True,
            )
        staged_result.replace(result_path)
    print(f"Saved {result_path}", flush=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--split", choices=QUERY_FILES,
        help="Run one split. By default, run eval.one, eval.two, and dev in order.",
    )
    parser.add_argument(
        "--data-dir", type=Path, default=REPO / "data/ms_marco",
        help="Directory containing qrels and query TSVs (default: %(default)s)",
    )
    parser.add_argument(
        "--index", type=Path, default=REPO / "output/ps3/bm25",
        help="Built BM25 index (default: %(default)s)",
    )
    parser.add_argument(
        "--output-dir", type=Path, default=REPO / "output/ps3/runs",
        help="Result directory (default: %(default)s)",
    )
    args = parser.parse_args()

    data_dir = args.data_dir.resolve()
    index = args.index.resolve()
    output_dir = args.output_dir.resolve()
    splits = (args.split,) if args.split else QUERY_FILES
    try:
        output_dir.mkdir(parents=True, exist_ok=True)
        for split in splits:
            run_split(split, data_dir, index, output_dir)
    except subprocess.CalledProcessError as error:
        parser.exit(1, f"{parser.prog}: BM25 search failed for {split} (exit {error.returncode})\n")
    except (OSError, ValueError) as error:
        parser.exit(1, f"{parser.prog}: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
