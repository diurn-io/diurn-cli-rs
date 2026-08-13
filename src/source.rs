//! Finding a registry file and working out which vintage it is.
//!
//! Nothing is compiled into the binary. A registry the CLI shipped with would
//! age silently with every release, and stale market data that looks current is
//! the failure this whole project exists to avoid. The file is always something
//! the user fetched or supplied, and every command says which one it used.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use diurn_mic::{LoadOptions, LoadOutcome, MicRegistry};
use jiff::civil::Date;

pub const ISO_URL: &str =
    "https://www.iso20022.org/sites/default/files/ISO10383_MIC/ISO10383_MIC.csv";

/// Overrides the download URL.
///
/// Exists so the network failure path is testable — there is otherwise no way
/// to exercise exit code 3 without unplugging something — and incidentally lets
/// an air-gapped site point at an internal mirror.
pub const URL_ENV: &str = "DIURN_MIC_URL";

/// Overrides where fetched registries are kept and looked for.
pub const DATA_DIR_ENV: &str = "DIURN_DATA_DIR";

/// Where `mic fetch` should download from.
pub fn fetch_url() -> String {
    std::env::var(URL_ENV).unwrap_or_else(|_| ISO_URL.to_string())
}

/// Where fetched registries live, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDir {
    pub path: PathBuf,
    /// Set when an environment variable was deliberately not honoured, so the
    /// choice can be explained rather than looking arbitrary.
    pub note: Option<String>,
}

/// Whether a path lies inside a snap's private per-revision tree.
///
/// Snap user data lives at `~/snap/<app>/<revision>/`, and several snaps — VS
/// Code among them — export `XDG_DATA_HOME` pointing there and leak it into any
/// terminal they spawn. The revision number is the problem: it changes when
/// *that* application updates, so anything stored under it silently disappears
/// on an unrelated upgrade.
fn is_snap_private(p: &Path) -> bool {
    p.components().any(|c| c.as_os_str() == "snap")
}

/// Where fetched registries live.
///
/// A *data* directory rather than a cache directory, deliberately: a pinned
/// vintage is the evidence for what was served on a given day, and caches are
/// something the operating system is entitled to delete.
///
/// Follows each platform's convention, and `DIURN_DATA_DIR` overrides all of it.
pub fn data_dir() -> Result<DataDir> {
    let plain = |path: PathBuf| DataDir { path, note: None };

    if let Ok(dir) = std::env::var(DATA_DIR_ENV) {
        if !dir.is_empty() {
            return Ok(plain(PathBuf::from(dir)));
        }
    }

    // Under real snap confinement, HOME is redirected into the snap's own tree
    // and SNAP_REAL_HOME holds the actual one.
    let home = std::env::var_os("SNAP_REAL_HOME")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from);

    if cfg!(target_os = "windows") {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return Ok(plain(PathBuf::from(appdata).join("diurn")));
        }
    } else if cfg!(target_os = "macos") {
        if let Some(h) = home {
            return Ok(plain(h.join("Library/Application Support/diurn")));
        }
    } else {
        let mut note = None;
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
            let xdg = PathBuf::from(xdg);
            if xdg.is_absolute() {
                if !is_snap_private(&xdg) {
                    return Ok(plain(xdg.join("diurn")));
                }
                // Honouring this would put the registry under a path containing
                // a snap revision, and lose it on that app's next update.
                note = Some(format!(
                    "ignoring XDG_DATA_HOME={} — it points inside a snap's \
                     per-revision directory, which would not survive that \
                     application updating. Set {DATA_DIR_ENV} to override.",
                    xdg.display()
                ));
            }
        }
        if let Some(h) = home {
            return Ok(DataDir {
                path: h.join(".local/share/diurn"),
                note,
            });
        }
    }

    Err(anyhow!(
        "could not determine a data directory; set {DATA_DIR_ENV}"
    ))
}

/// Parse `YYYY-MM-DD`.
pub fn parse_date(s: &str) -> Result<Date> {
    s.parse::<Date>()
        .with_context(|| format!("expected a date as YYYY-MM-DD, got {s:?}"))
}

