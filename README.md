# diurn-cli-rs

The `diurn` command: ISO 10383 Market Identifier Codes and, later, market
calendars. Publishes as the crate
[`diurn-cli`](https://crates.io/crates/diurn-cli) and installs the binary


```sh
cargo install diurn-cli
```

Nothing is bundled with the binary. Fetch the registry once and every later
command finds it on its own:

```console
$ diurn mic fetch
fetching https://www.iso20022.org/sites/default/files/ISO10383_MIC/ISO10383_MIC.csv
received 587384 bytes
ISO 10383 vintage 2026-08-10 — 2875 records, ~/.local/share/diurn/ISO10383_MIC_2026-08-10.csv
published 2026-08-10 (derived from the file's effective date)

$ diurn mic get XLON
MIC               XLON
Name              LONDON STOCK EXCHANGE
Operating MIC     XLON
Type              OPRT (Operating)
Category          RMKT (Regulated Market)
Status            ACTIVE
Country           GB
City              LONDON
Legal entity      LONDON STOCK EXCHANGE PLC
LEI               213800D1EI4B9WTWWD28
Acronym           LSE
Website           WWW.LONDONSTOCKEXCHANGE.COM
Created           2005-06-27
Last updated      2024-10-28
```

## Commands

```
diurn mic get XNYS [--segments]     one record, optionally with its segments
diurn mic list [filters]            filter by country, status, category, ...
diurn mic segments XNYS             what operates under an operating MIC
diurn mic load <path>               parse a file and summarise it
diurn mic validate <path>           report every problem found
diurn mic diff <old> <new>          what changed between two vintages
diurn mic fetch [--out <path>]      download the current registry
diurn mic vintages                  which registry files you have locally
```

Commands that are not given `--path` use the newest registry in the data
directory:

```console
$ diurn mic vintages
~/.local/share/diurn
   PUBLISHED   SIZE    FILE
-  ----------  ------  ---------------------------
*  2026-08-10  573 KB  ISO10383_MIC_2026-08-10.csv
   2026-07-13  572 KB  ISO10383_MIC_2026-07-13.csv

* used when no --path is given; the newest publication date wins
```

The location follows platform convention — `$XDG_DATA_HOME/diurn` or
`~/.local/share/diurn`, `~/Library/Application Support/diurn` on macOS,
`%APPDATA%\diurn` on Windows — and `DIURN_DATA_DIR` overrides it.

To read a particular file rather than the newest, use `--path` (or `-p`):

```console
$ diurn mic get XNYS --path ~/.local/share/diurn/ISO10383_MIC_2026-07-13.csv
```

`--format json|jsonl|csv|table`, defaulting to `table` on a terminal and
`jsonl` when piped — one JSON object per line, with the conventional extension:

```console
$ diurn mic list --country US --format jsonl > us-venues.jsonl
```

Provenance always goes to stderr, so stdout stays clean:

```console
$ diurn mic get XNYS --segments --format json | jq -r '.[].mic'
XNYS
CISD
XCHI
...
```

## Pending records

ISO publishes the registry on the second Monday of each month, and the changes
in that file take effect on the **fourth** Monday. A freshly published file
therefore contains records that are not yet in force:

```console
$ diurn mic list --pending --format table
MIC   OPER  TYPE  CC  STATUS    CATEGORY  NAME
----  ----  ----  --  --------  --------  ------------------------------------
ASPC  INCC  SGMT  CA  ACTIVE*   ATSS      CIX INTELLIGENTCROSS ASPEN
ASPV  INCC  SGMT  CA  ACTIVE*   ATSS      CIX INTELLIGENTCROSS ASPEN VERT
BTAM  BTAM  OPRT  NL  UPDATED*  RMKT      BROKERTEC EU REGULATED MARKET
...
```

The `*` marks a record whose change is published but not yet effective. Note
that `UPDATED` is not the only pending status — plenty of pending records are
`ACTIVE`, so filtering on status alone misses them.

## How the vintage date is derived

The file carries no publication date and the ISO download URL is unversioned,
so the date is worked out rather than read: the latest effective date in the
file is a fourth Monday, and the publication date is a fortnight earlier. The
command always reports which rule it used, because a wrong vintage date quietly
corrupts every pending-record answer. Override it with `--published` if you
know better.

`DIURN_MIC_URL` points the download elsewhere — an internal mirror, or a
refused port when testing the failure path.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Usage error, or the thing you asked for does not exist |
| 2 | The file loaded, but contained records that could not be used |
| 3 | Network failure during `fetch` |

Exit 2 is about the data, not the command: the load still produced a usable
registry, and the unusable rows are reported.

## Data source and attribution

MIC data is published by the ISO 10383 Registration Authority at
<https://www.iso20022.org/market-identifier-codes>, which operates the registry
free of charge. Parsing is done by
[`diurn-mic`](https://github.com/diurn-io/diurn-mic-rs).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
