//! End-to-end tests: run the real binary, inspect what it emits.
//!
//! These cover the Phase 2 acceptance criteria. They invoke the compiled
//! command rather than calling into library functions, because exit codes,
//! stream separation, and format defaults are only observable from outside.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_diurn");

const URL_ENV: &str = "DIURN_MIC_URL";
const DATA_DIR_ENV: &str = "DIURN_DATA_DIR";

/// The pinned vintage. Nothing is bundled with the binary, so tests point the
/// data directory here — which exercises discovery rather than bypassing it.
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn snapshot() -> PathBuf {
    fixture_dir().join("ISO10383_MIC_2026-08-10.csv")
}

/// Run with the fixture directory as the data directory, so commands given no
/// `--path` discover the pinned vintage.
fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .env(DATA_DIR_ENV, fixture_dir())
        // Never let a stray override leak in from the developer's shell.
        .env_remove(URL_ENV)
        .output()
        .expect("failed to run diurn")
}

/// Run with an empty data directory, for the no-registry-yet paths.
fn run_bare(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .env(DATA_DIR_ENV, dir)
        .env_remove(URL_ENV)
        .output()
        .expect("failed to run diurn")
}

fn code(o: &Output) -> i32 {
    o.status.code().expect("process was signalled")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

// ---------------------------------------------------------------- formats --

/// Acceptance: `diurn mic get XNYS --segments --format json | jq` is valid JSON.
#[test]
fn get_with_segments_emits_valid_json() {
    let o = run(&["mic", "get", "XNYS", "--segments", "--format", "json"]);
    assert_eq!(code(&o), 0);

    let v: serde_json::Value =
        serde_json::from_str(&stdout(&o)).expect("stdout must be valid JSON");
    let arr = v.as_array().expect("an array of records");
    assert!(arr.len() > 1, "XNYS has segments");
    assert_eq!(arr[0]["mic"], "XNYS");
    assert_eq!(arr[0]["kind"], "OPRT");
    // ISO's readable names ride along, so a consumer never has to hold a copy
    // of the code list.
    assert_eq!(arr[0]["kind_name"], "Operating");
    assert_eq!(arr[0]["category_name"], "Not Specified");
    assert!(arr[1..].iter().all(|r| r["kind"] == "SGMT"));
}

/// The banner must not reach stdout, or it would corrupt every machine format.
#[test]
fn provenance_goes_to_stderr_only() {
    let o = run(&["mic", "get", "XNYS", "--format", "json"]);
    assert!(stderr(&o).contains("vintage 2026-08-10"));
    assert!(!stdout(&o).contains("vintage"));
    serde_json::from_str::<serde_json::Value>(&stdout(&o)).expect("clean JSON");
}

#[test]
fn quiet_silences_the_banner() {
    let o = run(&["mic", "get", "XNYS", "--format", "json", "--quiet"]);
    assert_eq!(code(&o), 0);
    assert_eq!(stderr(&o), "");
}

/// Not a terminal, so the default is the streaming format.
#[test]
fn default_format_when_piped_is_ndjson() {
    let o = run(&["mic", "list", "--country", "JP", "--quiet"]);
    let out = stdout(&o);
    let lines: Vec<_> = out.lines().collect();
    assert!(lines.len() > 10);
    for line in &lines {
        serde_json::from_str::<serde_json::Value>(line).expect("every line is a JSON object");
    }
}

#[test]
fn csv_output_has_a_header() {
    let o = run(&["mic", "get", "XNYS", "--format", "csv", "--quiet"]);
    let out = stdout(&o);
    let mut lines = out.lines();
    assert!(lines.next().unwrap().starts_with("mic,operating_mic,kind"));
    assert!(lines.next().unwrap().starts_with("XNYS,XNYS,OPRT"));
}

#[test]
fn table_output_is_aligned_and_labelled() {
    let o = run(&["mic", "get", "XNYS", "--format", "table", "--quiet"]);
    let out = stdout(&o);
    assert!(out.contains("MIC               XNYS"));
    assert!(out.contains("Type              OPRT (Operating)"));
    assert!(out.contains("Category          NSPD (Not Specified)"));
}

// --------------------------------------------------------------- querying --

/// Acceptance: `diurn mic list --country JP --status active` returns plausible
/// venues.
#[test]
fn list_filters_by_country_and_status() {
    let o = run(&[
        "mic",
        "list",
        "--country",
        "JP",
        "--status",
        "active",
        "--quiet",
    ]);
    assert_eq!(code(&o), 0);

    let rows: Vec<serde_json::Value> = stdout(&o)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();

    assert!(rows.len() > 20, "Japan has many venues, got {}", rows.len());
    assert!(rows.iter().all(|r| r["country"] == "JP"));
    assert!(rows.iter().all(|r| r["status"] == "ACTIVE"));
    // The Tokyo Stock Exchange should be in there.
    assert!(rows.iter().any(|r| r["mic"] == "XTKS"));
}

#[test]
fn list_filters_by_category_code_or_name() {
    let by_code = run(&["mic", "list", "--category", "TRFS", "--quiet"]);
    let by_name = run(&[
        "mic",
        "list",
        "--category",
        "Trade Reporting Facility",
        "--quiet",
    ]);
    assert_eq!(code(&by_code), 0);
    assert_eq!(code(&by_name), 0);
    assert_eq!(stdout(&by_code), stdout(&by_name));
    assert_eq!(stdout(&by_code).lines().count(), 5);
}

#[test]
fn list_operating_excludes_segments() {
    let o = run(&["mic", "list", "--country", "US", "--operating", "--quiet"]);
    let rows: Vec<serde_json::Value> = stdout(&o)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(!rows.is_empty());
    assert!(rows.iter().all(|r| r["kind"] == "OPRT"));
}

/// The pending flag surfaces the publication cycle at the command line.
#[test]
fn list_pending_finds_the_records_not_yet_in_force() {
    let o = run(&["mic", "list", "--pending", "--quiet"]);
    let rows: Vec<serde_json::Value> = stdout(&o)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 35, "the 2026-08-10 vintage has 35 pending");
    assert!(rows.iter().all(|r| r["pending"] == true));
    assert!(rows.iter().all(|r| r["last_updated"] == "2026-08-24"));
}

