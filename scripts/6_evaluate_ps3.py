#!/usr/bin/env python3
"""Compare the saved PS3 rankings"""

import argparse
from dataclasses import dataclass
from itertools import groupby
from math import log2
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
SPLITS = ("eval.one", "eval.two", "dev")
SYSTEMS = (("bm25", "BM25"), ("hnsw", "HNSW"), ("rerank", "BM25 reranked"))


def read_qrels(path: Path, graded: bool) -> dict[str, dict[str, int]]:
    judgments: dict[str, dict[str, int]] = {}
    with path.open(encoding="utf-8") as source:
        for line_number, line in enumerate(source, start=1):
            fields = line.split()
            if not fields:
                continue
            if len(fields) != (4 if graded else 3):
                raise ValueError(f"{path}:{line_number}: unexpected qrel format")
            query, passage, grade_text = fields[0], fields[-2], fields[-1]
            grade = int(grade_text)
            passages = judgments.setdefault(query, {})
            passages[passage] = grade
    if not judgments:
        raise ValueError(f"{path}: no relevance judgments")
    return judgments


def run_rows(source):
    for line in source:
        query, passage, _, _ = line.rstrip("\r\n").split("\t")
        yield query, passage


@dataclass
class Metrics:
    mrr10: float = 0.0 # Mean Reciprocal Rank - MRR@10 averages 1 / rank of the first relevant hit per query, using 0 if none appears in the top 10.
    recall100: float = 0.0 # Recall@100 averages the proportion of relevant hits in the top 100 per query.
    ndcg10: float = 0.0 # NDCG@10 averages the normalized discounted cumulative gain in the top 10 per query.
    ndcg100: float = 0.0 # NDCG@100 averages the normalized discounted cumulative gain in the top 100 per query.
    ap: float = 0.0 # Average Precision averages the precision at each relevant hit per query.


def dcg(grades: list[int]) -> float:
    return sum(grade / log2(rank + 1) for rank, grade in enumerate(grades, start=1))


def score_query(ranked: list[str], judgments: dict[str, int], graded: bool) -> Metrics:
    threshold = 2 if graded else 1
    relevant_total = sum(grade >= threshold for grade in judgments.values())
    grades = [judgments.get(passage, 0) for passage in ranked]
    relevant_ranks = [rank for rank, grade in enumerate(grades, start=1) if grade >= threshold]

    result = Metrics()
    if relevant_ranks and relevant_ranks[0] <= 10:
        result.mrr10 = 1 / relevant_ranks[0]
    if relevant_total:
        result.recall100 = sum(rank <= 100 for rank in relevant_ranks) / relevant_total
        if not graded:
            result.ap = sum(
                hits / rank for hits, rank in enumerate(relevant_ranks, start=1)
            ) / relevant_total

    if graded:
        ideal = sorted(judgments.values(), reverse=True)
        ideal10, ideal100 = dcg(ideal[:10]), dcg(ideal[:100])
        if ideal10:
            result.ndcg10 = dcg(grades[:10]) / ideal10
        if ideal100:
            result.ndcg100 = dcg(grades[:100]) / ideal100
    return result


def evaluate_run(
    path: Path, judgments: dict[str, dict[str, int]], graded: bool,
) -> tuple[Metrics, int]:
    totals = Metrics()
    seen_queries: set[str] = set()

    with path.open(encoding="utf-8") as source:
        for query, hits in groupby(run_rows(source), key=lambda row: row[0]):
            seen_queries.add(query)
            ranked = [passage for _, passage in hits]

            result = score_query(ranked, judgments[query], graded)
            totals.mrr10 += result.mrr10
            totals.recall100 += result.recall100
            totals.ndcg10 += result.ndcg10
            totals.ndcg100 += result.ndcg100
            totals.ap += result.ap

    # Judged queries with no returned rows contribute zero to each mean.
    count = len(judgments)
    return Metrics(
        mrr10=totals.mrr10 / count,
        recall100=totals.recall100 / count,
        ndcg10=totals.ndcg10 / count,
        ndcg100=totals.ndcg100 / count,
        ap=totals.ap / count,
    ), len(seen_queries)


def evaluation_table(data_dir: Path, runs_dir: Path, splits: tuple[str, ...]) -> str:
    headers = (
        "Split", "System", "Judged queries", "Queries with hits",
        "MRR@10", "Recall@100", "NDCG@10", "NDCG@100", "MAP",
    )
    rows = [headers]
    for split in splits:
        graded = split != "dev"
        qrels = read_qrels(data_dir / f"qrels.{split}.tsv", graded)
        for prefix, name in SYSTEMS:
            result, with_hits = evaluate_run(runs_dir / f"{prefix}.{split}.tsv", qrels, graded)
            ndcg10 = f"{result.ndcg10:.4f}" if graded else "—"
            ndcg100 = f"{result.ndcg100:.4f}" if graded else "—"
            mean_ap = f"{result.ap:.4f}" if not graded else "—"
            rows.append((
                split, name, f"{len(qrels):,}", f"{with_hits:,}",
                f"{result.mrr10:.4f}", f"{result.recall100:.4f}", ndcg10, ndcg100, mean_ap,
            ))

    widths = [max(map(len, column)) for column in zip(*rows)]
    lines = []
    for row in rows:
        cells = [
            cell.ljust(width) if column < 2 else cell.rjust(width)
            for column, (cell, width) in enumerate(zip(row, widths))
        ]
        lines.append("| " + " | ".join(cells) + " |")
    separator = ["-" * width if column < 2 else "-" * (width - 1) + ":"
                 for column, width in enumerate(widths)]
    lines.insert(1, "| " + " | ".join(separator) + " |")
    lines.extend([
        "",
        "Metrics follow the saved ranks. Each judged query has equal weight, including queries with zero hits.",
        "MRR and recall count grades 2–3 as relevant on the graded splits and label 1 on dev.",
        "NDCG uses the original grades as gains. Dev MAP uses the full saved ranking, up to 1,000 hits per query.",
    ])
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--split", choices=SPLITS, help="Evaluate one split (default: all three)")
    parser.add_argument("--data-dir", type=Path, default=REPO / "data/ms_marco")
    parser.add_argument("--runs-dir", type=Path, default=REPO / "output/ps3/runs")
    args = parser.parse_args()

    table = evaluation_table(
        args.data_dir, args.runs_dir, (args.split,) if args.split else SPLITS,
    )
    print(table)


if __name__ == "__main__":
    main()
