//! Turning library values into output.
//!
//! `diurn-mic` deliberately ships no `Display` shaped like a table row and no
//! notion of a terminal (SPEC 1.6). This module is where that boundary is paid
//! for: every column choice, every JSON shape, lives here.

use std::io::Write;

use anyhow::Result;
use diurn_mic::{Issue, MicDiff, MicRecord, MicRegistry, Severity};
use serde::Serialize;

use crate::cli::Format;
use crate::output::Table;

/// A record as the CLI emits it: exactly `MicRecord`'s own serialization, plus
/// the one thing a record cannot know about itself.
///
/// Deliberately a thin wrapper rather than a hand-listed copy of every field.
/// `diurn-api` will serialize the same `MicRecord`, and two hand-maintained
/// field lists would drift — a consumer would then see `GET /v1/venues/XNYS`
/// and `diurn mic get XNYS --format json` disagree for no good reason. Adding a
/// field to the library now flows through here automatically.
#[derive(Serialize)]
pub struct RecordView<'a> {
    #[serde(flatten)]
    record: &'a MicRecord,

    /// Whether this record's change is published but not yet in force.
    ///
    /// Not on `MicRecord`: it is a comparison against the registry's
    /// publication date, which a single record has no access to.
    pending: bool,
}

impl<'a> RecordView<'a> {
    pub fn new(record: &'a MicRecord, registry: &MicRegistry) -> Self {
        Self {
            record,
            pending: registry.is_pending(record.mic),
        }
    }
}

/// CSV columns, in order.
///
/// Written out rather than derived, for two reasons. `serde(flatten)`
/// serializes as a map and the `csv` crate refuses those outright. And column
/// order is part of a CSV's contract in a way it is not for JSON — a consumer
/// indexing by position deserves it to be deliberate rather than an accident of
/// struct layout.
///
/// Keep these in step with the JSON field names above.
const CSV_COLUMNS: [&str; 18] = [
    "mic",
    "operating_mic",
    "kind",
    "market_name",
    "legal_entity_name",
    "lei",
    "category",
    "acronym",
    "country",
    "city",
    "website",
    "status",
    "created",
    "last_updated",
    "last_validated",
    "expires",
    "comments",
    "pending",
];

fn csv_row(v: &RecordView<'_>) -> Vec<String> {
    let r = v.record;
    let date = |d: Option<jiff::civil::Date>| d.map(|d| d.to_string()).unwrap_or_default();
    vec![
        r.mic.to_string(),
        r.operating_mic.to_string(),
        r.kind.to_string(),
        r.market_name.to_string(),
        r.legal_entity_name.as_deref().unwrap_or("").to_string(),
        r.lei.map(|l| l.to_string()).unwrap_or_default(),
        r.category.to_string(),
        r.acronym.as_deref().unwrap_or("").to_string(),
        r.country.map(|c| c.to_string()).unwrap_or_default(),
        r.city.as_deref().unwrap_or("").to_string(),
        r.website.as_deref().unwrap_or("").to_string(),
        r.status.to_string(),
        date(r.created),
        date(r.last_updated),
        date(r.last_validated),
        date(r.expires),
        r.comments.as_deref().unwrap_or("").to_string(),
        v.pending.to_string(),
    ]
}

fn write_csv_records(w: &mut impl Write, views: &[RecordView<'_>]) -> Result<()> {
    let mut wtr = csv::Writer::from_writer(w);
    wtr.write_record(CSV_COLUMNS)?;
    for v in views {
        wtr.write_record(csv_row(v))?;
    }
    wtr.flush()?;
    Ok(())
}

/// Trim a cell so one 102-character market name does not blow out the table.
/// Machine formats never truncate.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let kept: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn write_json<T: Serialize>(w: &mut impl Write, value: &T) -> Result<()> {
    serde_json::to_writer_pretty(&mut *w, value)?;
    writeln!(w)?;
    Ok(())
}

fn write_jsonl<T: Serialize>(w: &mut impl Write, values: &[T]) -> Result<()> {
    for v in values {
        serde_json::to_writer(&mut *w, v)?;
        writeln!(w)?;
    }
    Ok(())
}

