import argparse
from pathlib import Path
from tempfile import TemporaryDirectory


REPO = Path(__file__).resolve().parents[1]


def filter_collection(collection: Path, ids: Path, output: Path) -> int:
    if output.resolve() in (collection.resolve(), ids.resolve()):
        raise ValueError("Choose an output path different from both input files.")

    missing = set(ids.read_bytes().split())
    output.parent.mkdir(parents=True, exist_ok=True)
    matched = 0
    with TemporaryDirectory(prefix=".ps3-subset-", dir=output.parent) as staging:
        staged_output = Path(staging) / "passages.tsv"
        with collection.open("rb") as source, staged_output.open("wb") as target:
            for line in source:
                passage_id = line.split(b"\t", 1)[0]
                if passage_id in missing:
                    target.write(line)
                    missing.remove(passage_id)
                    matched += 1

        if missing:
            raise ValueError(f"{len(missing):,} requested passages are missing from {collection}")

        staged_output.replace(output)
    return matched


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--collection", type=Path, default=REPO / "data/data_set.tsv",
        help="Original passage TSV (default: %(default)s)",
    )
    parser.add_argument(
        "--ids", type=Path, default=REPO / "data/ms_marco/msmarco_passages_subset.tsv",
        help="PS3 passage IDs, one per line (default: %(default)s)",
    )
    parser.add_argument(
        "--output", type=Path, default=REPO / "data/ps3/passages.tsv",
        help="Filtered passage TSV (default: %(default)s)",
    )
    args = parser.parse_args()

    count = filter_collection(args.collection, args.ids, args.output)
    print(f"Wrote {count:,} passages to {args.output}")


if __name__ == "__main__":
    main()
