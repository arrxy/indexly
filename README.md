# Passage search engine

One executable supports two operations: build a complete index and query it.

## Build an index

```sh
cargo build --release
./target/release/search-engine ./data/data_set.tsv \
  --output ./output/index_01 \
  --sort cpu --threads 8 --merge-threads 2 \
  --batch-bytes 67108864 --merge-fan-in 128 --block-size 128
```

Use a new output directory. The command tokenizes the TSV, builds sorted
runs internally, merges and compresses them, prepares snippet offsets,
and writes the collection statistics. Temporary work is removed on success.
There are no separate run-building, merging, or snippet-preparation commands.

The final directory contains exactly five files:

| File | Contents |
| --- | --- |
| `postings.bin` | Compressed document gaps and term frequencies |
| `blocks.bin` | Last document ID and posting offset for each block |
| `lexicon.bin` | Term locations, document frequencies, and directory ranges |
| `page_table.bin` | Original passage IDs, token counts, and source-row offsets |
| `metadata.bin` | Collection counts and tokenizer identity |

GPU sorting is compiled in by default and requires a suitable Metal device.
Select `--sort gpu` to use it. For a CPU-only build on other platforms:

```sh
cargo build --release --no-default-features
```

Build options (`--help` lists them):

- `--threads`: build workers, default 8.
- `--merge-threads`: merge workers, default 2; capped by the open-file budget.
- `--batch-bytes`: estimated serialized postings per worker, default 67108864.
  This is not a hard memory limit; large documents can exceed the threshold.
- `--merge-fan-in`: inputs per merge, default 64, maximum 128.
- `--block-size`: postings per compressed block, default 128.
- `--sort cpu|gpu`: backend; default GPU when compiled in, otherwise CPU.
- `--keep-intermediate`: retain internal work files for debugging.

## Query

```sh
./target/release/search-engine search --index ./output/index_01
```

```text
OR> rust search
OR> /mode and
AND> rust search
AND> /mode or
OR> /quit
```

The index loads once for the interactive session. `/and` and `/or` are
shortcuts; `/help` shows commands; EOF also exits. Options include
`--mode and|or`, `--k1`, `--b`, `--dataset`, and `--no-snippets`.

The engine returns the top ten exact BM25 results, per-term frequencies and
scores, original passage IDs, and highlighted snippets. Ties favor smaller
internal document IDs. Repeated normalized query terms count once.

Keep the original TSV unchanged and available for snippets. If an identical
copy moves, supply `--dataset <new-path>`. Ranking can run without text using
`--no-snippets`. Rebuild an older index to generate missing passage offsets.

## Internals and assumptions

The tokenizer splits on non-alphanumeric Unicode characters and lowercases
terms. It performs no stemming, stop-word removal, or phrase matching.
AND aligns forward posting cursors; OR visits their union. A per-term block
directory allows direct seeks, and only one block per cursor is decoded at a
time. The dictionary and page table remain resident in memory.

BM25 uses `ln(1 + (N - df + 0.5)/(df + 0.5))`, with defaults `k1=1.2`
and `b=0.75`. All matching candidates are evaluated; there is no score-bound
pruning. Snippets read source text only for the final hits.

The reader trusts indexes produced together by this pipeline. It does not
revalidate posting order, directory boundaries, tokenizer identity, or source
modification times. Writers require sorted unique term/document pairs,
positive frequencies, u32-compatible lengths/counts, and codec-compatible
values. Sorters must supply valid permutations. I/O, format headers, UTF-8,
codec decoding, and source passage-ID checks still propagate errors.

## Verify

```sh
cargo test --locked --offline --no-default-features
cargo test --locked --offline
# Hardware Metal GPU required:
cargo test --locked --offline -- --ignored
```
