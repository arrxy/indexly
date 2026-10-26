# Release preparation

The package name is `indexly`; executable names remain `search-engine`, `search`,
`build_runs`, and `merge_index`. The repository is https://github.com/arrxy/indexly.
The first registry upload has not been performed by this preparation work.

1. Review public API and index-format compatibility, update CHANGELOG.md, and
   run `cargo fmt --check`, `cargo test --locked`, and
   `cargo clippy --locked --all-targets -- -D warnings`.
2. Run `cargo test --locked --features gpu-sort`. Hardware tests are ignored by
   default; run `cargo test --locked --features gpu-sort -- --ignored` on a
   working Metal GPU. Confirm default `cargo tree --edges normal` has no GPU
   dependencies. If changing platform-specific code, check non-macOS CPU builds.
3. Run `cargo doc --locked --no-deps` with `RUSTDOCFLAGS="-D warnings"`.
4. Inspect `cargo package --locked --list`; ensure source, examples, test modules,
   compatibility fixtures, README, license, and format documentation are present.
   Dataset files, generated indexes, caches, and benchmark artifacts must be absent.
5. Run `cargo publish --locked --dry-run` (use `--allow-dirty` only while checking
   uncommitted release work). This packages, extracts, and builds the archive but
   does not upload it. Extract the archive to a new directory, run its tests and
   examples, and compile an independent consumer against that extracted directory.
6. Commit reviewed changes, authenticate with `cargo login`, and publish with
   `cargo publish --locked`. Never put an API token in source or documentation.
   Published version contents cannot be overwritten; fix mistakes with a new
   version. Add a matching Git tag pointing to the released commit.
7. After publication, switch Indexify's dependency to:

   ```toml
   search-engine = { package = "indexly", version = "0.1.0" }
   ```

   Remove the vendored `engine` workspace member, update its lockfile, and run
   Indexify's tests. Keep extraction/collections/TUI in Indexify. Do not release
   an Indexify archive that depends on an unbundled sibling checkout. Publish a
   new Indexify tag and update the Homebrew formula URL/checksum for that tag.

The repository name and crate name match, but availability is only guaranteed
when crates.io accepts the upload. Library API versioning does not replace the
persisted-format compatibility policy in `index-format.md`.
