//! `diurn cal`: market calendars and trading hours, from the Diurn API.
//!
//! A thin wrapper over the `diurn` client crate (SPEC 2.1): every request is
//! one of its operations, every answer one of its models, and `--format json`
//! emits that model's own serialization, so the command cannot disagree with
//! a program using the crate. What lives here is only the terminal: parsing
//! arguments into dates and instants, laying answers out as tables and CSV,
//! and turning the client's errors into exit codes.

use std::collections::BTreeMap;
use std::io::{self, Write};

use anyhow::{Context, Result};
use diurn::blocking::Client;
use diurn::models as m;
use diurn::{Error, ExceptionsOptions, JointOptions, SessionsOptions, StepOptions};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};
use serde::Serialize;

use crate::cli::{AsOfArg, CalCommand, Format, Mode, RangeArgs, StepArgs};
use crate::commands::{Ctx, EXIT_NETWORK, EXIT_OK, EXIT_USAGE};
use crate::output::{note, Table};

/// The API refused: no or inactive key, not in the plan, outside its dates,
/// a MIC cap, the rate, or an unknown MIC.
pub const EXIT_REFUSED: i32 = 4;
/// Diurn cannot say whether the market trades then. Never reported as open
/// or closed.
pub const EXIT_INDETERMINATE: i32 = 5;

const SIGNIN: &str = "https://diurn.io/signin/";

pub fn run(cmd: CalCommand, ctx: &Ctx) -> Result<i32> {
    // Without a key every calendar command is refused, so say so before
    // making a request. A server named by DIURN_API_URL may need none (a
    // local one has no key store), so it is asked anyway.
    let has_key = std::env::var("DIURN_API_KEY").is_ok_and(|k| !k.trim().is_empty());
    if !has_key && std::env::var_os("DIURN_API_URL").is_none() {
        eprintln!("calendar commands require DIURN_API_KEY: a free key is at {SIGNIN}");
        eprintln!("MIC commands need no key and work offline: try `diurn mic --help`.");
        return Ok(EXIT_REFUSED);
    }
    let client = match Client::new() {
        Ok(c) => c,
        Err(e) => return Ok(report(&e)),
    };

    let mut out = io::stdout().lock();
    let outcome = dispatch(&client, cmd, ctx.format, &mut out);
    match outcome {
        Ok(code) => {
            out.flush()?;
            meters(&client, ctx.quiet);
            Ok(code)
        }
        Err(e) => match e.downcast_ref::<Error>() {
            Some(api) => Ok(report(api)),
            None => Err(e),
        },
    }
}

fn dispatch(client: &Client, cmd: CalCommand, format: Format, w: &mut impl Write) -> Result<i32> {
    match cmd {
        CalCommand::Status { mics, at, as_of } => {
            let at = instant_opt(at.as_deref())?;
            let as_of = as_of_arg(&as_of)?;
            if let [mic] = mics.as_slice() {
                let r = client.status(mic.as_str(), at, as_of)?;
                let row = StatusRow::from_one(&r);
                status(w, format, &r, None, &[row])
            } else {
                let r = client.statuses(&mics, at, as_of)?;
                let rows: Vec<StatusRow> = r
                    .venues
                    .iter()
                    .map(|(mic, v)| StatusRow::from_venue(mic, v))
                    .collect();
                status(w, format, &r, Some(&r.venues), &rows)
            }
        }
        CalCommand::NextOpen { mic, after, as_of } => {
            let from = instant_or_now(after.as_deref())?;
            let r = client.next_open(mic.as_str(), from, as_of_arg(&as_of)?)?;
            edge(w, format, &mic, "opens", &r)
        }
        CalCommand::NextClose { mic, after, as_of } => {
            let from = instant_or_now(after.as_deref())?;
            let r = client.next_close(mic.as_str(), from, as_of_arg(&as_of)?)?;
            edge(w, format, &mic, "closes", &r)
        }
        CalCommand::Sessions {
            mic,
            range,
            kinds,
            as_of,
        } => {
            let (from, to) = range_of(&range)?;
            let options = SessionsOptions {
                kinds: day_kinds(&kinds)?,
                provenance: false,
                as_of: as_of_arg(&as_of)?,
            };
            let r = client.sessions(mic.as_str(), from, to, options)?;
            days(w, format, &r)
        }
        CalCommand::Day { mic, date, as_of } => {
            let r = client.day(mic.as_str(), date_arg(&date)?, as_of_arg(&as_of)?)?;
            day(w, format, &mic, &r)
        }
        CalCommand::Next(args) => step(client, w, format, args, Direction::Next),
        CalCommand::Previous(args) => step(client, w, format, args, Direction::Previous),
        CalCommand::Count { mic, range, as_of } => {
            let (from, to) = range_of(&range)?;
            let r = client.count_trading_days(mic.as_str(), from, to, as_of_arg(&as_of)?)?;
            count(w, format, &r)
        }
        CalCommand::Exceptions {
            mics,
            range,
            kinds,
            as_of,
        } => {
            let (from, to) = range_of(&range)?;
            let options = ExceptionsOptions {
                kinds: day_kinds(&kinds)?,
                as_of: as_of_arg(&as_of)?,
            };
            let r = client.exceptions(&mics, from, to, options)?;
            exceptions(w, format, &r)
        }
        CalCommand::Usage => {
            let r = client.usage()?;
            usage(w, format, &r)
        }
    }
}

