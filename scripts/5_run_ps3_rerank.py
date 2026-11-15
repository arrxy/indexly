#!/usr/bin/env python3
# /// script
# dependencies = ["h5py==3.15.1", "numpy==2.0.0"]
# ///
"""Rerank PS3 BM25 candidates by the raw dot product of their embeddings."""

import argparse
from itertools import groupby
from pathlib import Path
from tempfile import TemporaryDirectory
from time import perf_counter

import h5py
import numpy as np


REPO = Path(__file__).resolve().parents[1]
SPLITS = ("eval.one", "eval.two", "dev")
PASSAGE_EMBEDDINGS = "msmarco_passages_embeddings_subset.h5"
QUERY_EMBEDDINGS = "msmarco_queries_dev_eval_embeddings.h5"


def load_embeddings(path: Path) -> tuple[dict[bytes, int], np.ndarray]:
    with h5py.File(path, "r") as source:
        ids = source["id"][:]
        embeddings = source["embedding"]
        if ids.ndim != 1 or embeddings.ndim != 2 or not len(ids) or len(ids) != len(embeddings):
            raise ValueError(f"{path}: expected one ID per embedding")
        rows = {identifier: row for row, identifier in enumerate(ids)}
        if len(rows) != len(ids):
            raise ValueError(f"{path}: duplicate embedding IDs")
        # Keep the original float16 storage in memory. Only candidates for
        # the current query need to be converted to float32 for scoring.
        return rows, embeddings[:]


def candidate_rows(source):
    for line_number, line in enumerate(source, start=1):
        fields = line.rstrip(b"\r\n").split(b"\t")
        if len(fields) != 4 or not fields[0] or not fields[1]:
            raise ValueError(f"Line {line_number}: expected query ID, passage ID, rank, and score")
        query_id, passage_id, rank, _ = fields
        yield query_id, passage_id, int(rank)


def rerank_split(
    split: str,
    runs_dir: Path,
    output_dir: Path,
    passage_rows: dict[bytes, int],
    passages: np.ndarray,
    query_rows: dict[bytes, int],
    queries: np.ndarray,
) -> None:
    input_path = runs_dir / f"bm25.{split}.tsv"
    output_path = output_dir / f"rerank.{split}.tsv"
    seen_queries = set()
    hit_count = 0
    started = perf_counter()
    with TemporaryDirectory(prefix=f".rerank-{split}-", dir=output_dir) as staging:
        staged_output = Path(staging) / output_path.name
        with input_path.open("rb") as source, staged_output.open("wb") as target:
            for query_id, candidates in groupby(candidate_rows(source), key=lambda row: row[0]):
                if query_id in seen_queries:
                    raise ValueError(f"Query {query_id.decode()} appears in multiple groups")
                seen_queries.add(query_id)
                query_row = query_rows.get(query_id)
                if query_row is None:
                    raise ValueError(f"Missing embedding for query {query_id.decode()}")

                passage_ids = []
                vector_rows = []
                seen_passages = set()
                for expected_rank, (_, passage_id, rank) in enumerate(candidates, start=1):
                    if rank != expected_rank or passage_id in seen_passages:
                        raise ValueError(f"Invalid ranks or duplicate passages for query {query_id.decode()}")
                    seen_passages.add(passage_id)
                    passage_row = passage_rows.get(passage_id)
                    if passage_row is None:
                        raise ValueError(f"Missing embedding for passage {passage_id.decode()}")
                    passage_ids.append(passage_id)
                    vector_rows.append(passage_row)

                query_vector = queries[query_row].astype(np.float32)
                candidate_vectors = passages[vector_rows].astype(np.float32)
                scores = np.einsum("ij,j->i", candidate_vectors, query_vector)
                if not np.isfinite(scores).all():
                    raise ValueError(f"Non-finite dot product for query {query_id.decode()}")
                # Keep the BM25 order when two dot products tie.
                order = np.argsort(-scores, kind="stable")
                for rank, candidate in enumerate(order, start=1):
                    fields = b"%s\t%s\t%d\t%.9g\n"
                    target.write(fields % (query_id, passage_ids[candidate], rank, scores[candidate]))
                hit_count += len(passage_ids)

        staged_output.replace(output_path)
    elapsed = perf_counter() - started
    print(f"Saved {output_path} with {hit_count:,} hits for {len(seen_queries):,} queries in {elapsed:.2f}s")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--split", choices=SPLITS, help="Run one split (default: all three)")
    parser.add_argument("--data-dir", type=Path, default=REPO / "data/ms_marco")
    parser.add_argument("--runs-dir", type=Path, default=REPO / "output/ps3/runs")
    parser.add_argument("--output-dir", type=Path, help="Save results here (default: --runs-dir)")
    args = parser.parse_args()

    output_dir = args.output_dir or args.runs_dir
    try:
        print("Loading passage and query embeddings", flush=True)
        passage_rows, passages = load_embeddings(args.data_dir / PASSAGE_EMBEDDINGS)
        query_rows, queries = load_embeddings(args.data_dir / QUERY_EMBEDDINGS)
        if passages.shape[1] != queries.shape[1]:
            raise ValueError("Query and passage embedding dimensions differ")
        output_dir.mkdir(parents=True, exist_ok=True)
        for split in (args.split,) if args.split else SPLITS:
            try:
                rerank_split(split, args.runs_dir, output_dir, passage_rows, passages, query_rows, queries)
            except ValueError as error:
                raise ValueError(f"bm25.{split}.tsv: {error}") from error
    except (KeyError, OSError, ValueError) as error:
        parser.exit(1, f"{parser.prog}: {error}\n")


if __name__ == "__main__":
    main()
