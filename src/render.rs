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

/// A record flattened for output.
///
/// Hand-written rather than serialising `MicRecord` directly, because the wire
/// shape of a CLI is a compatibility promise of its own: renaming a library
/// field should not silently change what `--format json` emits.
#[derive(Serialize)]
pub struct RecordView<'a> {
    pub mic: &'a str,
    pub operating_mic: &'a str,
    pub kind: &'a str,
    pub kind_name: &'a str,
    pub market_name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub legal_entity_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lei: Option<String>,
    pub category: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_name: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acronym: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub city: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website: Option<&'a str>,
    pub status: &'a str,
    /// True when this record's change is published but not yet in force.
    pub pending: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_validated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<&'a str>,
}

impl<'a> RecordView<'a> {
    pub fn new(r: &'a MicRecord, registry: &MicRegistry) -> Self {
        Self {
            mic: r.mic.as_str(),
            operating_mic: r.operating_mic.as_str(),
            kind: r.kind.as_str(),
            kind_name: r.kind.description(),
            market_name: &r.market_name,
            legal_entity_name: r.legal_entity_name.as_deref(),
            lei: r.lei.map(|l| l.as_str().to_string()),
            category: r.category.as_str(),
            category_name: r.category.description(),
            acronym: r.acronym.as_deref(),
            country: r.country.map(|c| c.as_str().to_string()),
            city: r.city.as_deref(),
            website: r.website.as_deref(),
            status: r.status.as_str(),
            pending: registry.is_pending(r.mic),
            created: r.created.map(|d| d.to_string()),
            last_updated: r.last_updated.map(|d| d.to_string()),
            last_validated: r.last_validated.map(|d| d.to_string()),
            expires: r.expires.map(|d| d.to_string()),
            comments: r.comments.as_deref(),
        }
    }
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

fn write_ndjson<T: Serialize>(w: &mut impl Write, values: &[T]) -> Result<()> {
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
        Format::Ndjson => write_ndjson(w, &views)?,
        Format::Csv => {
            let mut wtr = csv::Writer::from_writer(&mut *w);
            for v in &views {
                wtr.serialize(v)?;
            }
            wtr.flush()?;
        }
        Format::Table => {
            let mut t = Table::new(&["MIC", "OPER", "TYPE", "CC", "STATUS", "CATEGORY", "NAME"]);
            for v in &views {
                t.push(vec![
                    v.mic.to_string(),
                    v.operating_mic.to_string(),
                    v.kind.to_string(),
                    v.country.clone().unwrap_or_else(|| "--".into()),
                    // A pending record is visibly distinct from a settled one.
                    if v.pending {
                        format!("{}*", v.status)
                    } else {
                        v.status.to_string()
                    },
                    v.category.to_string(),
                    clip(v.market_name, 52),
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
        Format::Ndjson => write_ndjson(w, std::slice::from_ref(&v))?,
        Format::Csv => {
            let mut wtr = csv::Writer::from_writer(&mut *w);
            wtr.serialize(&v)?;
            wtr.flush()?;
        }
        Format::Table => {
            let mut field = |k: &str, val: &str| -> Result<()> {
                if !val.is_empty() {
                    writeln!(w, "{k:<18}{val}")?;
                }
                Ok(())
            };
            field("MIC", v.mic)?;
            field("Name", v.market_name)?;
            field("Operating MIC", v.operating_mic)?;
            field("Type", &format!("{} ({})", v.kind, v.kind_name))?;
            field(
                "Category",
                &match v.category_name {
                    Some(n) => format!("{} ({})", v.category, n),
                    None => format!("{} (unrecognised)", v.category),
                },
            )?;
            field("Status", v.status)?;
            if v.pending {
                field(
                    "Pending",
                    &format!(
                        "yes — takes effect {}",
                        v.last_updated.as_deref().unwrap_or("?")
                    ),
                )?;
            }
            field("Country", v.country.as_deref().unwrap_or(""))?;
            field("City", v.city.unwrap_or(""))?;
            field("Legal entity", v.legal_entity_name.unwrap_or(""))?;
            field("LEI", v.lei.as_deref().unwrap_or(""))?;
            field("Acronym", v.acronym.unwrap_or(""))?;
            field("Website", v.website.unwrap_or(""))?;
            field("Created", v.created.as_deref().unwrap_or(""))?;
            field("Last updated", v.last_updated.as_deref().unwrap_or(""))?;
            field("Last validated", v.last_validated.as_deref().unwrap_or(""))?;
            field("Expires", v.expires.as_deref().unwrap_or(""))?;
            field("Comments", v.comments.unwrap_or(""))?;
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
        Format::Ndjson => write_ndjson(w, &views)?,
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
        Format::Ndjson => write_ndjson(w, std::slice::from_ref(&view))?,
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
        Format::Ndjson => {
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
            write_ndjson(w, &rows)?;
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
