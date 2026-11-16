"""Measure build and eval.one batch costs for the three PS3 search systems."""

import argparse
import json
import os
import platform
import subprocess
import sys
from contextlib import redirect_stdout
from importlib import import_module
from pathlib import Path
from statistics import median
from tempfile import TemporaryDirectory
from time import perf_counter

select_queries = import_module("3_run_ps3_bm25").select_queries


REPO = Path(__file__).resolve().parents[1]
SCRIPTS = Path(__file__).resolve().parent
TOP_K = 1_000
REPEATS = 3


def measured(label: str, command: list[str], log: Path, stdout: Path | None = None) -> dict:
    started = perf_counter()
    with log.open("wb") as errors, (stdout or log.with_suffix(".stdout")).open("wb") as output:
        with subprocess.Popen(command, stdout=output, stderr=errors) as process:
            _, status, usage = os.wait4(process.pid, 0)
            process.returncode = os.waitstatus_to_exitcode(status)
    seconds = perf_counter() - started
    if process.returncode != 0:
        raise RuntimeError(f"{label} failed ({process.returncode}): {log.read_text().strip()}")

    # Darwin reports bytes. Linux reports KiB.
    rss_bytes = usage.ru_maxrss if sys.platform == "darwin" else usage.ru_maxrss * 1024
    print(f"{label}: {seconds:.3f}s, {rss_bytes / 2**20:.1f} MiB peak RSS", flush=True)
    return {"seconds": seconds, "peak_rss_bytes": rss_bytes}


def rerank_setup_and_score(data_dir: Path, source: Path, output: Path) -> None:
    reranker = import_module("5_run_ps3_rerank")

    started = perf_counter()
    passage_rows, passages = reranker.load_embeddings(data_dir / reranker.PASSAGE_EMBEDDINGS)
    query_rows, queries = reranker.load_embeddings(data_dir / reranker.QUERY_EMBEDDINGS)
    setup_seconds = perf_counter() - started
    started = perf_counter()
    with redirect_stdout(sys.stderr):
        reranker.rerank_split("eval.one", source, output, passage_rows, passages, query_rows, queries)
    print(json.dumps({"setup_seconds": setup_seconds, "score_and_write_seconds": perf_counter() - started}))


def compare(args: argparse.Namespace, work: Path, search: Path, indexer: Path) -> dict:
    python = sys.executable
    data_dir = args.data_dir.resolve()
    queries = work / "queries.tsv"
    query_count = select_queries(data_dir / "qrels.eval.one.tsv", data_dir / "queries.eval.tsv", queries)
    bm25_index = work / "bm25"
    hnsw_index = work / "hnsw.faiss"
    bm25_hits = work / "bm25.eval.one.tsv"

    bm25_build = measured(
        "BM25 build",
        [str(indexer), str(args.passages.resolve()), "--output", str(bm25_index), "--sort", "cpu"],
        work / "bm25-build.log",
    )
    bm25_build["index_bytes"] = sum(path.stat().st_size for path in bm25_index.iterdir())
    hnsw_base = ["--data-dir", str(data_dir), "--index", str(hnsw_index), "--threads", "8"]
    hnsw_script = str(SCRIPTS / "4_run_ps3_hnsw.py")
    hnsw_build = measured(
        "HNSW build",
        [python, hnsw_script, "build", *hnsw_base, "--m", "8", "--ef-construction", "100"],
        work / "hnsw-build.log",
    )
    hnsw_build["index_bytes"] = hnsw_index.stat().st_size

    bm25_command = [
        str(search), "--index", str(bm25_index), "--queries", str(queries),
        "--mode", "or", "--top-k", str(TOP_K),
    ]
    hnsw_command = [
        python, hnsw_script, "search", *hnsw_base, "--split", "eval.one", "--ef-search", "200",
        "--top-k", str(TOP_K), "--output-dir", str(work),
    ]
    rerank_command = [
        python, str(Path(__file__).resolve()), "--measure-rerank",
        "--data-dir", str(data_dir), "--output", str(work),
    ]

    trials = {system: [] for system in ("bm25", "hnsw", "rerank", "hybrid")}
    systems = ("bm25", "hnsw", "rerank")
    # Seed the standalone reranker and warm all three paths before timed runs.
    measured("BM25 warmup", bm25_command, work / "warm-bm25.log", bm25_hits)
    measured("HNSW warmup", hnsw_command, work / "warm-hnsw.log")
    measured("Rerank warmup", rerank_command, work / "warm-rerank.log")

    for trial in range(REPEATS):
        for system in systems[trial:] + systems[:trial]:
            suffix = f"{system}-{trial + 1}"
            log = work / f"{suffix}.log"
            if system == "bm25":
                measurement = measured(suffix, bm25_command, log, bm25_hits)
            elif system == "hnsw":
                measurement = measured(suffix, hnsw_command, log)
            else:
                started = perf_counter()
                candidates = measured(
                    f"hybrid candidates-{trial + 1}", bm25_command, work / "hybrid-bm25.log", bm25_hits,
                )
                measurement = measured(suffix, rerank_command, log)
                hybrid_seconds = perf_counter() - started
                measurement.update(json.loads(log.with_suffix(".stdout").read_text()))
                trials["hybrid"].append({
                    "seconds": hybrid_seconds,
                    "peak_rss_bytes": max(candidates["peak_rss_bytes"], measurement["peak_rss_bytes"]),
                })
            trials[system].append(measurement)

    import faiss
    import h5py
    import numpy

    source_commit = subprocess.run(
        ["git", "-C", str(args.repo), "rev-parse", "--short", "HEAD"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    return {
        "environment": {
            "platform": platform.platform(),
            "cpu_count": os.cpu_count(),
            "python": platform.python_version(),
            "faiss": faiss.__version__,
            "h5py": h5py.__version__,
            "numpy": numpy.__version__,
            "rust_commit": source_commit,
        },
        "split": "eval.one",
        "query_count": query_count,
        "top_k": TOP_K,
        "build": {"bm25": bm25_build, "hnsw": hnsw_build},
        "rerank_embedding_bytes": sum(
            path.stat().st_size for path in (
                data_dir / "msmarco_passages_embeddings_subset.h5",
                data_dir / "msmarco_queries_dev_eval_embeddings.h5",
            )
        ),
        "trials": trials,
        "medians": {
            system: {key: median(trial[key] for trial in measurements) for key in measurements[0]}
            for system, measurements in trials.items()
        },
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=REPO)
    parser.add_argument("--data-dir", type=Path, default=REPO / "data/ms_marco")
    parser.add_argument("--passages", type=Path, default=REPO / "data/ps3/passages.tsv")
    parser.add_argument("--output", type=Path, default=REPO / "output/ps3/benchmark.json")
    parser.add_argument("--measure-rerank", action="store_true", help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.measure_rerank:
        rerank_setup_and_score(args.data_dir, args.output, args.output)
        return

    with TemporaryDirectory(prefix="ps3-bench-") as temporary:
        work = Path(temporary)
        target = work / "target"
        subprocess.run(
            [
                "cargo", "build", "--release", "--locked", "--manifest-path",
                str(args.repo / "Cargo.toml"), "--target-dir", str(target),
                "--bin", "search", "--bin", "search-engine",
            ],
            check=True,
        )
        report = compare(args, work, target / "release/search", target / "release/search-engine")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(f"Saved {args.output}")


if __name__ == "__main__":
    main()
