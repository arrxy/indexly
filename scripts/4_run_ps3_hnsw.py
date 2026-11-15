#!/usr/bin/env python3
# /// script
# dependencies = ["faiss-cpu==1.16.0", "h5py==3.16.0", "numpy==2.5.4"]
# ///
"""Build and search the PS3 FAISS HNSW index using raw dot products."""

import argparse
import os
from pathlib import Path
from tempfile import TemporaryDirectory
from time import perf_counter

import faiss
import h5py
import numpy as np


REPO = Path(__file__).resolve().parents[1]
SPLITS = ("eval.one", "eval.two", "dev")
PASSAGE_EMBEDDINGS = "msmarco_passages_embeddings_subset.h5"
QUERY_EMBEDDINGS = "msmarco_queries_dev_eval_embeddings.h5"
BUILD_BATCH_SIZE = 25_000
SEARCH_BATCH_SIZE = 64


def positive_int(value: str) -> int:
    number = int(value)
    if number < 1:
        raise argparse.ArgumentTypeError("expected a positive integer")
    return number


def embedding_datasets(source: h5py.File) -> tuple[h5py.Dataset, h5py.Dataset]:
    ids = source["id"]
    vectors = source["embedding"]
    if ids.ndim != 1 or vectors.ndim != 2 or len(ids) != len(vectors) or not len(ids):
        raise ValueError(f"{source.filename}: expected one ID per embedding")
    return ids, vectors


def build_index(args: argparse.Namespace) -> None:
    started = perf_counter()
    with h5py.File(args.data_dir / PASSAGE_EMBEDDINGS, "r") as source:
        ids, vectors = embedding_datasets(source)
        total, dimensions = vectors.shape
        graph = faiss.IndexHNSWFlat(dimensions, args.m, faiss.METRIC_INNER_PRODUCT)
        graph.hnsw.efConstruction = args.ef_construction
        index = faiss.IndexIDMap(graph)
        seen_ids = set()
        print(
            f"Building HNSW for {total:,} passages with M={args.m}, "
            f"efConstruction={args.ef_construction}, threads={args.threads}",
            flush=True,
        )
        for start in range(0, total, BUILD_BATCH_SIZE):
            stop = min(start + BUILD_BATCH_SIZE, total)
            ids_batch = ids[start:stop].astype(np.int64)
            batch_ids = set(ids_batch.tolist())
            if len(batch_ids) != len(ids_batch) or seen_ids & batch_ids or np.any(ids_batch < 0):
                raise ValueError("Passage IDs must be unique nonnegative integers")
            seen_ids.update(batch_ids)
            batch = np.asarray(vectors[start:stop], dtype=np.float32, order="C")
            index.add_with_ids(batch, ids_batch)
            if stop == total or stop % 100_000 == 0:
                print(f"Indexed {stop:,}/{total:,} passages", flush=True)

    args.index.parent.mkdir(parents=True, exist_ok=True)
    with TemporaryDirectory(prefix=".hnsw-build-", dir=args.index.parent) as staging:
        staged_index = Path(staging) / "index.faiss"
        faiss.write_index(index, str(staged_index))
        staged_index.replace(args.index)
    print(f"Saved {args.index} in {perf_counter() - started:.2f}s")


