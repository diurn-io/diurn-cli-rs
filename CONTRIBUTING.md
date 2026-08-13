# Contributing

Org-wide notes live in [diurn-io/.github](https://github.com/diurn-io/.github).
These are specific to this repository.

## Working against a local `diurn-mic`

`Cargo.toml` pins `diurn-mic` to a git rev while its types settle. To build
against a local checkout instead, create `.cargo/config.toml`:

```toml
[patch."https://github.com/diurn-io/diurn-mic-rs"]
diurn-mic = { path = "../diurn-mic-rs" }
```

That file is gitignored deliberately. Committing it would break CI, where the
sibling checkout does not exist and Cargo errors on a `[patch]` it cannot
resolve.

The source URL must match the dependency's exactly. Cargo silently ignores a
`[patch]` whose source does not match — no error, no warning, it just builds the
git rev as though the override were absent.

## The offline promise

Every `mic` subcommand except `fetch` works with no network, and that is
enforced rather than assumed:

- `only_fetch_links_the_network` asserts `ureq::` appears in `fetch.rs` and
  nowhere else in `src/`.
- `everything_except_fetch_works_offline` runs each command with
  `DIURN_MIC_URL` pointed at a refused port. `network_failure_is_three` uses the
  same address to confirm `fetch` really does fail against it, so that test is
  not a vacuous pass.

If you add a command that needs the network, it belongs behind an explicit
opt-in and those tests need updating on purpose, not by deletion.

## Nothing is bundled with the binary

There is no registry compiled in, deliberately. One would age with every release
and say nothing about it, and stale market data that looks current is the exact
failure this project exists to prevent. Users fetch a registry, and the command
always reports which file and vintage it used.

Tests point `DIURN_DATA_DIR` at `tests/fixtures/`, which exercises discovery
rather than bypassing it.

## Updating the pinned fixture

`tests/fixtures/ISO10383_MIC_2026-08-10.csv` is what the tests assert against.
Replacing it means updating three things together:

1. The file itself — use `diurn mic fetch`, which names it correctly.
2. The sha256 in `.github/workflows/ci.yml`.
3. The record counts asserted in `tests/cli.rs`.

CI fails on the hash first, which is deliberate: "the fixture changed" is a far
clearer failure than a record count that no longer matches.

## Before opening a pull request

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

Output formats are a compatibility surface. `--format json` field names are
hand-written in `src/render.rs` rather than derived from library types, so that
renaming a field in `diurn-mic` cannot silently change what scripts parse. Treat
a change there as a breaking change.
