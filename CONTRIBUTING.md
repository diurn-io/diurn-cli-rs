# Contributing

Org-wide notes live in [diurn-io/.github](https://github.com/diurn-io/.github).
These are specific to this repository.

## Working against a local `diurn-mic`

`diurn-mic` comes from crates.io. To build against a sibling checkout instead,
create `.cargo/config.toml`:

```toml
[patch.crates-io]
diurn-mic = { path = "../diurn-mic-rs" }
```

That file is gitignored deliberately. Committing it would break CI, where the
sibling checkout does not exist and Cargo errors on a `[patch]` it cannot
resolve.

Note the section name. While the dependency was a pinned git rev this had to be
`[patch."https://github.com/diurn-io/diurn-mic-rs"]`, matching the source URL
exactly — Cargo silently ignores a `[patch]` whose source does not match, with
no error and no warning. Now that it resolves from the registry, the section is
`[patch.crates-io]`.

Anything relying on an unreleased `diurn-mic` change needs that change published
first; there is no longer a rev to pin.

## The offline promise

Every `mic` subcommand except `fetch` works with no network, and that is
enforced rather than assumed:

- `only_fetch_and_cal_reach_the_network` asserts that `fetch.rs` (the
  registry download, on `reqwest`) and `cal.rs` (the calendar commands, through
  the `diurn` client crate's blocking client) are the only modules in `src/`
  that reach the network, and that `cal.rs` makes no HTTP request of its own:
  it is a thin wrapper over the client crate (SPEC 2.1).
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

## The calendar commands

`diurn cal` is a thin wrapper over the `diurn` client crate: no HTTP and no
request types of its own, so the CLI exercises the library everyone else
integrates against. `tests/cal.rs` runs the binary against a fake API built on
`std::net`, answering with responses recorded from a local `diurn-api`
(`tests/fixtures/api/`); rerecord them with `curl` against
`cargo run -p diurn-api -- --bind 127.0.0.1:8099` in diurn-platform when the
wire format changes.

The `diurn` crate's dependencies include ICU crates whose newest versions need
Rust 1.88, so `Cargo.lock` pins them to the newest that build on this crate's
`rust-version` (1.85). After a `cargo update`, keep it that way:

```sh
CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS=fallback cargo update
cargo +1.85.0 check --locked --all-targets
```
