# Persisted index compatibility

Indexly 0.1.0's library packaging/API changes do not change the existing engine
or Indexify index bytes. CPU and optional GPU sorting produce the same format.
API versions follow Cargo SemVer; file compatibility follows the explicit
headers and tokenizer identifiers below. Do not infer disk compatibility solely
from the crate version. Future format changes must document supported readers,
an upgrade, or a required rebuild in the changelog. Forward compatibility with
unknown future formats is not promised.

All integer fields are explicitly serialized little-endian, independent of Rust
struct padding. The current final index consists of:

| File | Header | Contents |
| --- | --- | --- |
| `postings.bin` | `IDX1` | Codec ID, block size, block headers, Simple9 document gaps and frequencies |
| `lexicon.bin` | `LEX2` | UTF-8 terms, posting byte ranges, document frequencies, block-directory ranges |
| `blocks.bin` | `BLK1` | 12-byte records: last document ID (`u32`), absolute posting offset (`u64`) |
| `page_table.bin` | `PTB2` | Source path/size/mtime and 20-byte passage ID, token length, offset, row-length records |
| `metadata.bin` | `STA1` | Document count, token count, tokenizer ID and tokenizer version |

The Simple9 codec ID is 1; tokenizer ID/version are 1/1. Index startup validates
file headers, metadata counts, and tokenizer identity. Additional posting and
block validation happens during cursor traversal; opening is not a complete
scan of all posting bytes and is not a security guarantee for arbitrary input.

## Supported existing indexes

- Existing standalone-engine and Indexify indexes with `IDX1`, `LEX2`, `BLK1`,
  `PTB2`, and `STA1` are supported without rebuilding. This repository retains a
  small index produced by the pre-refactor engine as a compatibility fixture.
- Headerless legacy page tables containing 8-byte `(passage_id, token_count)`
  records remain readable for ranking with `SearchOptions { snippets: false,
  ..Default::default() }` or CLI `--no-snippets`, provided the other files and
  collection metadata use supported formats.
- Add text offsets to those legacy page tables using the original TSV:

  ```sh
  search-engine prepare-snippets --index existing-index --dataset passages.tsv
  ```

  The operation checks passage IDs/counts, retains token lengths, and atomically
  replaces the page table. It does not rebuild compressed postings. It does not
  lock an application collection; close readers or use the application's lock.
- Lexicons older than `LEX2` require rebuilding to create block directories.
  Unsupported metadata/posting headers or tokenizer identities are rejected.

## Text-store lifetime and relocation

Index files do not embed full text. `build_index` records the original TSV;
`index_documents` creates `passages.tsv` inside the new index directory. Preserve
the appropriate TSV along with the five binary files. PTB2 records its canonical
absolute path, size, and modification time. Moving an index directory alone does
not update that path; pass the TSV's new path through `SearchOptions::dataset`.

The override checks file size and passage IDs when read, but skips the original
mtime requirement. It is intended for an identical relocated file; it cannot
prove content identity. Never edit the TSV beneath an active index. Rebuild
when passage text or tokenization changes.

Indexify's JSONL source metadata, offset sidecars, snapshot pointers, and locks
are application-owned and are not interpreted by this engine crate. Applications
must coordinate concurrent readers, index deletion, and atomic generation swaps.
