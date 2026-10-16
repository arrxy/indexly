# Search engine index construction

Build the executable with `cargo build --release`.

The CLI builds sorted runs, merges them, and writes `postings.bin`,
`lexicon.bin`, and `page_table.bin` in a new output directory.

```sh
./target/release/search-engine ./data/data_set.tsv \
  --output ./output/parallel_01 \
  --batch-bytes 67108864 \
  --threads 4 \
  --merge-threads 2
```

## Worker options

- `--threads`: workers for tokenization, frequency counting, sorting, and
  writing runs. Default: **1**, preserving the serial baseline.
- `--merge-threads`: concurrent independent merge groups. Default: **1**.
  The final merge may have only one group, regardless of this setting.
- `--merge-fan-in`: inputs in each merge group, from **2 to 128**, default
  **64**. The merger caps concurrent workers so at most **128 input files**
  are open across its jobs. Output files and standard I/O require additional
  descriptors.

The reader assigns internal document IDs before dispatch. Workers own
separate posting buffers and share a bounded input queue. Page table records
are written at their assigned doc-ID positions, preserving the original
passage mapping even when jobs complete in a different order.

`--batch-bytes` remains a serialized posting-byte threshold **per build
worker**, not a total RAM budget. Actual RAM includes strings, vector spare
capacity, sorting, token counts, and queued passage text. Increasing the
worker count increases memory consumption. The threshold is checked after
processing each passage.

Intermediate runs may differ in count and naming between parallel runs,
but the three final binary index files should match the serial output byte
for byte for the same input, codec, and block size.

## Benchmark independently

Use new output directories and the same input, batch threshold, block size,
and fan-in. Change one worker setting at a time:

```sh
/usr/bin/time -l ./target/release/search-engine ./data/data_set.tsv \
  --output ./output/serial_01 --batch-bytes 67108864 \
  --threads 1 --merge-threads 1

/usr/bin/time -l ./target/release/search-engine ./data/data_set.tsv \
  --output ./output/build_parallel_01 --batch-bytes 67108864 \
  --threads 4 --merge-threads 1

/usr/bin/time -l ./target/release/search-engine ./data/data_set.tsv \
  --output ./output/merge_parallel_01 --batch-bytes 67108864 \
  --threads 4 --merge-threads 2
```

Compare the saved files, not intermediate run names:

```sh
cmp ./output/serial_01/postings.bin ./output/build_parallel_01/postings.bin
cmp ./output/serial_01/lexicon.bin ./output/build_parallel_01/lexicon.bin
cmp ./output/serial_01/page_table.bin ./output/build_parallel_01/page_table.bin
```

## Streaming final merge

Intermediate passes run only while the number of runs exceeds
`--merge-fan-in`. Once the remaining runs fit, the final heap traversal
passes each posting directly to `PostingsWriter::push`. `finish` writes
the last block and lexicon entry and flushes both output files.

There is no final uncompressed `merged.bin` write/read cycle. Even with
`--keep-intermediate`, only source runs and necessary intermediate merge
outputs are retained. Temporary inputs are cleaned up after the compressed
writer finishes successfully.

The program prints build, intermediate merge, final merge + compression,
and total elapsed times. Compare the new combined final-stage time with
the old final merge **plus** compression times, rather than either alone.
`/usr/bin/time -l` additionally reports CPU time and peak resident memory on
macOS. Filesystem caching and competing disk I/O can affect measurements.
Compression remains sequential in this version.

Run correctness and worker error tests with `cargo test --locked --offline`.

## Experimental GPU posting sort (macOS Metal)

Build with `cargo build --release --features gpu-sort`. GPU sorting is
selected with `--sort gpu`; `--sort cpu` is the default even in this build.
A normal `cargo build --release` still builds the CPU-only program without
compiling GPU dependencies. The optional backend uses wgpu and WGSL compute
shaders through Metal. An unavailable GPU produces an error before creating
the output directory; there is no silent CPU fallback.

Each build worker counts frequencies as before. When its batch is ready:

