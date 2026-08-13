//! Command implementations.

use std::io::Write;
use std::path::Path;

use anyhow::{bail, Context, Result};
use diurn_mic::{diff as mic_diff, validate, LoadOptions, MicRecord, MicRegistry, Status};

use crate::cli::{Format, MicCommand};
use crate::output::{banner, note, Provenance, Table};
use crate::render;
use crate::source;

/// Exit codes, per SPEC Phase 2.
pub const EXIT_OK: i32 = 0;
pub const EXIT_USAGE: i32 = 1;
pub const EXIT_LOAD_ERRORS: i32 = 2;
pub const EXIT_NETWORK: i32 = 3;

pub struct Ctx {
    pub format: Format,
    pub quiet: bool,
}

pub fn run(cmd: MicCommand, ctx: &Ctx) -> Result<i32> {
    let stdout = std::io::stdout();
    let mut w = stdout.lock();
    match cmd {
        MicCommand::Load {
            path,
            vintage,
            issues,
        } => load(&mut w, ctx, &path, vintage.published.as_deref(), issues),
        MicCommand::Get {
            mic,
            segments,
            source,
        } => get(
            &mut w,
            ctx,
            &mic,
            segments,
            source.path.as_deref(),
            source.published.as_deref(),
        ),
        MicCommand::List {
            country,
            status,
            category,
            operating,
            pending,
            limit,
            source,
        } => list(
            &mut w,
            ctx,
            ListFilters {
                country,
                status,
                category,
                operating,
                pending,
                limit,
            },
            source.path.as_deref(),
            source.published.as_deref(),
        ),
        MicCommand::Segments { mic, source } => segments(
            &mut w,
            ctx,
            &mic,
            source.path.as_deref(),
            source.published.as_deref(),
        ),
        MicCommand::Validate { path, vintage } => {
            validate_cmd(&mut w, ctx, &path, vintage.published.as_deref())
        }
        MicCommand::Diff { old, new } => diff_cmd(&mut w, ctx, &old, &new),
        MicCommand::Fetch { out, published } => {
            crate::fetch::run(&mut w, ctx, out.as_deref(), published.as_deref())
        }
        MicCommand::Vintages => vintages(&mut w, ctx),
    }
}

/// What is on disk, and which one an unqualified command would pick.
fn vintages(w: &mut impl Write, ctx: &Ctx) -> Result<i32> {
    let dir = source::data_dir()?;
    if let Some(note) = &dir.note {
        note!(ctx.quiet, "note: {note}");
    }
    let found = source::available();

    if found.is_empty() {
        note!(
            ctx.quiet,
            "no registry files in {}\nRun `diurn mic fetch` to download one.",
            dir.path.display()
        );
        // Not an error: an empty data directory is a normal state, and a script
        // asking "what do I have?" deserves an answer rather than a failure.
        return Ok(EXIT_OK);
    }

    note!(ctx.quiet, "{}", dir.path.display());

    #[derive(serde::Serialize)]
    struct VintageView {
        path: String,
        published: Option<String>,
        bytes: u64,
        /// The one an unqualified command uses.
        selected: bool,
    }

    let views: Vec<VintageView> = found
        .iter()
        .enumerate()
        .map(|(i, v)| VintageView {
            path: v.path.display().to_string(),
            published: v.published.map(|d| d.to_string()),
            bytes: v.bytes,
            selected: i == 0,
        })
        .collect();

    match ctx.format {
        Format::Table => {
            let mut t = Table::new(&["", "PUBLISHED", "SIZE", "FILE"]);
            for v in &views {
                t.push(vec![
                    if v.selected {
                        "*".into()
                    } else {
                        String::new()
                    },
                    v.published.clone().unwrap_or_else(|| "unknown".into()),
                    format!("{} KB", v.bytes / 1024),
                    std::path::Path::new(&v.path)
                        .file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_else(|| v.path.clone()),
                ]);
            }
            t.write(w)?;
            note!(
                ctx.quiet,
                "\n* used when no --path is given{}",
                if found.len() > 1 {
                    "; the newest publication date wins"
                } else {
                    ""
                }
            );
        }
        Format::Json => {
            serde_json::to_writer_pretty(&mut *w, &views)?;
            writeln!(w)?;
        }
        Format::Jsonl => {
            for v in &views {
                serde_json::to_writer(&mut *w, v)?;
                writeln!(w)?;
            }
        }
        Format::Csv => {
            let mut wtr = csv::Writer::from_writer(&mut *w);
            for v in &views {
                wtr.serialize(v)?;
            }
            wtr.flush()?;
        }
    }
    Ok(EXIT_OK)
}