// --- errors ------------------------------------------------------------------------

/// Print what went wrong and how to act on it, and choose the exit code.
fn report(e: &Error) -> i32 {
    eprintln!("error: {e}");
    match e {
        Error::Auth(_) => {
            eprintln!("set DIURN_API_KEY to an active key: a free one is at {SIGNIN}");
            EXIT_REFUSED
        }
        Error::OutOfWindow {
            allowed_from,
            allowed_to,
            ..
        } => {
            let end = |d: &Option<Date>| d.map_or("unbounded".to_string(), |d| d.to_string());
            eprintln!(
                "your plan may ask about {} to {}",
                end(allowed_from),
                end(allowed_to)
            );
            EXIT_REFUSED
        }
        Error::MicCapExceeded {
            mics_in_use,
            resets_at,
            ..
        } => {
            if !mics_in_use.is_empty() {
                eprintln!(
                    "in use: {} (asking again for these is free)",
                    mics_in_use.join(" ")
                );
            }
            if let Some(t) = resets_at {
                eprintln!("a slot frees at {t}");
            }
            EXIT_REFUSED
        }
        Error::RateLimited { retry_after, .. } => {
            if let Some(d) = retry_after {
                eprintln!("retry in {}s", d.as_secs().max(1));
            }
            EXIT_REFUSED
        }
        Error::UnknownVenue(_) | Error::NotInPlan(_) | Error::OverLimit(_) | Error::Api(_) => {
            EXIT_REFUSED
        }
        Error::Indeterminate { .. } => EXIT_INDETERMINATE,
        Error::Transport(_) | Error::Decode(_) | Error::Unavailable(_) => EXIT_NETWORK,
        // BadRequest, InvalidMic, InvalidArgument, and anything newer: the
        // request itself was at fault.
        _ => EXIT_USAGE,
    }
}

