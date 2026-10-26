# Changelog

## 0.1.0 -- release preparation (unpublished)

- Package the canonical standalone engine as `indexly`, retaining CLI binary names.
- Default to CPU sorting; Metal sorting requires `gpu-sort` and explicit selection.
- Add `build_index`, `index_documents`, `Document`, `IndexOptions`, `BuildReport`,
  `SearchOptions`, root-level retrieval types, and `SearchEngine::open_default`.
- Reconcile Indexify's passage-preview method and strict CLI option validation.
- Reuse one indexing implementation between the CLI and public library API.
- Add MIT licensing, runnable examples, API documentation, a compatibility policy,
  and tests for the public API and pre-refactor index files.
- Preserve existing index headers and binary layout; no index rebuild is required.
- Existing directories are never overwritten; failed high-level builds remove
  their newly created output on a best-effort basis.
