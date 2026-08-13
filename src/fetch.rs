//! `diurn mic fetch` — the only command that touches the network.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use diurn_mic::{LoadOptions, PublishedSource};
use jiff::civil::Date;

use crate::commands::{Ctx, EXIT_LOAD_ERRORS, EXIT_NETWORK, EXIT_OK};
use crate::output::{banner, note, Provenance};
use crate::source;

/// A downloaded vintage smells stale beyond this. ISO publishes monthly, so a
/// current file is at most a few weeks old; five weeks leaves room for a late
/// publication without accepting a date from a previous cycle.
const STALE_AFTER_DAYS: i32 = 35;

/// How the publication date was settled, for reporting.
enum Resolution {
    Explicit,
    FromFile,
    /// Inference produced something implausible, so the schedule was used.
    ScheduledFallback {
        rejected: Date,
    },
    /// No effective date in the file at all.
    Scheduled,
}

impl Resolution {
    fn describe(&self, date: Date) -> String {
        match self {
            Self::Explicit => format!("published {date} (from --published)"),
            Self::FromFile => format!("published {date} (derived from the file's effective date)"),
            Self::Scheduled => {
                format!("published {date} (assumed: second Monday, no effective date in file)")
            }
            Self::ScheduledFallback { rejected } => format!(
                "published {date} (assumed: second Monday; the file implied {rejected}, \
                 which is too old to be this month's vintage)"
            ),
        }
    }
}

/// Download the current registry and write it under a dated filename.
pub fn run(
    w: &mut impl Write,
    ctx: &Ctx,
    out: Option<&Path>,
    published: Option<&str>,
) -> Result<i32> {
    let explicit = published.map(source::parse_date).transpose()?;

    let url = source::fetch_url();
    note!(ctx.quiet, "fetching {url}");
    let bytes = match download(&url) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e:#}");
            return Ok(EXIT_NETWORK);
        }
    };
    note!(ctx.quiet, "received {} bytes", bytes.len());

    let today = today()?;
    let (date, resolution) = resolve_published(&bytes, explicit, today)?;

    // Re-parse with the settled date so pending detection is right.
    let outcome = crate::commands::parse_downloaded(&bytes, LoadOptions::new(date))?;

    // Default to the data directory, so later sessions find this without being
    // told where it went. `--out` still puts it wherever you like.
    let path = match out {
        Some(p) if p.is_dir() => p.join(filename(date)),
        Some(p) => p.to_path_buf(),
        None => {
            let dir = source::data_dir()?;
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("could not create {}", dir.display()))?;
            dir.join(filename(date))
        }
    };
    std::fs::write(&path, &bytes).with_context(|| format!("could not write {}", path.display()))?;

    let prov = Provenance::new(path.display().to_string(), &outcome.registry);
    banner(&prov, ctx.quiet);
    note(ctx.quiet, format_args!("{}", resolution.describe(date)));

    writeln!(w, "{}", path.display())?;

    Ok(if outcome.has_errors() {
        EXIT_LOAD_ERRORS
    } else {
        EXIT_OK
    })
}

pub fn filename(published: Date) -> String {
    format!("ISO10383_MIC_{published}.csv")
}

fn today() -> Result<Date> {
    Ok(jiff::Zoned::now().date())
}

/// Settle the publication date, per SPEC 2.1.
///
/// The library recovers a date from the file without a clock and reports which
/// rule it used. This is the layer that is allowed to know what day it is, so
/// it is where the result gets sanity-checked: a fresh download that claims to
/// have been published months ago has latched onto a stale effective date, the
/// one case calendar arithmetic cannot detect on its own.
fn resolve_published(
    bytes: &[u8],
    explicit: Option<Date>,
    today: Date,
) -> Result<(Date, Resolution)> {
    if let Some(d) = explicit {
        return Ok((d, Resolution::Explicit));
    }

    let probe = crate::commands::parse_downloaded(bytes, LoadOptions::infer())?;
    let inferred = probe.registry.published();

    match probe.registry.published_source() {
        PublishedSource::InferredFromEffectiveDate => {
            let age = today
                .since(inferred)
                .map(|s| s.get_days())
                .unwrap_or(i32::MAX);
            if age <= STALE_AFTER_DAYS {
                Ok((inferred, Resolution::FromFile))
            } else {
                Ok((
                    source::second_monday_of_month(today)?,
                    Resolution::ScheduledFallback { rejected: inferred },
                ))
            }
        }
        // Nothing pending in the file, so there is no effective date to work
        // back from. Fall through to the schedule.
        PublishedSource::LatestUpdateInFile | PublishedSource::Given => Ok((
            source::second_monday_of_month(today)?,
            Resolution::Scheduled,
        )),
    }
}