/// What the plan has left, from the usage headers on the last response:
/// on stderr, like the MIC commands' vintage banner, so stdout stays data.
fn meters(client: &Client, quiet: bool) {
    let headers = client.last_headers();
    let get = |name: &str| {
        headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    let mut parts = Vec::new();
    if let Some(v) = get("diurn-mics-day") {
        parts.push(format!("MICs today {v}"));
    }
    if let Some(v) = get("diurn-mics-month") {
        parts.push(format!("30 days {v}"));
    }
    if let Some(v) = get("x-diurn-dataset") {
        parts.push(format!("dataset {v}"));
    }
    if !parts.is_empty() {
        note!(quiet, "Diurn API: {}", parts.join(", "));
    }
}

// --- arguments ---------------------------------------------------------------------

fn date_arg(s: &str) -> Result<Date> {
    s.parse::<Date>()
        .with_context(|| format!("not a date: {s:?} (expected YYYY-MM-DD)"))
}

fn today() -> Date {
    Timestamp::now().to_zoned(TimeZone::UTC).date()
}

fn instant_arg(s: &str) -> Result<Timestamp> {
    if s.eq_ignore_ascii_case("now") {
        return Ok(Timestamp::now());
    }
    s.parse::<Timestamp>().with_context(|| {
        format!("not an instant: {s:?} (expected RFC 3339 with an offset or Z, or `now`)")
    })
}

fn instant_opt(s: Option<&str>) -> Result<Option<Timestamp>> {
    s.map(instant_arg).transpose()
}

fn instant_or_now(s: Option<&str>) -> Result<Timestamp> {
    Ok(instant_opt(s)?.unwrap_or_else(Timestamp::now))
}

fn as_of_arg(a: &AsOfArg) -> Result<Option<Timestamp>> {
    instant_opt(a.as_of.as_deref())
}

fn range_of(r: &RangeArgs) -> Result<(Date, Date)> {
    let from = r
        .from
        .as_deref()
        .map(date_arg)
        .transpose()?
        .unwrap_or_else(today);
    let to = match r.to.as_deref() {
        Some(s) => date_arg(s)?,
        None => from.checked_add(30.days()).context("--from is too late")?,
    };
    if to < from {
        anyhow::bail!("--to ({to}) is before --from ({from})");
    }
    Ok((from, to))
}

fn day_kinds(kinds: &[String]) -> Result<Vec<m::DayKindDto>> {
    kinds
        .iter()
        .map(|k| {
            let kind: m::DayKindDto = serde_json::from_value(serde_json::Value::String(k.clone()))
                .with_context(|| format!("not a day kind: {k:?}"))?;
            if matches!(kind, m::DayKindDto::Unrecognised) {
                anyhow::bail!(
                    "not a day kind: {k:?} (open, weekend, holiday, early_close, open_for_some, unknown)"
                );
            }
            Ok(kind)
        })
        .collect()
}

// --- shared rendering ----------------------------------------------------------------

/// An enum's wire spelling (`early_close`), for tables and CSV.
fn label<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// A zoned instant for reading: the venue's own clock and zone.
fn clock(z: &Zoned) -> String {
    z.strftime("%Y-%m-%d %H:%M %Z").to_string()
}

fn opt_clock(z: &Option<Zoned>) -> String {
    z.as_ref().map(clock).unwrap_or_default()
}

/// A day's hours in the venue's clock: `09:30-16:00`, a product's hours with
/// its name, and breaks as separate spans.
fn hours(sessions: &[m::SessionDto]) -> String {
    sessions
        .iter()
        .map(|s| {
            let mut spans = Vec::new();
            let mut start = s.open.clone();
            for b in &s.breaks {
                spans.push(format!(
                    "{}-{}",
                    start.strftime("%H:%M"),
                    b.start.strftime("%H:%M")
                ));
                start = b.end.clone();
            }
            spans.push(format!(
                "{}-{}",
                start.strftime("%H:%M"),
                s.close.strftime("%H:%M")
            ));
            let text = spans.join(", ");
            match &s.products {
                Some(p) => format!("{text} ({p})"),
                None => text,
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn json<T: Serialize>(w: &mut impl Write, value: &T) -> Result<()> {
    serde_json::to_writer_pretty(&mut *w, value)?;
    writeln!(w)?;
    Ok(())
}

fn jsonl<T: Serialize>(w: &mut impl Write, value: &T) -> Result<()> {
    serde_json::to_writer(&mut *w, value)?;
    writeln!(w)?;
    Ok(())
}

fn csv_rows(w: &mut impl Write, headers: &[&str], rows: &[Vec<String>]) -> Result<()> {
    let mut c = csv::Writer::from_writer(w);
    c.write_record(headers)?;
    for row in rows {
        c.write_record(row)?;
    }
    c.flush()?;
    Ok(())
}

fn table(w: &mut impl Write, headers: &[&str], rows: Vec<Vec<String>>) -> Result<()> {
    let mut t = Table::new(headers);
    for row in rows {
        t.push(row);
    }
    t.write(w)
}

/// A value with its MIC beside it, for one JSON line per venue.
#[derive(Serialize)]
struct WithMic<'a, T: Serialize> {
    mic: &'a str,
    #[serde(flatten)]
    value: &'a T,
}

// --- status ------------------------------------------------------------------------

struct StatusRow {
    mic: String,
    state: String,
    kind: String,
    reason: String,
    session: String,
    next_open: String,
    next_close: String,
}

impl StatusRow {
    fn from_one(r: &m::StatusResponse) -> Self {
        Self::new(
            r.mic.clone().unwrap_or_default(),
            &r.state,
            &r.day,
            &r.running,
            &r.next_open,
            &r.next_close,
        )
    }

    fn from_venue(mic: &str, v: &m::VenueStatus) -> Self {
        Self::new(
            mic.to_string(),
            &v.state,
            &v.day,
            &v.running,
            &v.next_open,
            &v.next_close,
        )
    }

    fn new(
        mic: String,
        state: &m::StateDto,
        day: &m::DayDto,
        running: &Option<m::RunningSession>,
        next_open: &Option<Zoned>,
        next_close: &Option<Zoned>,
    ) -> Self {
        Self {
            mic,
            state: label(state),
            kind: label(&day.kind),
            reason: day.reason.clone().unwrap_or_default(),
            session: running
                .as_ref()
                .map(|s| {
                    format!(
                        "{}-{}",
                        s.open.strftime("%H:%M"),
                        s.close.strftime("%H:%M %Z")
                    )
                })
                .unwrap_or_default(),
            next_open: opt_clock(next_open),
            next_close: opt_clock(next_close),
        }
    }

    fn cells(&self) -> Vec<String> {
        vec![
            self.mic.clone(),
            self.state.clone(),
            self.kind.clone(),
            self.session.clone(),
            self.next_open.clone(),
            self.next_close.clone(),
            self.reason.clone(),
        ]
    }
}

/// `venues` is the many-venue answer's map, so JSON Lines gives one line per
/// venue, as for every other list.
fn status<T: Serialize>(
    w: &mut impl Write,
    format: Format,
    response: &T,
    venues: Option<&BTreeMap<String, m::VenueStatus>>,
    rows: &[StatusRow],
) -> Result<i32> {
    const HEADERS: [&str; 7] = [
        "MIC",
        "STATE",
        "DAY",
        "SESSION",
        "NEXT OPEN",
        "NEXT CLOSE",
        "REASON",
    ];
    match format {
        Format::Json => json(w, response)?,
        Format::Jsonl => match venues {
            Some(venues) => {
                for (mic, value) in venues {
                    jsonl(w, &WithMic { mic, value })?;
                }
            }
            None => jsonl(w, response)?,
        },
        Format::Csv => csv_rows(
            w,
            &[
                "mic",
                "state",
                "day",
                "session",
                "next_open",
                "next_close",
                "reason",
            ],
            &rows.iter().map(StatusRow::cells).collect::<Vec<_>>(),
        )?,
        Format::Table => table(w, &HEADERS, rows.iter().map(StatusRow::cells).collect())?,
    }
    // A venue whose state cannot be answered is not reported as either.
    Ok(if rows.iter().any(|r| r.state == "unknown") {
        EXIT_INDETERMINATE
    } else {
        EXIT_OK
    })
}

// --- next open and close -------------------------------------------------------------

fn edge(
    w: &mut impl Write,
    format: Format,
    mic: &str,
    verb: &str,
    r: &m::InstantResponse,
) -> Result<i32> {
    match format {
        Format::Json => json(w, r)?,
        Format::Jsonl => jsonl(w, r)?,
        Format::Csv => csv_rows(
            w,
            &["mic", "from", "at"],
            &[vec![
                mic.to_uppercase(),
                r.from.to_string(),
                r.at.as_ref().map(|z| z.to_string()).unwrap_or_default(),
            ]],
        )?,
        Format::Table => match &r.at {
            Some(z) => writeln!(
                w,
                "{} {verb} {}  ({})",
                mic.to_uppercase(),
                clock(z),
                z.timestamp()
            )?,
            None => writeln!(w, "{} cannot say when it next {verb}", mic.to_uppercase())?,
        },
    }
    Ok(if r.at.is_some() {
        EXIT_OK
    } else {
        EXIT_INDETERMINATE
    })
}

// --- days ----------------------------------------------------------------------------

/// A day's row. Takes the fields, not a type: `DayDto` and `DayResponse` share
/// them but are separate models.
fn row_cells(
    date: Date,
    kind: &m::DayKindDto,
    sessions: &[m::SessionDto],
    reason: &Option<String>,
    verification: &m::VerificationDto,
) -> Vec<String> {
    vec![
        date.to_string(),
        date.strftime("%a").to_string(),
        label(kind),
        hours(sessions),
        reason.clone().unwrap_or_default(),
        label(verification),
    ]
}

fn day_cells(d: &m::DayDto) -> Vec<String> {
    row_cells(d.date, &d.kind, &d.sessions, &d.reason, &d.verification)
}

fn days(w: &mut impl Write, format: Format, r: &m::DaysResponse) -> Result<i32> {
    match format {
        Format::Json => json(w, r)?,
        Format::Jsonl => {
            for d in &r.days {
                jsonl(w, d)?;
            }
        }
        Format::Csv => csv_rows(
            w,
            &["date", "weekday", "kind", "hours", "reason", "verification"],
            &r.days.iter().map(day_cells).collect::<Vec<_>>(),
        )?,
        Format::Table => table(
            w,
            &["DATE", "DAY", "KIND", "HOURS", "REASON", "VERIFICATION"],
            r.days.iter().map(day_cells).collect(),
        )?,
    }
    Ok(EXIT_OK)
}

fn day(w: &mut impl Write, format: Format, mic: &str, r: &m::DayResponse) -> Result<i32> {
    match format {
        Format::Json => json(w, r)?,
        Format::Jsonl => jsonl(w, r)?,
        Format::Csv => {
            let mut cells = row_cells(r.date, &r.kind, &r.sessions, &r.reason, &r.verification);
            cells.insert(0, mic.to_uppercase());
            let (source, retrieved) = evidence(&r.provenance);
            cells.extend([source, retrieved]);
            csv_rows(
                w,
                &[
                    "mic",
                    "date",
                    "weekday",
                    "kind",
                    "hours",
                    "reason",
                    "verification",
                    "source",
                    "retrieved",
                ],
                &[cells],
            )?
        }
        Format::Table => {
            let (source, retrieved) = evidence(&r.provenance);
            let mut t = Vec::new();
            t.push(("MIC", mic.to_uppercase()));
            t.push(("Date", format!("{} ({})", r.date, r.date.strftime("%A"))));
            t.push(("Kind", label(&r.kind)));
            if let Some(reason) = &r.reason {
                t.push(("Reason", reason.clone()));
            }
            if !r.sessions.is_empty() {
                t.push(("Hours", hours(&r.sessions)));
            }
            t.push(("Verification", label(&r.verification)));
            if !source.is_empty() {
                t.push(("Source", source));
            }
            if !retrieved.is_empty() {
                t.push(("Retrieved", retrieved));
            }
            for (k, v) in t {
                writeln!(w, "{k:<14}{v}")?;
            }
        }
    }
    Ok(if r.kind == m::DayKindDto::Unknown {
        EXIT_INDETERMINATE
    } else {
        EXIT_OK
    })
}

fn evidence(p: &Option<m::Provenance>) -> (String, String) {
    match p {
        Some(p) => (
            p.source.clone().unwrap_or_default(),
            p.retrieved.map(|d| d.to_string()).unwrap_or_default(),
        ),
        None => (String::new(), String::new()),
    }
}

// --- trading-day arithmetic ------------------------------------------------------------

#[derive(Copy, Clone)]
enum Direction {
    Next,
    Previous,
}

fn step(
    client: &Client,
    w: &mut impl Write,
    format: Format,
    a: StepArgs,
    dir: Direction,
) -> Result<i32> {
    let from = a
        .from
        .as_deref()
        .map(date_arg)
        .transpose()?
        .unwrap_or_else(today);
    let as_of = as_of_arg(&a.as_of)?;
    let word = match dir {
        Direction::Next => "after",
        Direction::Previous => "before",
    };
    if let [mic] = a.mics.as_slice() {
        let options = StepOptions {
            n: Some(a.n),
            inclusive: a.inclusive,
            on_unknown: Default::default(),
            as_of,
        };
        let r = match dir {
            Direction::Next => client.next_trading_day(mic.as_str(), from, options)?,
            Direction::Previous => client.previous_trading_day(mic.as_str(), from, options)?,
        };
        match format {
            Format::Json => json(w, &r)?,
            Format::Jsonl => jsonl(w, &r)?,
            Format::Csv => csv_rows(
                w,
                &["mic", "from", "n", "date"],
                &[vec![
                    r.mic.clone(),
                    r.from.to_string(),
                    r.n.to_string(),
                    r.date.to_string(),
                ]],
            )?,
            Format::Table => {
                writeln!(
                    w,
                    "{}  {}  ({} trading day{} {word} {})",
                    r.mic,
                    r.date,
                    r.n,
                    if r.n == 1 { "" } else { "s" },
                    r.from
                )?;
                for s in &r.skipped {
                    let why = s
                        .reason
                        .as_deref()
                        .map(|r| format!(": {r}"))
                        .unwrap_or_default();
                    writeln!(w, "  skipped {} {}{why}", s.date, label(&s.kind))?;
                }
            }
        }
    } else {
        let options = JointOptions {
            n: Some(a.n),
            mode: match a.mode {
                Mode::All => diurn::Mode::All,
                Mode::Any => diurn::Mode::Any,
            },
            inclusive: a.inclusive,
            on_unknown: Default::default(),
            as_of,
        };
        let r = match dir {
            Direction::Next => client.joint_next_trading_day(&a.mics, from, options)?,
            Direction::Previous => client.joint_previous_trading_day(&a.mics, from, options)?,
        };
        match format {
            Format::Json => json(w, &r)?,
            Format::Jsonl => jsonl(w, &r)?,
            Format::Csv => csv_rows(
                w,
                &["mics", "mode", "from", "n", "date"],
                &[vec![
                    r.mics.join(" "),
                    r.mode.clone(),
                    r.from.to_string(),
                    r.n.to_string(),
                    r.date.to_string(),
                ]],
            )?,
            Format::Table => {
                writeln!(
                    w,
                    "{}  {}  ({} date{} {word} {} on which {} trade)",
                    r.mics.join(" "),
                    r.date,
                    r.n,
                    if r.n == 1 { "" } else { "s" },
                    r.from,
                    if r.mode == "any" { "any" } else { "all" }
                )?;
                for s in &r.skipped {
                    let closed: BTreeMap<_, _> = s
                        .closed
                        .iter()
                        .map(|(mic, d)| {
                            let why = d.reason.as_deref().unwrap_or(&label(&d.kind)).to_string();
                            (mic.clone(), why)
                        })
                        .collect();
                    let list: Vec<String> = closed
                        .iter()
                        .map(|(mic, why)| format!("{mic} {why}"))
                        .collect();
                    writeln!(w, "  skipped {}: {}", s.date, list.join(", "))?;
                }
            }
        }
    }
    Ok(EXIT_OK)
}

fn count(w: &mut impl Write, format: Format, r: &m::CountResponse) -> Result<i32> {
    match format {
        Format::Json => json(w, r)?,
        Format::Jsonl => jsonl(w, r)?,
        Format::Csv => csv_rows(
            w,
            &["mic", "from", "to", "count", "unknown"],
            &[vec![
                r.mic.clone(),
                r.from.to_string(),
                r.to.to_string(),
                r.count.to_string(),
                r.unknown.to_string(),
            ]],
        )?,
        Format::Table => {
            if r.unknown > 0 {
                writeln!(
                    w,
                    "{}  at least {} trading days, {} to {}; {} cannot be answered",
                    r.mic, r.count, r.from, r.to, r.unknown
                )?;
            } else {
                writeln!(
                    w,
                    "{}  {} trading days, {} to {}",
                    r.mic, r.count, r.from, r.to
                )?;
            }
        }
    }
    // A count with unknown days in it is a lower bound, not an answer.
    Ok(if r.unknown > 0 {
        EXIT_INDETERMINATE
    } else {
        EXIT_OK
    })
}

// --- exceptions ------------------------------------------------------------------------

fn exceptions(w: &mut impl Write, format: Format, r: &m::ExceptionsResponse) -> Result<i32> {
    let cells = |d: &m::VenueDay| {
        vec![
            d.date.to_string(),
            d.mic.clone(),
            label(&d.kind),
            hours(&d.sessions),
            d.reason.clone().unwrap_or_default(),
            label(&d.verification),
        ]
    };
    match format {
        Format::Json => json(w, r)?,
        Format::Jsonl => {
            for d in &r.days {
                jsonl(w, d)?;
            }
        }
        Format::Csv => csv_rows(
            w,
            &["date", "mic", "kind", "hours", "reason", "verification"],
            &r.days.iter().map(cells).collect::<Vec<_>>(),
        )?,
        Format::Table => {
            if r.days.is_empty() {
                writeln!(w, "no exceptions from {} to {}", r.from, r.to)?;
            } else {
                table(
                    w,
                    &["DATE", "MIC", "KIND", "HOURS", "REASON", "VERIFICATION"],
                    r.days.iter().map(cells).collect(),
                )?;
            }
        }
    }
    Ok(EXIT_OK)
}

// --- usage -----------------------------------------------------------------------------

fn usage(w: &mut impl Write, format: Format, r: &m::UsageView) -> Result<i32> {
    let cap = |m: &m::MicWindow| match m.max {
        Some(max) => format!("{}/{max}", m.used),
        None => format!("{}/all", m.used),
    };
    let end = |d: &Option<Date>| d.map_or("unbounded".to_string(), |d| d.to_string());
    match format {
        Format::Json => json(w, r)?,
        Format::Jsonl => jsonl(w, r)?,
        Format::Csv => csv_rows(
            w,
            &["plan", "mics_day", "mics_month", "from", "to", "per_minute"],
            &[vec![
                r.plan.clone(),
                cap(&r.mics_day),
                cap(&r.mics_month),
                end(&r.horizon.from),
                end(&r.horizon.to),
                r.rate.per_minute.to_string(),
            ]],
        )?,
        Format::Table => {
            writeln!(w, "{:<22}{}", "Plan", r.plan)?;
            let in_use = r.mics_day.mics.as_ref().filter(|m| !m.is_empty());
            let listed = in_use
                .map(|m| format!("  ({})", m.join(" ")))
                .unwrap_or_default();
            writeln!(
                w,
                "{:<22}{}{listed}",
                "Unique MICs, 24 hours",
                cap(&r.mics_day)
            )?;
            writeln!(w, "{:<22}{}", "Unique MICs, 30 days", cap(&r.mics_month))?;
            writeln!(
                w,
                "{:<22}{} to {}",
                "Dates",
                end(&r.horizon.from),
                end(&r.horizon.to)
            )?;
            writeln!(
                w,
                "{:<22}{} a minute ({} for many venues at once)",
                "Rate", r.rate.per_minute, r.rate.bulk_per_minute
            )?;
        }
    }
    Ok(EXIT_OK)
}
