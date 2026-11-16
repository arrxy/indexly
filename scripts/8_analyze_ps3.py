#!/usr/bin/env python3
# /// script
# dependencies = ["matplotlib==3.11.2"]
# ///
"""Plot PS3 reranking sensitivity and retrieval quality by query length."""

import argparse
import json
import re
from collections import Counter, defaultdict
from importlib import import_module
from itertools import groupby
from pathlib import Path
from statistics import mean

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.ticker import MaxNLocator

evaluation = import_module("6_evaluate_ps3")


CUTOFFS = range(10, 101, 10)
METRICS = {"mrr10": "MRR@10", "ndcg10": "NDCG@10", "ndcg100": "NDCG@100", "ap": "MAP"}
SPLIT_NAMES = {"eval.one": "TREC 2019", "eval.two": "TREC 2020", "dev": "MS MARCO dev"}
COLORS = ("#126e82", "#a14a20", "#6a459a")
MARKERS = ("o", "s", "^")
TOKEN = re.compile(r"[^\W_]+")


def read_rankings(path):
    rankings = {}
    with path.open(encoding="utf-8") as source:
        for query, passage in evaluation.run_rows(source):
            rankings.setdefault(query, []).append(passage)
    return rankings


def query_lengths(path, judgments):
    lengths = {}
    with path.open(encoding="utf-8") as source:
        for line in source:
            query, _, text = line.partition("\t")
            if query in judgments:
                lengths[query] = len(TOKEN.findall(text))
    return lengths


def rerank_prefix(bm25, dense_order, cutoff):
    # The full rerank uses independent dot products and a stable sort. Filtering
    # it to the BM25 prefix recovers the dense order and tie breaks for that prefix.
    prefix = set(bm25[:cutoff])
    return [passage for passage in dense_order if passage in prefix] + bm25[cutoff:]


def analyze_split(split, data, runs):
    graded = split != "dev"
    metrics = ("mrr10", "ndcg10", "ndcg100") if graded else ("mrr10", "ap")
    judgments = evaluation.read_qrels(data / f"qrels.{split}.tsv", graded)
    lengths = query_lengths(data / ("queries.eval.tsv" if graded else "queries.dev.tsv"), judgments)
    rankings = {
        key: read_rankings(runs / f"{key}.{split}.tsv") for key, _ in evaluation.SYSTEMS
    }
    deltas = {k: defaultdict(list) for k in CUTOFFS}
    baseline = defaultdict(list)
    by_length = defaultdict(lambda: defaultdict(list))

    for query, qrels in judgments.items():
        bm25 = rankings["bm25"].get(query, [])
        reranked = rankings["rerank"].get(query, [])
        if set(bm25) != set(reranked):
            raise ValueError(f"{split}: BM25 and rerank candidates differ for query {query}")
        scores = {
            system: evaluation.score_query(ranking.get(query, []), qrels, graded)
            for system, ranking in rankings.items()
        }
        for metric in metrics:
            baseline[metric].append(getattr(scores["bm25"], metric))
            for system, result in scores.items():
                by_length[lengths[query]][system, metric].append(getattr(result, metric))
        for k in CUTOFFS:
            result = evaluation.score_query(rerank_prefix(bm25, reranked, k), qrels, graded)
            for metric in metrics:
                deltas[k][metric].append(getattr(result, metric) - getattr(scores["bm25"], metric))

    counts = Counter(lengths.values())
    return {
        "baseline": {metric: mean(values) for metric, values in baseline.items()},
        "cutoffs": [
            {"k": k, "delta": {metric: mean(values) for metric, values in deltas[k].items()}}
            for k in CUTOFFS
        ],
        "lengths": [
            {
                "tokens": length,
                "query_count": counts[length],
                "means": {
                    system: {
                        metric: mean(by_length[length][system, metric]) for metric in metrics
                    }
                    for system, _ in evaluation.SYSTEMS
                },
            }
            for length in sorted(counts)
        ],
    }


