//! End-to-end tests of `diurn cal`: the real binary, against a fake API.
//!
//! The fake is a few lines of `std::net`: it answers each path with a
//! response recorded from a local `diurn-api` (tests/fixtures/api/), or with
//! an error body a test names, so these run with no network and no key. What
//! is checked is what only the outside can see: exit codes, stdout against
//! stderr, and that `--format json` is the client crate's model, unchanged.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::thread;

const BIN: &str = env!("CARGO_BIN_EXE_diurn");

fn fixture(name: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/api")
        .join(format!("{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("no fixture {}", path.display()))
}

/// A fake API: path (without the query) to (status, body). Records the
/// requests it saw, query included.
struct Api {
    url: String,
    seen: Arc<Mutex<Vec<String>>>,
}

fn api(routes: &[(&str, u16, String)]) -> Api {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let routes: HashMap<String, (u16, String)> = routes
        .iter()
        .map(|(p, s, b)| (p.to_string(), (*s, b.clone())))
        .collect();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let target = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            // Read past the headers, and any body a POST sends.
            let mut length = 0usize;
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; length];
            let _ = reader.read_exact(&mut body);
            log.lock().unwrap().push(target.clone());
            let path = target.split('?').next().unwrap_or("/");
            let (status, payload) = routes.get(path).cloned().unwrap_or((
                404,
                format!(r#"{{"error":"not_found","message":"{path}"}}"#),
            ));
            let reply = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nDiurn-MICs-Day: 1/5\r\nX-Diurn-Dataset: 6962d9af709f\r\n\
                 Connection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    Api { url, seen }
}

fn cal(api: &Api, args: &[&str]) -> Output {
    Command::new(BIN)
        .arg("cal")
        .args(args)
        .env("DIURN_API_URL", &api.url)
        .env("DIURN_API_KEY", "diurn_key_test")
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

fn json(s: &str) -> serde_json::Value {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("not JSON ({e}):\n{s}"))
}

// --- answers ---------------------------------------------------------------------

#[test]
fn status_reads_like_a_departure_board() {
    let api = api(&[("/v1/venues/XNYS/status", 200, fixture("status-xnys"))]);
    let o = cal(
        &api,
        &[
            "status",
            "XNYS",
            "--at",
            "2026-11-27T17:00:00Z",
            "-f",
            "table",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(
        out.contains("running") && out.contains("early_close"),
        "{out}"
    );
    assert!(
        out.contains("2026-11-27 13:00 EST"),
        "the close, in the venue's clock:\n{out}"
    );
    assert!(out.contains("Day after Thanksgiving"));
    // The instant asked about reached the API as given.
    let seen = api.seen.lock().unwrap();
    assert!(seen[0].contains("at=2026-11-27T17%3A00%3A00Z"), "{seen:?}");
}

#[test]
fn json_is_the_client_crates_model_unchanged() {
    // SPEC Phase 2: the record shape is the library's, not the CLI's.
    let api = api(&[("/v1/venues/XNYS/days/2026-11-26", 200, fixture("day-xnys"))]);
    let o = cal(&api, &["day", "XNYS", "2026-11-26", "-f", "json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(json(&stdout(&o)), json(&fixture("day-xnys")));
}

#[test]
fn jsonl_is_one_line_per_day() {
    let api = api(&[("/v1/venues/XTKS/sessions", 200, fixture("sessions-xtks"))]);
    let o = cal(
        &api,
        &[
            "sessions",
            "XTKS",
            "--from",
            "2026-11-01",
            "--to",
            "2026-11-05",
            "-f",
            "jsonl",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let lines: Vec<_> = stdout(&o).lines().map(json).collect();
    assert_eq!(lines.len(), 5);
    assert_eq!(lines[2]["kind"], "holiday", "Culture Day");
}

#[test]
fn many_venues_give_one_json_line_each_with_its_mic() {
    let api = api(&[("/v1/status", 200, fixture("statuses"))]);
    let o = cal(
        &api,
        &[
            "status",
            "XNYS",
            "XLON",
            "--at",
            "2026-11-27T17:00:00Z",
            "-f",
            "jsonl",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let mics: Vec<String> = stdout(&o)
        .lines()
        .map(|l| json(l)["mic"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(mics, ["XLON", "XNYS"]);
}

#[test]
fn sessions_show_the_lunch_break() {
    let api = api(&[("/v1/venues/XTKS/sessions", 200, fixture("sessions-xtks"))]);
    let o = cal(
        &api,
        &[
            "sessions",
            "XTKS",
            "--from",
            "2026-11-01",
            "--to",
            "2026-11-05",
            "-f",
            "table",
        ],
    );
    let out = stdout(&o);
    assert!(out.contains("09:00-11:30; 12:30-15:30"), "{out}");
    assert!(out.contains("Culture Day"));
}

#[test]
fn next_names_what_it_skipped() {
    let api = api(&[(
        "/v1/venues/XNYS/trading-days/next",
        200,
        fixture("next-xnys"),
    )]);
    let o = cal(
        &api,
        &["next", "XNYS", "--from", "2026-11-25", "-f", "table"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("2026-11-27"), "{out}");
    assert!(out.contains("Thanksgiving Day"));
}

#[test]
fn several_mics_step_together() {
    let api = api(&[("/v1/trading-days/next", 200, fixture("joint-next"))]);
    let o = cal(
        &api,
        &["next", "XNYS", "XLON", "--from", "2026-12-24", "-f", "csv"],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.starts_with("mics,mode,from,n,date\n"), "{out}");
    assert!(
        out.contains(",2026-12-29"),
        "after Christmas and Boxing Day:\n{out}"
    );
    assert!(
        api.seen.lock().unwrap()[0].contains("mics=XNYS%2CXLON")
            || api.seen.lock().unwrap()[0].contains("mics=XNYS,XLON")
    );
}

#[test]
fn exceptions_as_csv() {
    let api = api(&[("/v1/exceptions", 200, fixture("exceptions"))]);
    let o = cal(
        &api,
        &[
            "exceptions",
            "XNYS",
            "XLON",
            "--from",
            "2026-12-20",
            "--to",
            "2027-01-05",
            "-f",
            "csv",
        ],
    );
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.starts_with("date,mic,kind,hours,reason,verification\n"));
    assert!(
        out.contains("2026-12-28,XLON,holiday,,Boxing Day,verified"),
        "{out}"
    );
}

#[test]
fn the_meters_go_to_stderr_and_quiet_silences_them() {
    let api = api(&[(
        "/v1/venues/XNYS/next-close",
        200,
        fixture("next-close-xnys"),
    )]);
    let o = cal(
        &api,
        &[
            "next-close",
            "XNYS",
            "--after",
            "2026-11-27T15:00:00Z",
            "-f",
            "table",
        ],
    );
    assert!(stdout(&o).contains("XNYS closes 2026-11-27 13:00 EST"));
    assert!(stderr(&o).contains("MICs today 1/5") && stderr(&o).contains("6962d9af709f"));
    let q = cal(
        &api,
        &[
            "next-close",
            "XNYS",
            "--after",
            "2026-11-27T15:00:00Z",
            "-q",
        ],
    );
    assert!(stderr(&q).is_empty(), "{}", stderr(&q));
}

// --- exit codes ------------------------------------------------------------------

#[test]
fn no_key_is_refused_before_any_request() {
    let o = Command::new(BIN)
        .args(["cal", "status", "XNYS"])
        .env_remove("DIURN_API_KEY")
        .env_remove("DIURN_API_URL")
        .output()
        .unwrap();
    assert_eq!(code(&o), 4);
    assert!(stderr(&o).contains("DIURN_API_KEY") && stderr(&o).contains("diurn.io/signin"));
    assert!(stdout(&o).is_empty());
}

#[test]
fn an_inactive_key_is_four() {
    let body = r#"{"error":"unauthorized","message":"that key is not active"}"#.to_string();
    let api = api(&[("/v1/venues/XNYS/status", 401, body)]);
    let o = cal(&api, &["status", "XNYS"]);
    assert_eq!(code(&o), 4);
    assert!(stderr(&o).contains("DIURN_API_KEY"), "says how to fix it");
}

#[test]
fn a_mic_cap_is_four_and_names_what_holds_it() {
    let body = r#"{"error":"mic_cap_exceeded","message":"XHKG would be unique MIC number 6",
        "limit":{"kind":"mics_day","used":5,"max":5},"mics_in_use":["XLON","XNYS"],
        "resets_at":"2026-11-28T09:00:00Z","upgrade":"https://diurn.io/pricing"}"#
        .to_string();
    let api = api(&[("/v1/venues/XHKG/status", 429, body)]);
    let o = cal(&api, &["status", "XHKG"]);
    assert_eq!(code(&o), 4);
    assert!(stderr(&o).contains("XLON XNYS"), "{}", stderr(&o));
}

#[test]
fn an_unknown_mic_is_four() {
    let body = r#"{"error":"unknown_mic","message":"ZZZZ is not in the registry"}"#.to_string();
    let api = api(&[("/v1/venues/ZZZZ/status", 404, body)]);
    assert_eq!(code(&cal(&api, &["status", "ZZZZ"])), 4);
}

#[test]
fn indeterminate_is_five() {
    let body = r#"{"error":"indeterminate","message":"2119-12-30 cannot be answered","date":"2119-12-30"}"#
        .to_string();
    let api = api(&[
        ("/v1/venues/XBKK/trading-days/next", 422, body),
        (
            "/v1/venues/XBKK/trading-days/count",
            200,
            fixture("count-xbkk"),
        ),
    ]);
    assert_eq!(
        code(&cal(&api, &["next", "XBKK", "--from", "2119-12-29"])),
        5
    );
    // A count with unknown days in it is a lower bound: printed, and five.
    let o = cal(
        &api,
        &[
            "count",
            "XBKK",
            "--from",
            "2119-12-01",
            "--to",
            "2120-01-31",
            "-f",
            "table",
        ],
    );
    assert_eq!(code(&o), 5);
    assert!(stdout(&o).contains("at least"), "{}", stdout(&o));
}

#[test]
fn an_unreachable_api_is_three() {
    // Bound and closed again, so nothing is listening there.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let o = Command::new(BIN)
        .args(["cal", "status", "XNYS"])
        .env("DIURN_API_URL", format!("http://127.0.0.1:{port}"))
        .env("DIURN_API_KEY", "k")
        .output()
        .unwrap();
    assert_eq!(code(&o), 3, "{}", stderr(&o));
}

#[test]
fn a_malformed_date_is_one_and_asks_nothing() {
    let api = api(&[]);
    let o = cal(&api, &["day", "XNYS", "2026-13-01"]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("YYYY-MM-DD"));
    assert!(api.seen.lock().unwrap().is_empty());
}

#[test]
fn help_names_the_key_and_the_exit_codes() {
    let o = Command::new(BIN).args(["cal", "--help"]).output().unwrap();
    let help = stdout(&o);
    assert!(help.contains("DIURN_API_KEY") && help.contains("diurn.io/signin"));
    for line in [
        "3  the API could not be reached",
        "4  the API refused",
        "5  indeterminate",
    ] {
        assert!(help.contains(line), "missing {line:?}:\n{help}");
    }
}
