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