/// Render a set of records.
pub fn records(
    w: &mut impl Write,
    format: Format,
    registry: &MicRegistry,
    records: &[&MicRecord],
) -> Result<()> {
    let views: Vec<RecordView> = records
        .iter()
        .map(|r| RecordView::new(r, registry))
        .collect();

    match format {
        Format::Json => write_json(w, &views)?,
        Format::Jsonl => write_jsonl(w, &views)?,
        Format::Csv => write_csv_records(w, &views)?,
        Format::Table => {
            let mut t = Table::new(&["MIC", "OPER", "TYPE", "CC", "STATUS", "CATEGORY", "NAME"]);
            for r in records {
                t.push(vec![
                    r.mic.to_string(),
                    r.operating_mic.to_string(),
                    r.kind.to_string(),
                    r.country
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "--".into()),
                    // A pending record is visibly distinct from a settled one.
                    if registry.is_pending(r.mic) {
                        format!("{}*", r.status)
                    } else {
                        r.status.to_string()
                    },
                    r.category.to_string(),
                    clip(&r.market_name, 52),
                ]);
            }
            if t.is_empty() {
                writeln!(w, "no matching records")?;
            } else {
                t.write(w)?;
            }
        }
    }
    Ok(())
}

/// Render one record in full. Table format goes long rather than wide, since a
/// single record has no columns to align against.
pub fn record_detail(
    w: &mut impl Write,
    format: Format,
    registry: &MicRegistry,
    record: &MicRecord,
) -> Result<()> {
    let v = RecordView::new(record, registry);
    match format {
        Format::Json => write_json(w, &v)?,
        Format::Jsonl => write_jsonl(w, std::slice::from_ref(&v))?,
        Format::Csv => write_csv_records(w, std::slice::from_ref(&v))?,
        Format::Table => {
            let r = record;
            let mut field = |k: &str, val: &str| -> Result<()> {
                if !val.is_empty() {
                    writeln!(w, "{k:<18}{val}")?;
                }
                Ok(())
            };
            let date = |d: Option<jiff::civil::Date>| d.map(|d| d.to_string()).unwrap_or_default();

            field("MIC", r.mic.as_str())?;
            field("Name", &r.market_name)?;
            field("Operating MIC", r.operating_mic.as_str())?;
            // A human reading a table wants the expansion; the machine formats
            // carry only the code, which is what the library and the API speak.
            field("Type", &format!("{} ({})", r.kind, r.kind.description()))?;
            field(
                "Category",
                &match r.category.description() {
                    Some(n) => format!("{} ({n})", r.category),
                    None => format!("{} (unrecognised)", r.category),
                },
            )?;
            field("Status", r.status.as_str())?;
            if registry.is_pending(r.mic) {
                field(
                    "Pending",
                    &format!(
                        "yes — takes effect {}",
                        r.last_updated
                            .map(|d| d.to_string())
                            .unwrap_or_else(|| "?".into())
                    ),
                )?;
            }
            field(
                "Country",
                &r.country.map(|c| c.to_string()).unwrap_or_default(),
            )?;
            field("City", r.city.as_deref().unwrap_or(""))?;
            field("Legal entity", r.legal_entity_name.as_deref().unwrap_or(""))?;
            field("LEI", &r.lei.map(|l| l.to_string()).unwrap_or_default())?;
            field("Acronym", r.acronym.as_deref().unwrap_or(""))?;
            field("Website", r.website.as_deref().unwrap_or(""))?;
            field("Created", &date(r.created))?;
            field("Last updated", &date(r.last_updated))?;
            field("Last validated", &date(r.last_validated))?;
            field("Expires", &date(r.expires))?;
            field("Comments", r.comments.as_deref().unwrap_or(""))?;
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct IssueView {
    severity: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    mic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<u64>,
    message: String,
}

fn severity_str(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Warning => "warning",
        Severity::Error => "error",
    }
}

/// The variant name without its payload, so issues can be counted by kind.
fn kind_name(i: &Issue) -> String {
    let dbg = format!("{:?}", i.kind);
    dbg.split([' ', '(', '{'])
        .next()
        .unwrap_or(&dbg)
        .to_string()
}

fn issue_views(issues: &[Issue]) -> Vec<IssueView> {
    issues
        .iter()
        .map(|i| IssueView {
            severity: severity_str(i.severity).to_string(),
            kind: kind_name(i),
            mic: i.mic.map(|m| m.as_str().to_string()),
            line: i.line,
            message: i.kind.to_string(),
        })
        .collect()
}

/// Every issue, individually.
pub fn issues(w: &mut impl Write, format: Format, issues: &[Issue]) -> Result<()> {
    let views = issue_views(issues);
    match format {
        Format::Json => write_json(w, &views)?,
        Format::Jsonl => write_jsonl(w, &views)?,
        Format::Csv => {
            let mut wtr = csv::Writer::from_writer(&mut *w);
            for v in &views {
                wtr.serialize(v)?;
            }
            wtr.flush()?;
        }
        Format::Table => {
            let mut t = Table::new(&["SEVERITY", "LINE", "MIC", "PROBLEM"]);
            for v in &views {
                t.push(vec![
                    v.severity.clone(),
                    v.line.map(|l| l.to_string()).unwrap_or_else(|| "--".into()),
                    v.mic.clone().unwrap_or_else(|| "--".into()),
                    v.message.clone(),
                ]);
            }
            if t.is_empty() {
                writeln!(w, "no issues")?;
            } else {
                t.write(w)?;
            }
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct SummaryView {
    published: String,
    records: usize,
    errors: usize,
    warnings: usize,
    info: usize,
    pending: usize,
    by_kind: Vec<KindCount>,
}

#[derive(Serialize)]
struct KindCount {
    kind: String,
    severity: String,
    count: usize,
}

/// A count per issue kind, which is what `mic load` reports by default.
pub fn summary(
    w: &mut impl Write,
    format: Format,
    registry: &MicRegistry,
    all: &[Issue],
) -> Result<()> {
    // BTreeMap keeps the output stable between runs, which matters when the
    // command ends up in a CI log that somebody diffs.
    let mut counts: std::collections::BTreeMap<String, (String, usize)> = Default::default();
    for i in all {
        let e = counts
            .entry(kind_name(i))
            .or_insert((severity_str(i.severity).to_string(), 0));
        e.1 += 1;
    }

    let view = SummaryView {
        published: registry.published().to_string(),
        records: registry.len(),
        errors: all.iter().filter(|i| i.severity == Severity::Error).count(),
        warnings: all
            .iter()
            .filter(|i| i.severity == Severity::Warning)
            .count(),
        info: all.iter().filter(|i| i.severity == Severity::Info).count(),
        pending: registry.pending().count(),
        by_kind: counts
            .iter()
            .map(|(k, (sev, n))| KindCount {
                kind: k.clone(),
                severity: sev.clone(),
                count: *n,
            })
            .collect(),
    };

    match format {
        Format::Json => write_json(w, &view)?,
        Format::Jsonl => write_jsonl(w, std::slice::from_ref(&view))?,
        Format::Csv => {
            let mut wtr = csv::Writer::from_writer(&mut *w);
            for k in &view.by_kind {
                wtr.serialize(k)?;
            }
            wtr.flush()?;
        }
        Format::Table => {
            writeln!(w, "vintage    {}", view.published)?;
            writeln!(w, "records    {}", view.records)?;
            writeln!(w, "pending    {}", view.pending)?;
            writeln!(
                w,
                "issues     {} error, {} warning, {} info",
                view.errors, view.warnings, view.info
            )?;
            if !view.by_kind.is_empty() {
                writeln!(w)?;
                let mut t = Table::new(&["COUNT", "SEVERITY", "KIND"]);
                for k in &view.by_kind {
                    t.push(vec![
                        k.count.to_string(),
                        k.severity.clone(),
                        k.kind.clone(),
                    ]);
                }
                t.write(w)?;
            }
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct DiffView {
    added: Vec<String>,
    removed: Vec<String>,
    changed: Vec<ChangedView>,
}

#[derive(Serialize)]
struct ChangedView {
    mic: String,
    changes: Vec<FieldChangeView>,
}

#[derive(Serialize)]
struct FieldChangeView {
    field: String,
    old: Option<String>,
    new: Option<String>,
}

pub fn diff(w: &mut impl Write, format: Format, d: &MicDiff) -> Result<()> {
    let view = DiffView {
        added: d.added.iter().map(|m| m.to_string()).collect(),
        removed: d.removed.iter().map(|m| m.to_string()).collect(),
        changed: d
            .changed
            .iter()
            .map(|c| ChangedView {
                mic: c.mic.to_string(),
                changes: c
                    .changes
                    .iter()
                    .map(|f| FieldChangeView {
                        field: f.field.to_string(),
                        old: f.old.as_ref().map(|s| s.to_string()),
                        new: f.new.as_ref().map(|s| s.to_string()),
                    })
                    .collect(),
            })
            .collect(),
    };

    match format {
        Format::Json => write_json(w, &view)?,
        Format::Jsonl => {
            // One line per affected MIC, so a diff streams like everything else.
            #[derive(Serialize)]
            struct Row<'a> {
                change: &'a str,
                mic: &'a str,
                #[serde(skip_serializing_if = "Option::is_none")]
                fields: Option<&'a [FieldChangeView]>,
            }
            let mut rows: Vec<Row> = Vec::new();
            for m in &view.added {
                rows.push(Row {
                    change: "added",
                    mic: m,
                    fields: None,
                });
            }
            for m in &view.removed {
                rows.push(Row {
                    change: "removed",
                    mic: m,
                    fields: None,
                });
            }
            for c in &view.changed {
                rows.push(Row {
                    change: "changed",
                    mic: &c.mic,
                    fields: Some(&c.changes),
                });
            }
            write_jsonl(w, &rows)?;
        }
        Format::Csv => {
            let mut wtr = csv::Writer::from_writer(&mut *w);
            wtr.write_record(["change", "mic", "field", "old", "new"])?;
            for m in &view.added {
                wtr.write_record(["added", m, "", "", ""])?;
            }
            for m in &view.removed {
                wtr.write_record(["removed", m, "", "", ""])?;
            }
            for c in &view.changed {
                for f in &c.changes {
                    wtr.write_record([
                        "changed",
                        &c.mic,
                        &f.field,
                        f.old.as_deref().unwrap_or(""),
                        f.new.as_deref().unwrap_or(""),
                    ])?;
                }
            }
            wtr.flush()?;
        }
        Format::Table => {
            if view.added.is_empty() && view.removed.is_empty() && view.changed.is_empty() {
                writeln!(w, "no differences")?;
                return Ok(());
            }
            writeln!(
                w,
                "{} added, {} removed, {} changed\n",
                view.added.len(),
                view.removed.len(),
                view.changed.len()
            )?;
            let mut t = Table::new(&["CHANGE", "MIC", "FIELD", "OLD", "NEW"]);
            for m in &view.added {
                t.push(vec![
                    "added".into(),
                    m.clone(),
                    String::new(),
                    String::new(),
                    String::new(),
                ]);
            }
            for m in &view.removed {
                t.push(vec![
                    "removed".into(),
                    m.clone(),
                    String::new(),
                    String::new(),
                    String::new(),
                ]);
            }
            for c in &view.changed {
                for f in &c.changes {
                    t.push(vec![
                        "changed".into(),
                        c.mic.clone(),
                        f.field.clone(),
                        clip(f.old.as_deref().unwrap_or("--"), 30),
                        clip(f.new.as_deref().unwrap_or("--"), 30),
                    ]);
                }
            }
            t.write(w)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_leaves_short_strings_alone() {
        assert_eq!(clip("NYSE", 10), "NYSE");
        assert_eq!(clip("NYSE", 4), "NYSE");
    }

    #[test]
    fn clip_marks_truncation() {
        assert_eq!(clip("NEW YORK STOCK EXCHANGE", 10), "NEW YORK …");
        assert_eq!(clip("NEW YORK STOCK EXCHANGE", 10).chars().count(), 10);
    }

    /// Non-ASCII names must clip on character boundaries, not bytes.
    #[test]
    fn clip_handles_non_ascii() {
        let s = "ČESKOSLOVENSKÁ OBCHODNÍ BANKA";
        assert_eq!(clip(s, 8).chars().count(), 8);
        assert!(clip(s, 8).starts_with('Č'));
    }
}