fn opt_date(s: Option<&str>) -> Result<Option<jiff::civil::Date>> {
    s.map(source::parse_date).transpose()
}

fn open(
    path: Option<&Path>,
    published: Option<&str>,
    ctx: &Ctx,
) -> Result<(diurn_mic::LoadOutcome, Provenance)> {
    let (outcome, origin) = source::load(path, opt_date(published)?)?;
    let prov = Provenance::new(origin, &outcome.registry);
    banner(&prov, ctx.quiet);
    Ok((outcome, prov))
}

fn load(
    w: &mut impl Write,
    ctx: &Ctx,
    path: &Path,
    published: Option<&str>,
    show_issues: bool,
) -> Result<i32> {
    let (outcome, _) = open(Some(path), published, ctx)?;
    if show_issues {
        render::issues(w, ctx.format, &outcome.issues)?;
    } else {
        render::summary(w, ctx.format, &outcome.registry, &outcome.issues)?;
    }
    Ok(if outcome.has_errors() {
        EXIT_LOAD_ERRORS
    } else {
        EXIT_OK
    })
}

fn validate_cmd(
    w: &mut impl Write,
    ctx: &Ctx,
    path: &Path,
    published: Option<&str>,
) -> Result<i32> {
    let (outcome, _) = open(Some(path), published, ctx)?;

    // Everything the load found, plus anything a fresh structural pass turns
    // up. `validate` re-derives the whole-file checks; the per-row ones can
    // only be observed during parsing.
    let mut all = outcome.issues.clone();
    for issue in validate(&outcome.registry) {
        if !all.contains(&issue) {
            all.push(issue);
        }
    }

    render::issues(w, ctx.format, &all)?;
    Ok(if outcome.has_errors() {
        EXIT_LOAD_ERRORS
    } else {
        EXIT_OK
    })
}

fn parse_mic(s: &str) -> Result<diurn_mic::Mic> {
    diurn_mic::Mic::new(s).with_context(|| format!("{s:?} is not a MIC"))
}

fn get(
    w: &mut impl Write,
    ctx: &Ctx,
    mic: &str,
    with_segments: bool,
    path: Option<&Path>,
    published: Option<&str>,
) -> Result<i32> {
    let code = parse_mic(mic)?;
    let (outcome, _) = open(path, published, ctx)?;
    let registry = &outcome.registry;

    let Some(record) = registry.get(code) else {
        bail!("{code} is not in this vintage");
    };

    if !with_segments {
        render::record_detail(w, ctx.format, registry, record)?;
        return Ok(EXIT_OK);
    }

    let segs: Vec<&MicRecord> = registry
        .segments_of(diurn_mic::OperatingMic::new(code))
        .collect();

    match ctx.format {
        // A single flat list keeps `--format json | jq` predictable: the shape
        // does not change depending on whether --segments was passed.
        Format::Table => {
            render::record_detail(w, ctx.format, registry, record)?;
            writeln!(w)?;
            if segs.is_empty() {
                writeln!(w, "no segments")?;
            } else {
                writeln!(w, "{} segment(s):", segs.len())?;
                render::records(w, ctx.format, registry, &segs)?;
            }
        }
        _ => {
            let mut all = vec![record];
            all.extend(segs);
            render::records(w, ctx.format, registry, &all)?;
        }
    }
    Ok(EXIT_OK)
}

fn segments(
    w: &mut impl Write,
    ctx: &Ctx,
    mic: &str,
    path: Option<&Path>,
    published: Option<&str>,
) -> Result<i32> {
    let code = parse_mic(mic)?;
    let (outcome, _) = open(path, published, ctx)?;
    let registry = &outcome.registry;

    if registry.get(code).is_none() {
        bail!("{code} is not in this vintage");
    }
    let segs: Vec<&MicRecord> = registry
        .segments_of(diurn_mic::OperatingMic::new(code))
        .collect();
    render::records(w, ctx.format, registry, &segs)?;
    Ok(EXIT_OK)
}

pub struct ListFilters {
    pub country: Option<String>,
    pub status: Option<String>,
    pub category: Option<String>,
    pub operating: bool,
    pub pending: bool,
    pub limit: Option<usize>,
}

