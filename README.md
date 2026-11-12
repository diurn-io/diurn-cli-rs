# diurn-cli-rs

The `diurn` command: ISO 10383 Market Identifier Codes, offline, and market
calendars and trading hours from the [Diurn API](https://diurn.io). Publishes as the crate
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

## Market calendars

`diurn cal` asks the Diurn API, through the
[`diurn`](https://crates.io/crates/diurn) client crate: every command is one of
its operations, and `--format json` prints its model unchanged. It needs a key,
free at <https://diurn.io/signin/>, in `DIURN_API_KEY`.

```
diurn cal status XNYS [XLON ...] [--at INSTANT]   trading now (or then), next open and close
diurn cal next-open XNYS [--after INSTANT]        the next instant it starts trading
diurn cal next-close XNYS [--after INSTANT]       the next instant it stops
diurn cal sessions XNYS --from DATE --to DATE     every day in a range, with its hours
diurn cal day XNYS DATE                           one day, with the evidence behind it
diurn cal next XNYS [XLON ...] [--from DATE] [-n N]       the nth trading day after
diurn cal previous XNYS [XLON ...] [--from DATE] [-n N]   the nth trading day before
diurn cal count XNYS --from DATE --to DATE        trading days in a range
diurn cal exceptions XNYS [XLON ...] --from DATE --to DATE   holidays and irregular days
diurn cal usage                                   your plan and what is left of it
```

Dates are `YYYY-MM-DD`; instants are RFC 3339 with an offset or `Z`, or `now`.
Times print in the venue's own clock:

```console
$ diurn cal status XNYS --at 2026-11-27T17:00:00Z
MIC   STATE    DAY          SESSION          NEXT OPEN             NEXT CLOSE            REASON
----  -------  -----------  ---------------  --------------------  --------------------  ----------------------
XNYS  running  early_close  09:30-13:00 EST  2026-11-30 09:30 EST  2026-11-27 13:00 EST  Day after Thanksgiving

$ diurn cal next XNYS XLON --from 2026-12-24
XLON XNYS  2026-12-29  (1 date after 2026-12-24 on which all trade)
  skipped 2026-12-25: XLON Christmas Day, XNYS Christmas Day
  skipped 2026-12-28: XLON Boxing Day

$ diurn cal day XNYS 2026-11-26
MIC           XNYS
Date          2026-11-26 (Thursday)
Kind          holiday
Reason        Thanksgiving Day
Verification  verified
Source        https://www.nyse.com/markets/hours-calendars
Retrieved     2026-08-20
```

With several MICs, `next` and `previous` find the date they trade together
(`--mode any` for the first date any one does). What your plan has left goes to
stderr after each answer (`--quiet` silences it). **A day Diurn cannot answer is
never reported as open or closed**: the command says so and exits 5.

`DIURN_API_URL` points the commands at another server, such as a local
`diurn-api`, which needs no key.

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
| 3 | Network failure: `fetch`, or the Diurn API could not be reached |
| 4 | The Diurn API refused: no or inactive key, not in your plan, dates outside it, a MIC cap, the rate, or an unknown MIC |
| 5 | Indeterminate: Diurn cannot say whether the market trades then |

Exit 2 is about the data, not the command: the load still produced a usable
registry, and the unusable rows are reported. Exit 5 is likewise an answer, not
a failure: a count that includes unknown days is printed as a lower bound and
exits 5.

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