fn download(url: &str) -> Result<Vec<u8>> {
    let mut resp = ureq::get(url)
        .call()
        .with_context(|| format!("request to {url} failed"))?;

    let status = resp.status();
    if !status.is_success() {
        anyhow::bail!("{url} returned HTTP {status}");
    }

    let body = resp
        .body_mut()
        .with_config()
        // The registry is around 600 KB; the ceiling is generous but bounded,
        // so a misrouted request cannot fill the disk.
        .limit(32 * 1024 * 1024)
        .read_to_vec()
        .context("could not read the response body")?;

    if body.is_empty() {
        anyhow::bail!("{url} returned an empty body");
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;
    use std::path::PathBuf;

    const HEADER: &str = "\"MIC\",\"OPERATING MIC\",\"OPRT/SGMT\",\"MARKET NAME-INSTITUTION DESCRIPTION\",\"LEGAL ENTITY NAME\",\"LEI\",\"MARKET CATEGORY CODE\",\"ACRONYM\",\"ISO COUNTRY CODE (ISO 3166)\",\"CITY\",\"WEBSITE\",\"STATUS\",\"CREATION DATE\",\"LAST UPDATE DATE\",\"LAST VALIDATION DATE\",\"EXPIRY DATE\",\"COMMENTS\"";

    fn csv_with_last_update(d: &str) -> Vec<u8> {
        format!(
            "{HEADER}\n\"XAAA\",\"XAAA\",\"OPRT\",\"A\",,,\"RMKT\",,\"US\",\"NY\",,\"UPDATED\",,\"{d}\",,,\n"
        )
        .into_bytes()
    }

    #[test]
    fn explicit_date_wins() {
        let bytes = csv_with_last_update("20260824");
        let (d, r) = resolve_published(&bytes, Some(date(2001, 1, 1)), date(2026, 8, 13)).unwrap();
        assert_eq!(d, date(2001, 1, 1));
        assert!(matches!(r, Resolution::Explicit));
    }

    #[test]
    fn a_current_effective_date_is_accepted() {
        let bytes = csv_with_last_update("20260824");
        let (d, r) = resolve_published(&bytes, None, date(2026, 8, 13)).unwrap();
        assert_eq!(d, date(2026, 8, 10));
        assert!(matches!(r, Resolution::FromFile));
    }

    /// The case the library cannot catch on its own: a fourth-Monday date left
    /// over from an earlier cycle. Only a clock can reject it.
    #[test]
    fn a_stale_effective_date_is_rejected() {
        let bytes = csv_with_last_update("20260824");
        // Six months later; that August date can no longer be current.
        let today = date(2027, 2, 3);
        let (d, r) = resolve_published(&bytes, None, today).unwrap();
        assert!(matches!(r, Resolution::ScheduledFallback { .. }));
        assert_eq!(d, source::second_monday_of_month(today).unwrap());
        assert_eq!(d, date(2027, 2, 8));
    }

    #[test]
    fn no_effective_date_falls_back_to_the_schedule() {
        // A Tuesday, so not readable as an effective date.
        let bytes = csv_with_last_update("20260616");
        let today = date(2026, 6, 20);
        let (d, r) = resolve_published(&bytes, None, today).unwrap();
        assert!(matches!(r, Resolution::Scheduled));
        assert_eq!(d, date(2026, 6, 8));
    }

    #[test]
    fn filename_follows_the_ingest_convention() {
        assert_eq!(filename(date(2026, 8, 10)), "ISO10383_MIC_2026-08-10.csv");
        // ...and round-trips back through the parser diurn-ops will use.
        let p = PathBuf::from(filename(date(2026, 8, 10)));
        assert_eq!(source::published_from_filename(&p), Some(date(2026, 8, 10)));
    }

    #[test]
    fn resolutions_describe_themselves() {
        let d = date(2026, 8, 10);
        assert!(Resolution::Explicit.describe(d).contains("--published"));
        assert!(Resolution::FromFile.describe(d).contains("derived"));
        assert!(Resolution::Scheduled.describe(d).contains("assumed"));
        let s = Resolution::ScheduledFallback {
            rejected: date(2026, 2, 9),
        }
        .describe(d);
        assert!(s.contains("2026-02-09") && s.contains("too old"));
    }
}