#[test]
fn segments_of_an_operating_mic() {
    let o = run(&["mic", "segments", "XNYS", "--quiet"]);
    assert_eq!(code(&o), 0);
    let rows: Vec<serde_json::Value> = stdout(&o)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(rows.iter().any(|r| r["mic"] == "ARCX"));
    assert!(rows.iter().all(|r| r["operating_mic"] == "XNYS"));
}

#[test]
fn load_summarises_a_clean_vintage() {
    let path = snapshot();
    let o = run(&[
        "mic",
        "load",
        path.to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    assert_eq!(code(&o), 0);

    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_eq!(v["records"], 2875);
    assert_eq!(v["errors"], 0);
    assert_eq!(v["pending"], 35);
    assert_eq!(v["published"], "2026-08-10");
}

/// The date comes off the filename, so no flag is needed for a conventionally
/// named file.
#[test]
fn vintage_is_read_from_the_filename() {
    let o = run(&["mic", "load", snapshot().to_str().unwrap()]);
    assert!(stderr(&o).contains("vintage 2026-08-10"));
    assert!(
        !stderr(&o).contains("inferred"),
        "a conventional filename should not need inference"
    );
}

// ------------------------------------------------------------------- diff --

/// Build a plausible prior vintage from the pinned one, rather than committing
/// a second 587 KB file that differs in three rows.
fn prior_vintage(dir: &Path) -> PathBuf {
    let src = std::fs::read_to_string(snapshot()).unwrap();
    let mut out = String::with_capacity(src.len());
    for (i, line) in src.lines().enumerate() {
        if i == 0 {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if line.starts_with("\"XCHI\",") {
            // Before the rename: NYSE Chicago, in Chicago.
            out.push_str(
                &line
                    .replace("NYSE TEXAS, INC.", "NYSE CHICAGO, INC.")
                    .replace("\"DALLAS\"", "\"CHICAGO\""),
            );
            out.push('\n');
        } else if line.starts_with("\"CRYP\",") {
            // Absent from the older file, so it shows up as added.
            continue;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    // ...and one that is gone by August.
    out.push_str("\"ZTST\",\"ZTST\",\"OPRT\",\"RETIRED TEST VENUE\",,,\"OTHR\",,\"US\",\"NEW YORK\",,\"ACTIVE\",\"20200101\",\"20200101\",,,\n");

    let path = dir.join("ISO10383_MIC_2026-07-13.csv");
    std::fs::write(&path, out).unwrap();
    path
}

/// Acceptance: diff on two adjacent vintages reports the XCHI rename.
#[test]
fn diff_reports_the_xchi_rename() {
    let dir = tempfile::tempdir().unwrap();
    let old = prior_vintage(dir.path());

    let o = run(&[
        "mic",
        "diff",
        old.to_str().unwrap(),
        snapshot().to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    assert_eq!(code(&o), 0);

    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();

    let changed = v["changed"].as_array().unwrap();
    let xchi = changed
        .iter()
        .find(|c| c["mic"] == "XCHI")
        .expect("XCHI must be reported as changed");

    let fields: Vec<&str> = xchi["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["field"].as_str().unwrap())
        .collect();
    assert!(fields.contains(&"market_name"));
    assert!(fields.contains(&"city"));

    let name = xchi["changes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["field"] == "market_name")
        .unwrap();
    assert_eq!(name["old"], "NYSE CHICAGO, INC.");
    assert_eq!(name["new"], "NYSE TEXAS, INC.");

    assert_eq!(v["added"].as_array().unwrap(), &vec!["CRYP"]);
    assert_eq!(v["removed"].as_array().unwrap(), &vec!["ZTST"]);
}

#[test]
fn diff_of_a_file_against_itself_is_empty() {
    let p = snapshot();
    let s = p.to_str().unwrap();
    let o = run(&["mic", "diff", s, s, "--format", "json", "--quiet"]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert!(v["added"].as_array().unwrap().is_empty());
    assert!(v["removed"].as_array().unwrap().is_empty());
    assert!(v["changed"].as_array().unwrap().is_empty());
}

/// Arguments in the wrong order still work, but say so.
#[test]
fn diff_notices_reversed_arguments() {
    let dir = tempfile::tempdir().unwrap();
    let old = prior_vintage(dir.path());
    let o = run(&[
        "mic",
        "diff",
        snapshot().to_str().unwrap(),
        old.to_str().unwrap(),
    ]);
    assert!(stderr(&o).contains("arguments may be swapped"));
}

// ------------------------------------------------------------ exit codes --

#[test]
fn success_is_zero() {
    assert_eq!(code(&run(&["mic", "get", "XNYS", "--quiet"])), 0);
    assert_eq!(code(&run(&["--help"])), 0);
    assert_eq!(code(&run(&["--version"])), 0);
    assert_eq!(code(&run(&["mic", "--help"])), 0);
}

#[test]
fn usage_errors_are_one() {
    // clap's own default here is 2, which this CLI reserves for load errors.
    assert_eq!(code(&run(&["mic", "nonesuch"])), 1);
    assert_eq!(code(&run(&["--format", "yaml", "mic", "get", "XNYS"])), 1);
    assert_eq!(code(&run(&["mic", "get", "TOOLONG"])), 1);
    assert_eq!(code(&run(&["mic", "get", "ZZZZ"])), 1, "absent MIC");
    assert_eq!(code(&run(&["mic", "load", "/no/such/file.csv"])), 1);
    assert_eq!(code(&run(&["mic", "list", "--country", "NOTACOUNTRY"])), 1);
}

/// Exit 2 means the data was bad, not the command.
#[test]
fn load_errors_are_two() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("broken.csv");
    std::fs::write(
        &path,
        "\"MIC\",\"OPERATING MIC\",\"OPRT/SGMT\",\"MARKET NAME-INSTITUTION DESCRIPTION\",\"LEGAL ENTITY NAME\",\"LEI\",\"MARKET CATEGORY CODE\",\"ACRONYM\",\"ISO COUNTRY CODE (ISO 3166)\",\"CITY\",\"WEBSITE\",\"STATUS\",\"CREATION DATE\",\"LAST UPDATE DATE\",\"LAST VALIDATION DATE\",\"EXPIRY DATE\",\"COMMENTS\"\n\
         \"XAAA\",\"XAAA\",\"OPRT\",\"FINE\",,,\"RMKT\",,\"US\",\"NY\",,\"ACTIVE\",,,,,\n\
         \"!!\",\"????\",\"OPRT\",\"UNKEYABLE\",,,\"RMKT\",,\"US\",\"NY\",,\"ACTIVE\",,,,,\n",
    )
    .unwrap();

    let o = run(&["mic", "load", path.to_str().unwrap(), "--quiet"]);
    assert_eq!(code(&o), 2, "a row that cannot be keyed is exit 2");

    // ...and the usable record still came through.
    let v: serde_json::Value = serde_json::from_str(stdout(&o).lines().next().unwrap()).unwrap();
    assert_eq!(v["records"], 1);
    assert_eq!(v["errors"], 1);
}

#[test]
fn network_failure_is_three() {
    let dir = tempfile::tempdir().unwrap();
    let o = Command::new(BIN)
        .args(["mic", "fetch", "-o", dir.path().to_str().unwrap()])
        // Port 1 refuses connections everywhere.
        .env(URL_ENV, "http://127.0.0.1:1/mic.csv")
        .env(DATA_DIR_ENV, fixture_dir())
        .output()
        .unwrap();
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("error:"));
}

// ----------------------------------------------------------------- shape --

/// Acceptance: `--help` exposes nothing internal.
#[test]
fn help_shows_no_internal_commands() {
    let help = stdout(&run(&["--help"]));
    for internal in [
        "lint", "replay", "coverage", "build", "ingest", "harvest", "ops", "admin", "internal",
    ] {
        assert!(
            !help.contains(internal),
            "`{internal}` must not appear in public help:\n{help}"
        );
    }
    assert!(help.contains("mic"));
    assert!(help.contains("cal"));
}

/// The reserved namespace advertises the paid half without pretending to work.
#[test]
fn cal_is_reserved_and_says_so() {
    let o = run(&["cal"]);
    assert_eq!(code(&o), 1, "cannot do the work, so not a success");
    assert!(stderr(&o).contains("DIURN_API_KEY"));
    assert!(stderr(&o).contains("diurn.io"));
    assert!(stdout(&o).is_empty(), "nothing usable on stdout");

    // A plausible future invocation reports the missing key, not a parse error.
    let o = run(&["cal", "status", "US-EQUITY"]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("DIURN_API_KEY"));

    // ...and --help documents it too.
    let help = stdout(&run(&["cal", "--help"]));
    assert!(help.contains("DIURN_API_KEY"));
    assert!(help.contains("diurn.io"));
}

/// Acceptance: every mic subcommand except fetch works with no network.
///
/// Pointing the download URL at a refused port proves these paths never reach
/// for it; `network_failure_is_three` proves the same address does fail fetch,
/// so the address really is unreachable and this is not a vacuous pass.
#[test]
fn everything_except_fetch_works_offline() {
    let dir = tempfile::tempdir().unwrap();
    let old = prior_vintage(dir.path());
    let snap = snapshot();
    let snap = snap.to_str().unwrap();

    let offline = |args: &[&str]| -> Output {
        Command::new(BIN)
            .args(args)
            .env(URL_ENV, "http://127.0.0.1:1/mic.csv")
            .env(DATA_DIR_ENV, fixture_dir())
            .output()
            .unwrap()
    };

    for args in [
        vec!["mic", "get", "XNYS", "--quiet"],
        vec!["mic", "get", "XNYS", "--segments", "--quiet"],
        vec!["mic", "list", "--country", "GB", "--quiet"],
        vec!["mic", "segments", "XNYS", "--quiet"],
        vec!["mic", "load", snap, "--quiet"],
        vec!["mic", "validate", snap, "--quiet"],
        vec!["mic", "diff", old.to_str().unwrap(), snap, "--quiet"],
    ] {
        let o = offline(&args);
        assert_eq!(code(&o), 0, "offline failure: diurn {}", args.join(" "));
        assert!(!o.stdout.is_empty(), "no output: diurn {}", args.join(" "));
    }
}

/// The offline promise is structural: exactly one module may reach the network.
#[test]
fn only_fetch_links_the_network() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap().to_string();
        if name == "fetch.rs" {
            assert!(text.contains("ureq::"), "fetch.rs should be the caller");
        } else {
            assert!(
                !text.contains("ureq::"),
                "{name} reaches the network; only fetch.rs may"
            );
        }
    }
}

// ------------------------------------------------------- registry discovery --

/// Nothing is bundled, so a fresh install has to say what to do rather than
/// failing obscurely. This message is the first thing a new user reads.
#[test]
fn a_fresh_install_explains_itself() {
    let dir = tempfile::tempdir().unwrap();
    let o = run_bare(dir.path(), &["mic", "get", "XNYS"]);

    assert_eq!(code(&o), 1);
    let err = stderr(&o);
    assert!(err.contains("no MIC registry found"), "{err}");
    assert!(
        err.contains("diurn mic fetch"),
        "must say what to run: {err}"
    );
    assert!(
        err.contains("--path"),
        "must mention the alternative: {err}"
    );
    // ...and it must name the directory it looked in.
    assert!(err.contains(dir.path().to_str().unwrap()), "{err}");
}

/// With several vintages present and no `--path`, the newest wins.
#[test]
fn the_newest_vintage_is_selected_automatically() {
    let dir = tempfile::tempdir().unwrap();
    let real = std::fs::read(snapshot()).unwrap();

    std::fs::write(dir.path().join("ISO10383_MIC_2026-06-08.csv"), &real).unwrap();
    std::fs::write(dir.path().join("ISO10383_MIC_2026-08-10.csv"), &real).unwrap();
    std::fs::write(dir.path().join("ISO10383_MIC_2026-07-13.csv"), &real).unwrap();

    let o = run_bare(dir.path(), &["mic", "get", "XNYS"]);
    assert_eq!(code(&o), 0);
    assert!(
        stderr(&o).contains("ISO10383_MIC_2026-08-10.csv"),
        "should have picked the newest: {}",
        stderr(&o)
    );
    assert!(stderr(&o).contains("vintage 2026-08-10"));
}

/// An explicit `--path` always beats discovery.
#[test]
fn explicit_path_overrides_discovery() {
    let dir = tempfile::tempdir().unwrap();
    let real = std::fs::read(snapshot()).unwrap();
    // Deliberately dated far in the future, so discovery would prefer it.
    std::fs::write(dir.path().join("ISO10383_MIC_2099-01-12.csv"), &real).unwrap();

    let o = Command::new(BIN)
        .args(["mic", "get", "XNYS", "--path", snapshot().to_str().unwrap()])
        .env(DATA_DIR_ENV, dir.path())
        .env_remove(URL_ENV)
        .output()
        .unwrap();
    assert_eq!(code(&o), 0);
    assert!(stderr(&o).contains("vintage 2026-08-10"), "{}", stderr(&o));
}

#[test]
fn vintages_lists_what_is_available_and_marks_the_selection() {
    let dir = tempfile::tempdir().unwrap();
    let real = std::fs::read(snapshot()).unwrap();
    std::fs::write(dir.path().join("ISO10383_MIC_2026-06-08.csv"), &real).unwrap();
    std::fs::write(dir.path().join("ISO10383_MIC_2026-08-10.csv"), &real).unwrap();

    let o = run_bare(dir.path(), &["mic", "vintages", "--format", "json"]);
    assert_eq!(code(&o), 0);

    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["published"], "2026-08-10");
    assert_eq!(rows[0]["selected"], true);
    assert_eq!(rows[1]["published"], "2026-06-08");
    assert_eq!(rows[1]["selected"], false);
}

/// An empty data directory is a normal state, not a failure — a script asking
/// what is available deserves an answer.
#[test]
fn vintages_on_an_empty_directory_is_not_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let o = run_bare(dir.path(), &["mic", "vintages"]);
    assert_eq!(code(&o), 0);
    assert!(stderr(&o).contains("diurn mic fetch"));
}

/// A failed download must leave nothing behind for discovery to trip over.
#[test]
fn a_failed_fetch_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let o = Command::new(BIN)
        .args(["mic", "fetch"])
        .env(DATA_DIR_ENV, dir.path())
        .env(URL_ENV, "http://127.0.0.1:1/mic.csv")
        .output()
        .unwrap();
    assert_eq!(code(&o), 3);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn validate_reports_issues_without_failing_on_a_clean_file() {
    let o = run(&[
        "mic",
        "validate",
        snapshot().to_str().unwrap(),
        "--format",
        "json",
        "--quiet",
    ]);
    assert_eq!(code(&o), 0);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    let issues = v.as_array().unwrap();
    // Warnings and info, but nothing fatal.
    assert!(!issues.is_empty());
    assert!(issues.iter().all(|i| i["severity"] != "error"));
    assert!(issues.iter().any(|i| i["kind"] == "SegmentPointsToSegment"));
    assert!(issues.iter().any(|i| i["kind"] == "FutureDatedRecord"));
}
