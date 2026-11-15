# PS3 MS MARCO dataset guide

PS3 uses one million MS MARCO passages to compare BM25, FAISS HNSW, and BM25 reranking. Passage IDs connect the original text, the embedding files, the human relevance judgments, and the search results. Query IDs connect the query text, query embeddings, judgments, and results.

All paths in this guide are relative to the repository root. In the examples below, spaces display the tab-separated columns.

## Passages and embeddings

| File | Contents | Used by |
|---|---|---|
| `data/data_set.tsv` | Original PS2 collection. Each row contains `passage_id<TAB>passage_text`. | Subset preparation |
| `data/ms_marco/msmarco_passages_subset.tsv` | Exactly 1,000,000 passage IDs, one per line. | Subset preparation |
| `data/ps3/passages.tsv` | Generated text collection with one row per selected passage. It keeps the original passage IDs and text. | BM25 index build |
| `data/ms_marco/msmarco_passages_embeddings_subset.h5` | 1,000,000 passage embeddings. Each ID identifies a passage in the subset. | HNSW and reranking |
| `data/ms_marco/msmarco_queries_dev_eval_embeddings.h5` | 202,185 query embeddings covering both query text files. | HNSW and reranking |
| `data/ms_marco/read_h5.py` | Course-provided example for loading the HDF5 files. | Inspecting the embeddings |

Each HDF5 file contains an `id` array and an `embedding` matrix. Row `i` of the matrix belongs to the ID at position `i` in the array. Each embedding has 384 components stored as `float16`. The dense search uses raw dot product, and the dense scripts convert vectors to `float32` for indexing or scoring. Join files by ID so each vector stays associated with the right query or passage.

## Query text and judgments

The query text files have two columns, `query_id<TAB>query_text`. The shared `queries.eval.tsv` contains 101,092 rows, while `queries.dev.tsv` contains 101,093. Each evaluation split selects its query IDs from its own qrels file and uses the corresponding query text file as a lookup.

| Split | Query text file | Judgment file | Distinct judged queries | Judgment rows |
|---|---|---|---:|---:|
| `eval.one` (TREC 2019) | `queries.eval.tsv` | `qrels.eval.one.tsv` | 43 | 9,260 |
| `eval.two` (TREC 2020) | `queries.eval.tsv` | `qrels.eval.two.tsv` | 54 | 11,385 |
| `dev` (MS MARCO dev) | `queries.dev.tsv` | `qrels.dev.tsv` | 1,000 | 1,073 |

All five files are under `data/ms_marco/`. One qrels row gives the judgment for **one query and one passage**. The same query ID appears on multiple rows because assessors judged many passages for it. In `eval.one`, each query has 132 to 582 judged passages. The 9,260 rows average about 215 judgments across the 43 queries. Many rows have grade 0 because the judgments include irrelevant passages.

The two TREC judgment files use four columns.

```text
query_id  placeholder  passage_id  relevance_grade
527433    0            8617271     3
```

The placeholder is present in the TREC format. The last column is the grade. **0** means irrelevant, **1** means related, **2** means highly relevant, and **3** means a perfect match. MRR@10 and Recall@100 count grades 2 and 3 as relevant. NDCG@10 and NDCG@100 use the grades as gains. [TREC 2019 guidance](https://trec.nist.gov/data/deep2019.html) · [TREC 2020 guidance](https://trec.nist.gov/data/deep2020.html)

The dev judgment file uses three columns.

```text
query_id  passage_id  relevance_label
24567     7068445     1
```

All 1,073 rows in the supplied dev file have label 1. Some queries have several known relevant passages. The dev evaluation reports MRR@10, Recall@100, and MAP.

## Follow one query through the workflow

1. Find query `527433` in `queries.eval.tsv`. Its text is “types of dysarthria from cerebral palsy.”
2. Select it for `eval.one` because its ID appears in `qrels.eval.one.tsv`. One of its judgments gives passage `8617271` grade 3.
3. BM25 scores the query text against the subset index. HNSW searches using the query embedding. The reranker scores the BM25 candidates using that same query embedding and their passage embeddings.
4. In `output/ps3/runs/bm25.eval.one.tsv`, the row below shows that BM25 ranked this passage first. The evaluator matches the query and passage IDs to the qrels and uses the saved rank to calculate the metrics.

```text
query_id  passage_id  rank  score
527433    8617271     1     36.884516123196505
```

The actual run files are headerless. Each result row gives the original query ID, original passage ID, one-based rank, and the system’s score. The grade comes from qrels and the score comes from the ranking system.

## Scripts that read these files

| Step | Script | Dataset role |
|---|---|---|
| 1 | [Prepare the subset](../scripts/1_prepare_ps3_subset.py) | Filters the original collection by passage ID. |
| 2 | [Build BM25](../scripts/2_build_ps3_bm25.py) | Indexes the filtered passage text. |
| 3 | [Run BM25](../scripts/3_run_ps3_bm25.py) | Takes distinct judged query IDs from each qrels file and looks up their text. |
| 4 | [Run HNSW](../scripts/4_run_ps3_hnsw.py) | Builds and searches the vector index using the supplied embeddings. |
| 5 | [Rerank BM25](../scripts/5_run_ps3_rerank.py) | Uses the BM25 run and both embedding files to rescore candidates. |
| 6 | [Evaluate](../scripts/6_evaluate_ps3.py) | Uses the judgments to score each system’s saved ranking. |
| 7 | [Benchmark](../scripts/7_benchmark_ps3.py) | Measures index build costs and batch costs for the 43 `eval.one` queries. |
| 8 | [Analyze](../scripts/8_analyze_ps3.py) | Uses query text, qrels, and saved rankings to plot reranking depth and query length. |