fn list(
    w: &mut impl Write,
    ctx: &Ctx,
    f: ListFilters,
    path: Option<&Path>,
    published: Option<&str>,
) -> Result<i32> {
    let country = f
        .country
        .as_deref()
        .map(|c| {
            diurn_mic::CountryCode::new(c)
                .with_context(|| format!("{c:?} is not an ISO 3166-1 alpha-2 code"))
        })
        .transpose()?;

    let status = f
        .status
        .as_deref()
        .map(|s| {
            s.parse::<Status>()
                .map_err(|_| anyhow::anyhow!("{s:?} is not a status (active, updated, expired)"))
        })
        .transpose()?;

    // Accept either the code or ISO's full name, so `--category "Regulated
    // Market"` works as well as `--category RMKT`.
    let category = match f.category.as_deref() {
        None => None,
        Some(c) => Some(
            diurn_mic::MarketCategory::new(c)
                .ok()
                .or_else(|| diurn_mic::MarketCategory::from_description(c))
                .with_context(|| format!("{c:?} is not a market category code or name"))?,
        ),
    };

    let (outcome, _) = open(path, published, ctx)?;
    let registry = &outcome.registry;

    let mut selected: Vec<&MicRecord> = match country {
        Some(cc) => registry.by_country(cc).collect(),
        None => registry.iter().collect(),
    };
    selected.retain(|r| {
        status.is_none_or(|s| r.status == s)
            && category.is_none_or(|c| r.category == c)
            && (!f.operating || r.is_operating())
            && (!f.pending || registry.is_pending(r.mic))
    });
    // Stable, predictable ordering; the registry's own order is file order.
    selected.sort_by_key(|r| r.mic);

    let total = selected.len();
    if let Some(n) = f.limit {
        selected.truncate(n);
    }

    render::records(w, ctx.format, registry, &selected)?;

    if total > selected.len() {
        note(
            ctx.quiet,
            format_args!("showing {} of {total} matches", selected.len()),
        );
    }
    Ok(EXIT_OK)
}

fn diff_cmd(w: &mut impl Write, ctx: &Ctx, old: &Path, new: &Path) -> Result<i32> {
    let (old_out, old_origin) = source::load_path(old, None)?;
    let (new_out, new_origin) = source::load_path(new, None)?;

    note(
        ctx.quiet,
        format_args!(
            "{} ({}) -> {} ({})",
            old_origin,
            old_out.registry.published(),
            new_origin,
            new_out.registry.published()
        ),
    );

    if old_out.registry.published() > new_out.registry.published() {
        note(
            ctx.quiet,
            format_args!("note: the first file is the later vintage; arguments may be swapped"),
        );
    }

    let d = mic_diff(&old_out.registry, &new_out.registry);
    render::diff(w, ctx.format, &d)?;

    let errors = old_out.has_errors() || new_out.has_errors();
    Ok(if errors { EXIT_LOAD_ERRORS } else { EXIT_OK })
}

/// `diurn cal` — reserved, not implemented.
///
/// The free half of the CLI advertising the paid half is the funnel; the shape
/// exists from the start so it does not have to be retrofitted.
pub fn cal(quiet: bool) -> Result<i32> {
    let _ = quiet;
    let mut t = Table::new(&["COMMAND", "WHAT IT WILL DO"]);
    for (c, d) in [
        ("diurn cal status <id>", "open or closed at an instant"),
        ("diurn cal next-close <id>", "next session close"),
        ("diurn cal next-open <id>", "next session open"),
        ("diurn cal sessions <id>", "sessions over a date range"),
        ("diurn cal coverage", "which calendars are verified how far"),
    ] {
        t.push(vec![c.to_string(), d.to_string()]);
    }

    let mut err = std::io::stderr().lock();
    writeln!(
        err,
        "calendar commands require DIURN_API_KEY — get one at https://diurn.io\n"
    )?;
    t.write(&mut err)?;
    writeln!(
        err,
        "\nMIC commands need no key and work offline: try `diurn mic --help`."
    )?;
    Ok(EXIT_USAGE)
}

/// Used by `mic fetch` to build a registry from freshly downloaded bytes.
pub fn parse_downloaded(bytes: &[u8], opts: LoadOptions) -> Result<diurn_mic::LoadOutcome> {
    MicRegistry::load_csv(bytes, opts).context("the downloaded file is not an ISO 10383 CSV")
}