/// Recover the publication date from a filename like
/// `ISO10383_MIC_2026-08-10.csv`, which is the convention `mic fetch` writes
/// and `diurn-ops mic ingest` expects.
pub fn published_from_filename(path: &Path) -> Option<Date> {
    let stem = path.file_stem()?.to_str()?;
    let tail = stem.rsplit('_').next()?;
    tail.parse::<Date>().ok()
}

/// A registry file on disk, with whatever we can tell about it from outside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Vintage {
    pub path: PathBuf,
    /// From the filename, when it follows the convention.
    pub published: Option<Date>,
    /// Modification time, as a fallback ordering for files that do not.
    pub modified: Option<std::time::SystemTime>,
    pub bytes: u64,
}

impl Vintage {
    /// Ordering key: a dated file always beats an undated one, and among dated
    /// files the later publication wins. Modification time only breaks ties.
    fn rank(&self) -> (Option<Date>, Option<std::time::SystemTime>) {
        (self.published, self.modified)
    }
}

/// Every `.csv` in the data directory, newest first.
///
/// An unreadable or absent directory yields an empty list rather than an error:
/// "you have not fetched anything yet" is a normal state, and the caller gives
/// a better message than a bare I/O error would.
pub fn available() -> Vec<Vintage> {
    let Ok(dir) = data_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir.path) else {
        return Vec::new();
    };

    let mut found: Vec<Vintage> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if !path.is_file() || path.extension().is_none_or(|x| x != "csv") {
                return None;
            }
            let meta = e.metadata().ok();
            Some(Vintage {
                published: published_from_filename(&path),
                modified: meta.as_ref().and_then(|m| m.modified().ok()),
                bytes: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                path,
            })
        })
        .collect();

    found.sort_by_key(|v| std::cmp::Reverse(v.rank()));
    found
}

/// Load a file, working out its vintage.
///
/// The date comes from the first of these that answers: an explicit
/// `--published`, the filename, then the file's own contents.
pub fn load_path(path: &Path, published: Option<Date>) -> Result<(LoadOutcome, String)> {
    let file = File::open(path).with_context(|| format!("could not open {}", path.display()))?;

    let opts = match published.or_else(|| published_from_filename(path)) {
        Some(d) => LoadOptions::new(d),
        None => LoadOptions::infer(),
    };

    let outcome = MicRegistry::load_csv(file, opts)
        .with_context(|| format!("could not parse {}", path.display()))?;
    Ok((outcome, path.display().to_string()))
}

/// Load whichever registry the user asked for, or the newest one they have.
pub fn load(path: Option<&Path>, published: Option<Date>) -> Result<(LoadOutcome, String)> {
    match path {
        Some(p) => load_path(p, published),
        None => {
            let newest = available()
                .into_iter()
                .next()
                .ok_or_else(no_registry_available)?;
            load_path(&newest.path, published)
        }
    }
}

/// The message someone sees on a fresh install. It is the first thing a new
/// user will read, so it says exactly what to do next.
fn no_registry_available() -> anyhow::Error {
    let where_ = data_dir()
        .map(|d| d.path.display().to_string())
        .unwrap_or_else(|_| "the data directory".to_string());
    anyhow!(
        "no MIC registry found in {where_}\n\n\
         Run `diurn mic fetch` to download the current one, or point at a file \
         you already have with --path.\n\
         Nothing is bundled with this command on purpose: a built-in copy would \
         go stale without saying so."
    )
}