def search_split(
    split: str,
    args: argparse.Namespace,
    index: faiss.Index,
    vectors: h5py.Dataset,
    rows_by_id: dict[str, int],
) -> None:
    qrels = args.data_dir / f"qrels.{split}.tsv"
    with qrels.open(encoding="utf-8") as source:
        query_ids = {line.split(maxsplit=1)[0] for line in source if line.strip()}
    if not query_ids:
        raise ValueError(f"{qrels}: no judged query IDs")
    missing = query_ids - rows_by_id.keys()
    if missing:
        examples = ", ".join(sorted(missing)[:5])
        raise ValueError(f"{qrels}: missing embeddings for {len(missing):,} queries ({examples})")

    # h5py requires sorted row indices for indexed reads.
    ordered_ids = sorted(query_ids, key=rows_by_id.__getitem__)
    limit = min(args.top_k, index.ntotal)
    result_path = args.output_dir / f"hnsw.{split}.tsv"
    search_seconds = 0.0
    with TemporaryDirectory(prefix=f".{split}-hnsw-", dir=args.output_dir) as staging:
        staged_result = Path(staging) / result_path.name
        with staged_result.open("w", encoding="utf-8") as output:
            for start in range(0, len(ordered_ids), SEARCH_BATCH_SIZE):
                query_batch = ordered_ids[start:start + SEARCH_BATCH_SIZE]
                rows = [rows_by_id[query_id] for query_id in query_batch]
                batch = np.asarray(vectors[rows], dtype=np.float32, order="C")
                started = perf_counter()
                scores, passage_ids = index.search(batch, limit)
                search_seconds += perf_counter() - started
                for query_id, query_scores, hits in zip(query_batch, scores, passage_ids):
                    rank = 0
                    for passage_id, score in zip(hits, query_scores):
                        if passage_id == -1:
                            continue  # FAISS pads the result when the search finds fewer than k.
                        rank += 1
                        output.write(f"{query_id}\t{passage_id}\t{rank}\t{float(score)}\n")
        staged_result.replace(result_path)
    print(f"Saved {result_path} for {len(query_ids):,} queries ({search_seconds:.3f}s in FAISS)")


def search_index(args: argparse.Namespace) -> None:
    index = faiss.read_index(str(args.index))
    if not isinstance(index, faiss.IndexIDMap):
        raise ValueError(f"{args.index}: expected an index with original passage IDs")
    graph = faiss.downcast_index(index.index)
    if not isinstance(graph, faiss.IndexHNSWFlat) or graph.metric_type != faiss.METRIC_INNER_PRODUCT:
        raise ValueError(f"{args.index}: expected an HNSWFlat index using inner product")
    graph.hnsw.efSearch = args.ef_search

    args.output_dir.mkdir(parents=True, exist_ok=True)
    with h5py.File(args.data_dir / QUERY_EMBEDDINGS, "r") as source:
        ids, vectors = embedding_datasets(source)
        if vectors.shape[1] != index.d:
            raise ValueError("Query and passage embedding dimensions differ")
        rows_by_id = {query_id: row for row, query_id in enumerate(ids.asstr()[:])}
        if len(rows_by_id) != len(ids):
            raise ValueError(f"{source.filename}: duplicate query IDs")
        for split in (args.split,) if args.split else SPLITS:
            search_split(split, args, index, vectors, rows_by_id)


def main() -> None:
    common = argparse.ArgumentParser(add_help=False)
    common.add_argument("--data-dir", type=Path, default=REPO / "data/ms_marco")
    common.add_argument("--index", type=Path, default=REPO / "output/ps3/hnsw.faiss")
    common.add_argument("--threads", type=positive_int, default=min(8, os.cpu_count() or 1))

    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(required=True)
    build = commands.add_parser("build", parents=[common], help="Build and save the passage index")
    build.add_argument("--m", type=positive_int, default=8, help="HNSW graph degree (default: 8)")
    build.add_argument("--ef-construction", type=positive_int, default=100)
    build.set_defaults(run=build_index)

    search = commands.add_parser("search", parents=[common], help="Search the judged PS3 queries")
    search.add_argument("--split", choices=SPLITS, help="Search one split (default: all three)")
    search.add_argument("--output-dir", type=Path, default=REPO / "output/ps3/runs")
    search.add_argument("--top-k", type=positive_int, default=1_000)
    search.add_argument("--ef-search", type=positive_int, default=200)
    search.set_defaults(run=search_index)

    args = parser.parse_args()
    faiss.omp_set_num_threads(args.threads)
    try:
        args.run(args)
    except (KeyError, OSError, RuntimeError, ValueError) as error:
        parser.exit(1, f"{parser.prog}: {error}\n")


if __name__ == "__main__":
    main()