1. The CPU collects unique terms and sorts that vocabulary lexicographically.
2. It assigns **batch-local numeric term IDs** in that order and prepares
   `(term_id, doc_id, original_posting_index)` records.
3. The GPU runs a stable radix sort: document-ID nibbles first, then term-ID
   nibbles. GPU workgroups sort local tiles, scan bucket counts, and scatter
   records into global order.
4. The CPU validates the returned ordering and permutation. The run writer
   follows the sorted original posting indices directly, reading the original
   terms, document IDs, and frequencies without rearranging the posting vector.
   It writes the existing binary run format.

IDs are local to each batch, so they must never be compared between runs.
The saved runs still contain term strings; the merger and compressed index
format are unchanged. Strings, tokenization, term-frequency counting, file
I/O, merging and compression remain on the CPU.

All build workers share one GPU context. CPU dictionary preparation can
run concurrently, but GPU sort submissions are serialized to bound GPU
buffer memory. `--threads 8` still means eight CPU build workers. GPU code
uses additional numeric arrays and staging buffers; `--batch-bytes` is not
a total memory budget. Oversized batches return an error asking for a smaller
threshold rather than exceeding device buffer/dispatch limits.

The log reports the adapter, startup time, and aggregate worker times for
term-ID preparation, waiting for the GPU context, GPU submission/readback/
validation. GPU mode no longer has a CPU posting-reordering step. These
durations can overlap with other workers and **must not be added to estimate
wall time**. GPU time includes host work and readback, not just kernel execution. Total time includes GPU initialization.
The required CPU vocabulary sorting, memory traffic, and synchronization may
outweigh any GPU sorting benefit; this mode is an experiment to measure.

### Compare CPU and GPU

Build once with the feature enabled, then run both modes with the same
parameters and fresh output directories. Run from the repository root:

```sh
cargo build --release --features gpu-sort
mkdir -p benchmarks

/usr/bin/time -l ./target/release/search-engine ./data/data_set.tsv \
  --output ./output/cpu_sort_01 --sort cpu \
  --batch-bytes 67108864 --threads 8 --merge-threads 2 --merge-fan-in 128 \
  > ./benchmarks/cpu_sort_01.log 2> ./benchmarks/cpu_sort_01.time

/usr/bin/time -l ./target/release/search-engine ./data/data_set.tsv \
  --output ./output/gpu_sort_01 --sort gpu \
  --batch-bytes 67108864 --threads 8 --merge-threads 2 --merge-fan-in 128 \
  > ./benchmarks/gpu_sort_01.log 2> ./benchmarks/gpu_sort_01.time

cmp ./output/cpu_sort_01/postings.bin ./output/gpu_sort_01/postings.bin
cmp ./output/cpu_sort_01/lexicon.bin ./output/gpu_sort_01/lexicon.bin
cmp ./output/cpu_sort_01/page_table.bin ./output/gpu_sort_01/page_table.bin
```

For a quick correctness check, use a small TSV before the full collection.
GPU workgroup-boundary, random/full-u32-key, repeated-key, Unicode, and
CPU/GPU end-to-end equivalence tests are available with:

```sh
cargo test --locked --offline --features gpu-sort
cargo test --locked --offline --features gpu-sort -- --ignored
```

The second command explicitly runs hardware-dependent tests. They require a
working Metal GPU and are ignored by the ordinary test suite so CI can run
without GPU access. Feature-disabled builds also test that `--sort gpu`
returns a helpful error without creating output files.

### Direct indexed writes

GPU mode returns `PostingOrder::Indices`, with one original posting index
for each sorted output position. CPU mode returns `PostingOrder::InPlace`
after its existing in-place sort. The writer selects the appropriate traversal
once per batch; both paths use the same binary record serializer.

The GPU readback is still checked for valid numeric key order, unchanged
records, and a complete permutation with no duplicate or missing indices.
The writer does not clone or swap strings in GPU mode. This removes the
previous `reorder` timing; the log now states `writer follows sorted indices
(no posting reorder)`. Indexed writing is included in `Build` time and may
have different cache behavior from sequential writing, so compare total
build and wall times when benchmarking this change.