/// The second Monday of `date`'s month — ISO's scheduled publication day.
///
/// Used only as `mic fetch`'s last resort, when the downloaded file carries no
/// recognisable effective date to work back from.
pub fn second_monday_of_month(date: Date) -> Result<Date> {
    let first = Date::new(date.year(), date.month(), 1)?;
    // Weekday::to_monday_zero_offset: Monday == 0.
    let offset = first.weekday().to_monday_zero_offset();
    let first_monday_day = 1 + ((7 - offset) % 7);
    let day = first_monday_day + 7;
    if day > 28 {
        bail!("no second Monday in {}-{:02}", date.year(), date.month());
    }
    Ok(Date::new(date.year(), date.month(), day)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn reads_the_date_out_of_a_conventional_filename() {
        let p = PathBuf::from("/tmp/ISO10383_MIC_2026-08-10.csv");
        assert_eq!(published_from_filename(&p), Some(date(2026, 8, 10)));
    }

    #[test]
    fn ignores_filenames_that_do_not_carry_one() {
        for name in ["mic.csv", "ISO10383_MIC.csv", "ISO10383_MIC_2026-08.csv"] {
            assert_eq!(
                published_from_filename(&PathBuf::from(name)),
                None,
                "{name}"
            );
        }
    }

    #[test]
    fn second_monday_is_correct_across_a_year() {
        // Known values: August 2026 Mondays are 3, 10, 17, 24, 31.
        assert_eq!(
            second_monday_of_month(date(2026, 8, 1)).unwrap(),
            date(2026, 8, 10)
        );
        // A month starting on a Monday: June 2026 Mondays are 1, 8, 15, 22, 29.
        assert_eq!(
            second_monday_of_month(date(2026, 6, 30)).unwrap(),
            date(2026, 6, 8)
        );
        // A month starting on a Sunday: February 2026 begins on a Sunday.
        assert_eq!(
            second_monday_of_month(date(2026, 2, 5)).unwrap(),
            date(2026, 2, 9)
        );
    }

    /// Every month must have one, and it must land on a Monday in the second
    /// week.
    #[test]
    fn second_monday_holds_for_every_month_this_decade() {
        for year in 2024..=2035 {
            for month in 1..=12 {
                let d = second_monday_of_month(Date::new(year, month, 1).unwrap()).unwrap();
                assert_eq!(d.weekday(), jiff::civil::Weekday::Monday);
                assert_eq!(d.month(), month);
                assert!((8..=14).contains(&d.day()), "{d} is not the second Monday");
            }
        }
    }

    #[test]
    fn data_dir_honours_the_override() {
        temp_env(&[(DATA_DIR_ENV, Some("/tmp/diurn-test-dir"))], || {
            let d = data_dir().unwrap();
            assert_eq!(d.path, PathBuf::from("/tmp/diurn-test-dir"));
            assert!(d.note.is_none());
        });
    }

    /// An empty override must not silently become the current directory.
    #[test]
    fn empty_override_falls_through_to_the_platform_default() {
        temp_env(&[(DATA_DIR_ENV, Some(""))], || {
            let d = data_dir().unwrap();
            assert_ne!(d.path, PathBuf::from(""));
            assert!(d.path.ends_with("diurn"), "{d:?}");
        });
    }

    #[test]
    fn snap_private_paths_are_recognised() {
        assert!(is_snap_private(Path::new(
            "/home/ariza/snap/code/253/.local/share"
        )));
        assert!(is_snap_private(Path::new("/var/snap/foo/current")));
        assert!(!is_snap_private(Path::new("/home/ariza/.local/share")));
        // "snapshot" is not "snap".
        assert!(!is_snap_private(Path::new("/home/ariza/snapshots/data")));
    }

    /// A legitimate XDG_DATA_HOME is honoured.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    #[test]
    fn xdg_data_home_is_used_when_it_is_the_user_s_own() {
        temp_env(
            &[
                (DATA_DIR_ENV, None),
                ("XDG_DATA_HOME", Some("/home/someone/.local/share")),
            ],
            || {
                let d = data_dir().unwrap();
                assert_eq!(d.path, PathBuf::from("/home/someone/.local/share/diurn"));
                assert!(d.note.is_none());
            },
        );
    }

    /// The VS Code snap exports XDG_DATA_HOME into its integrated terminal,
    /// pointing at `~/snap/code/<revision>/`. Storing registries there loses
    /// them the next time VS Code updates, so it is deliberately not honoured —
    /// and the CLI says so rather than appearing to ignore the environment at
    /// random.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    #[test]
    fn a_snap_hijacked_xdg_data_home_is_declined_with_an_explanation() {
        temp_env(
            &[
                (DATA_DIR_ENV, None),
                (
                    "XDG_DATA_HOME",
                    Some("/home/ariza/snap/code/253/.local/share"),
                ),
            ],
            || {
                let d = data_dir().unwrap();
                assert!(
                    !is_snap_private(&d.path),
                    "must not land inside the snap tree: {d:?}"
                );
                assert!(d.path.ends_with(".local/share/diurn"), "{d:?}");

                let note = d.note.expect("the override must be explained");
                assert!(note.contains("XDG_DATA_HOME"));
                assert!(note.contains("snap"));
                assert!(note.contains(DATA_DIR_ENV), "must offer a way out");
            },
        );
    }

    /// An explicit override wins even over the snap heuristic — if someone
    /// really wants that directory, they can have it.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    #[test]
    fn explicit_override_beats_the_snap_heuristic() {
        temp_env(
            &[
                (DATA_DIR_ENV, Some("/home/ariza/snap/code/253/diurn")),
                (
                    "XDG_DATA_HOME",
                    Some("/home/ariza/snap/code/253/.local/share"),
                ),
            ],
            || {
                let d = data_dir().unwrap();
                assert_eq!(d.path, PathBuf::from("/home/ariza/snap/code/253/diurn"));
                assert!(d.note.is_none());
            },
        );
    }

    /// Dated files outrank undated ones; later dates win.
    #[test]
    fn newest_vintage_sorts_first() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "ISO10383_MIC_2026-06-08.csv",
            "ISO10383_MIC_2026-08-10.csv",
            "ISO10383_MIC_2026-07-13.csv",
            "some-other-export.csv",
            "notes.txt",
        ] {
            std::fs::write(dir.path().join(name), "x").unwrap();
        }

        temp_env(
            &[(DATA_DIR_ENV, Some(dir.path().to_str().unwrap()))],
            || {
                let found = available();
                // The .txt is ignored; the four CSVs remain.
                assert_eq!(found.len(), 4);
                assert_eq!(found[0].published, Some(date(2026, 8, 10)));
                assert_eq!(found[1].published, Some(date(2026, 7, 13)));
                assert_eq!(found[2].published, Some(date(2026, 6, 8)));
                // Undated sorts last, however recently it was written.
                assert_eq!(found[3].published, None);
            },
        );
    }

    #[test]
    fn a_missing_directory_is_empty_not_an_error() {
        temp_env(&[(DATA_DIR_ENV, Some("/nonexistent/diurn/xyz"))], || {
            assert!(available().is_empty());
        });
    }

    /// The first thing a new user reads must tell them what to do.
    #[test]
    fn the_empty_state_message_is_actionable() {
        temp_env(&[(DATA_DIR_ENV, Some("/nonexistent/diurn/xyz"))], || {
            let msg = no_registry_available().to_string();
            assert!(msg.contains("diurn mic fetch"));
            assert!(msg.contains("--path"));
            assert!(msg.contains("/nonexistent/diurn/xyz"));
        });
    }

    /// Set several environment variables for the duration of a closure.
    ///
    /// Environment is process-global, so these tests must not interleave. Takes
    /// every variable at once rather than supporting nested calls: the lock is
    /// a plain `Mutex`, so a nested acquisition would deadlock rather than fail.
    fn temp_env(vars: &[(&str, Option<&str>)], f: impl FnOnce()) {
        use std::sync::Mutex;
        static LOCK: Mutex<()> = Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

        let previous: Vec<(String, Option<std::ffi::OsString>)> = vars
            .iter()
            .map(|(k, _)| (k.to_string(), std::env::var_os(k)))
            .collect();

        for (k, v) in vars {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

        for (k, v) in previous {
            match v {
                Some(v) => std::env::set_var(&k, v),
                None => std::env::remove_var(&k),
            }
        }
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }
}
