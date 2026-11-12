import argparse
from itertools import islice
from pathlib import Path
from tempfile import TemporaryDirectory


REPO = Path(__file__).resolve().parents[1]


def read_passage_ids(path: Path) -> dict[bytes, bool]:
    found: dict[bytes, bool] = {}
    with path.open("rb") as source:
        for line_number, line in enumerate(source, start=1):
            passage_id = line.strip()
            if not passage_id.isdigit():
                raise ValueError(f"{path}:{line_number}: expected one numeric passage ID")
            if passage_id in found:
                raise ValueError(f"{path}:{line_number}: duplicate passage ID {passage_id.decode()}")
            found[passage_id] = False
    if not found:
        raise ValueError(f"{path}: the passage ID list is empty")
    return found


def filter_collection(collection: Path, ids: Path, output: Path) -> int:
    if output.resolve() in (collection.resolve(), ids.resolve()):
        raise ValueError("Choose an output path different from both input files.")

    found = read_passage_ids(ids)
    output.parent.mkdir(parents=True, exist_ok=True)
    matched = 0
    with TemporaryDirectory(prefix=".ps3-subset-", dir=output.parent) as staging:
        staged_output = Path(staging) / "passages.tsv"
        with collection.open("rb") as source, staged_output.open("wb") as target:
            for line in source:
                passage_id, separator, _ = line.partition(b"\t")
                already_found = found.get(passage_id)
                if already_found is None:
                    continue
                if not separator:
                    raise ValueError(f"Passage {passage_id.decode()} has no tab separator.")
                if already_found:
                    raise ValueError(f"{collection}: duplicate passage ID {passage_id.decode()}")
                target.write(line)
                found[passage_id] = True
                matched += 1

        if matched != len(found):
            missing = (passage_id.decode() for passage_id, seen in found.items() if not seen)
            examples = ", ".join(islice(missing, 5))
            raise ValueError(
                f"{len(found) - matched:,} passage IDs are missing from {collection}. "
                f"Examples: {examples}"
            )

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

    try:
        count = filter_collection(args.collection, args.ids, args.output)
    except (OSError, ValueError) as error:
        parser.exit(1, f"{parser.prog}: {error}\n")
    print(f"Wrote {count:,} passages to {args.output}")


if __name__ == "__main__":
    main()
