# Pre-refactor compatibility fixture

Generated with the unchanged standalone engine at commit `b7551e200c814fb555107ad3a6c28847ee48eb87` before the
Indexly library refactor. The tiny UTF-8 TSV contains four passages, including
Unicode and a passage without tokens. Parameters: CPU sort, one build worker,
one merge worker, 96-byte batch threshold, two postings/block, and fan-in two.

Keep these files fixed. The public-API test opens the old index with an explicit
source override, then compares newly written posting, lexicon, block, and stats
files byte for byte. PTB2 records are compared independently of the source
path/mtime header, which necessarily changes between checkouts.

SHA-256:

- `blocks.bin`: `d0a336c123a4195bd2bb5dd060f7ed209363fe90d8f5c37dea28d05bac1e985d`
- `lexicon.bin`: `19e5ed333e79cbe40ed09ca1ab78b12933e178b171fa7cab21db9206d8b3e74a`
- `metadata.bin`: `e23f677286305ef58f3667138b385dc6e981d452972497a7c67bb4654983e4c4`
- `page_table.bin`: `ffa0c43d266b31a8d499c31b57a29e8c4568f104a04f768a8a30d5eabb761826`
- `passages.tsv`: `6236b3aa2d660c4feaa1c63ecb291be996ef9b2ecb5c3807f37ea0d35630191d`
- `postings.bin`: `e05b8f26966dd19c97aaa7e28cd5ae1fca8491605a61e0a535cb787e72b98c16`