def style_axes(ax):
    ax.spines[["top", "right"]].set_visible(False)
    ax.grid(axis="y", color="#e1e7ee", linewidth=0.8)
    ax.set_axisbelow(True)


def plot_cutoffs(results, output):
    figure, axes = plt.subplots(2, 2, figsize=(9.6, 6.0), layout="constrained")
    for ax, (metric, label) in zip(axes.flat, METRICS.items(), strict=True):
        for (split, result), color, marker in zip(results.items(), COLORS, MARKERS, strict=True):
            if metric in result["baseline"]:
                points = result["cutoffs"]
                ax.plot(
                    [point["k"] for point in points],
                    [point["delta"][metric] for point in points],
                    color=color, marker=marker, markersize=4, label=SPLIT_NAMES[split],
                )
        style_axes(ax)
        ax.axhline(0, color="#657587", linewidth=1, linestyle="--")
        ax.set(title=label, xlabel="BM25 hits reranked (k)", ylabel="Change from BM25",
               xticks=CUTOFFS, xlim=(7, 103))
        ax.legend(frameon=False, fontsize=8)
    figure.savefig(output, dpi=220, bbox_inches="tight")
    plt.close(figure)


def plot_lengths(results, splits, output):
    metrics = ("mrr10", "ap") if splits == ("dev",) else ("mrr10", "ndcg10", "ndcg100")
    figure, panels = plt.subplots(
        len(splits), len(metrics), squeeze=False,
        figsize=(9.6, 3.1 if len(splits) == 1 else 5.8), layout="constrained",
    )
    for split, axes in zip(splits, panels, strict=True):
        rows = results[split]["lengths"]
        low = rows[0]["tokens"]
        high = rows[-1]["tokens"]
        points = {row["tokens"]: row for row in rows}
        for metric, ax in zip(metrics, axes, strict=True):
            for (system, name), color, marker in zip(evaluation.SYSTEMS, COLORS, MARKERS, strict=True):
                lengths = range(low, high + 1)
                values = [
                    points[n]["means"][system][metric]
                    if n in points and points[n]["query_count"] >= 5 else float("nan")
                    for n in lengths
                ]
                ax.plot(lengths, values, label=name, color=color, marker=marker, markersize=3.5)
                sparse = [row for row in rows if row["query_count"] < 5]
                ax.scatter(
                    [row["tokens"] for row in sparse],
                    [row["means"][system][metric] for row in sparse],
                    facecolors="none", edgecolors=color, marker=marker, s=23, zorder=3,
                )
            style_axes(ax)
            ax.set(title=f"{SPLIT_NAMES[split]}  |  {METRICS[metric]}",
                   xlabel="Query length (tokens)", ylim=(-0.03, 1.03))
            ax.xaxis.set_major_locator(MaxNLocator(integer=True, nbins=6))
        axes[-1].legend(frameon=False, fontsize=8, loc="lower left")
    figure.savefig(output, dpi=220, bbox_inches="tight")
    plt.close(figure)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data-dir", type=Path, default=evaluation.REPO / "data/ms_marco")
    parser.add_argument("--runs-dir", type=Path, default=evaluation.REPO / "output/ps3/runs")
    parser.add_argument("--output-dir", type=Path, default=evaluation.REPO / "output/ps3/analysis")
    args = parser.parse_args()
    results = {split: analyze_split(split, args.data_dir, args.runs_dir) for split in evaluation.SPLITS}
    args.output_dir.mkdir(parents=True, exist_ok=True)
    (args.output_dir / "plot-data.json").write_text(
        json.dumps(results, indent=2) + "\n", encoding="utf-8",
    )
    with plt.rc_context({"font.family": "DejaVu Sans", "font.size": 10, "axes.titleweight": "bold"}):
        plot_cutoffs(results, args.output_dir / "reranking-cutoff.png")
        plot_lengths(results, ("eval.one", "eval.two"), args.output_dir / "query-length-trec.png")
        plot_lengths(results, ("dev",), args.output_dir / "query-length-dev.png")
    print(f"Saved three plots and their data to {args.output_dir}")


if __name__ == "__main__":
    main()
