//! Format resolution and the shared rendering helpers.

use std::io::{self, IsTerminal, Write};

use anyhow::Result;
use diurn_mic::{MicRegistry, PublishedSource};

use crate::cli::Format;

impl Format {
    /// Pick a format when the user did not name one.
    ///
    /// A terminal gets aligned columns; anything else gets JSON Lines, because the
    /// overwhelmingly likely reason stdout is not a terminal is that something
    /// else is about to read it.
    pub fn resolve(requested: Option<Format>) -> Format {
        requested.unwrap_or_else(|| {
            if io::stdout().is_terminal() {
                Format::Table
            } else {
                Format::Jsonl
            }
        })
    }
}

/// Where the registry came from, for the banner.
pub struct Provenance {
    pub origin: String,
    pub published: jiff::civil::Date,
    pub source: PublishedSource,
    pub records: usize,
}

impl Provenance {
    pub fn new(origin: impl Into<String>, registry: &MicRegistry) -> Self {
        Self {
            origin: origin.into(),
            published: registry.published(),
            source: registry.published_source(),
            records: registry.len(),
        }
    }
}

/// Announce which vintage produced the output that follows.
///
/// Always on stderr, never stdout — `diurn mic get XNYS --format json | jq`
/// must see nothing but JSON, and a human still needs to know how stale the
/// data is. `--quiet` suppresses it for scripts that find it noisy.
pub fn banner(p: &Provenance, quiet: bool) {
    if quiet {
        return;
    }
    let derivation = match p.source {
        PublishedSource::Given => "",
        PublishedSource::InferredFromEffectiveDate => " (date inferred from file)",
        PublishedSource::LatestUpdateInFile => " (date is the latest in file; no pending records)",
    };
    eprintln!(
        "ISO 10383 vintage {} — {} records, {}{}",
        p.published, p.records, p.origin, derivation
    );
}

/// Print a note to stderr unless silenced.
pub fn note(quiet: bool, args: std::fmt::Arguments<'_>) {
    if !quiet {
        eprintln!("{args}");
    }
}

/// `note!(quiet, "...", ..)` — the formatting sugar over [`note()`].
macro_rules! note_fmt {
    ($quiet:expr, $($arg:tt)*) => {
        $crate::output::note($quiet, format_args!($($arg)*))
    };
}
pub(crate) use note_fmt as note;

/// A table with right-sized columns.
///
/// Hand-rolled rather than pulled from a crate: the data contains no
/// double-width characters — verified against the pinned vintage, which holds
/// only Latin-1 accents and a few punctuation marks — so a character count is a
/// correct display width and the whole job is about thirty lines.
pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(headers: &[&str]) -> Self {
        Self {
            headers: headers.iter().map(|h| h.to_string()).collect(),
            rows: Vec::new(),
        }
    }

    pub fn push(&mut self, row: Vec<String>) {
        debug_assert_eq!(row.len(), self.headers.len(), "row width must match header");
        self.rows.push(row);
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn write(&self, w: &mut impl Write) -> Result<()> {
        let mut widths: Vec<usize> = self.headers.iter().map(|h| h.chars().count()).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }

        let line = |w: &mut dyn Write, cells: &[String]| -> Result<()> {
            let mut out = String::new();
            for (i, cell) in cells.iter().enumerate() {
                if i > 0 {
                    out.push_str("  ");
                }
                out.push_str(cell);
                // No trailing whitespace on the last column.
                if i + 1 < cells.len() {
                    let pad = widths[i].saturating_sub(cell.chars().count());
                    out.extend(std::iter::repeat_n(' ', pad));
                }
            }
            writeln!(w, "{}", out.trim_end())?;
            Ok(())
        };

        line(w, &self.headers)?;
        let rule: Vec<String> = widths.iter().map(|n| "-".repeat(*n)).collect();
        line(w, &rule)?;
        for row in &self.rows {
            line(w, row)?;
        }
        Ok(())
    }
}
