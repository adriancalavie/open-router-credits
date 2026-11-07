//! End-to-end checks of `--range` against a local mock of both endpoints.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_open-router-credits");

/// Accepts exactly as many requests as it has responses, in order, and returns
/// everything the binary sent so the test can assert on the request bodies.
struct Mock {
    base: String,
    requests: thread::JoinHandle<String>,
}

impl Mock {
    fn new(responses: Vec<(&'static str, &'static str)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();

        let requests = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut seen = String::new();
            for (status, body) in responses {
                let stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(_) if Instant::now() < deadline => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => return seen,
                    }
                };
                let mut stream = stream;
                // A nonblocking listener hands out nonblocking streams on macOS;
                // reads would then return empty before the request arrives.
                stream.set_nonblocking(false).ok();
                stream.set_read_timeout(Some(Duration::from_secs(2))).ok();

                let mut raw = Vec::new();
                let mut chunk = [0u8; 1024];
                while !raw.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => raw.extend_from_slice(&chunk[..n]),
                    }
                }
                // Drain the request body: the header read above can return
                // before the JSON payload has arrived.
                let header_end = raw
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .map(|at| at + 4)
                    .unwrap_or(raw.len());
                let head = String::from_utf8_lossy(&raw[..header_end]).to_ascii_lowercase();
                let length: usize = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse().ok())
                    .unwrap_or(0);
                while raw.len() < header_end + length {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => raw.extend_from_slice(&chunk[..n]),
                    }
                }

                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
                seen.push_str(&String::from_utf8_lossy(&raw));
                seen.push_str("\n---\n");
            }
            seen
        });

        Self {
            base: format!("http://{addr}"),
            requests,
        }
    }
}

fn config_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "open-router-credits-range-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(&path, "api_key = \"sk-or-v1-test\"\nrefresh_secs = 60\n").unwrap();
    path
}

#[test]
fn a_window_queries_analytics_and_prints_the_spend() {
    let mock = Mock::new(vec![
        (
            "200 OK",
            r#"{"data":{"total_credits":40.0,"total_usage":36.5}}"#,
        ),
        (
            "200 OK",
            r#"{"data":{"data":[{"credits_usage":5.684795}],"metadata":{"row_count":1,"truncated":false}}}"#,
        ),
    ]);

    let output = Command::new(BIN)
        .args(["--once", "--range", "week", "--config"])
        .arg(config_file("week"))
        .env("OPENROUTER_API_BASE", &mock.base)
        .output()
        .expect("binary should run");
    let requests = mock.requests.join().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stderr: {stderr}");
    assert!(stdout.contains("Used · this week"), "{stdout}");
    assert!(stdout.contains("$5.68"), "{stdout}");
    assert!(stdout.contains("$3.50"), "{stdout}");
    // The lifetime "Purchased" row belongs to the default view only.
    assert!(!stdout.contains("Purchased"), "{stdout}");

    assert!(requests.contains("GET /credits"), "{requests}");
    assert!(requests.contains("POST /analytics/query"), "{requests}");
    assert!(requests.contains("credits_usage"), "{requests}");
    assert!(requests.contains(r#""time_range""#), "{requests}");
    // Local midnight is sent as an instant, never as a bare date.
    assert!(
        requests.contains(r#""start":"20"#) && requests.contains('Z'),
        "{requests}"
    );
}

#[test]
fn an_unknown_range_explains_the_choices() {
    let output = Command::new(BIN)
        .args(["--range", "fortnight"])
        .output()
        .expect("binary should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown range"), "{stderr}");
    assert!(stderr.contains("quarter"), "{stderr}");
}

#[test]
fn the_default_view_still_prints_lifetime_totals() {
    let mock = Mock::new(vec![(
        "200 OK",
        r#"{"data":{"total_credits":100.5,"total_usage":25.75}}"#,
    )]);

    let output = Command::new(BIN)
        .args(["--once", "--config"])
        .arg(config_file("default"))
        .env("OPENROUTER_API_BASE", &mock.base)
        .output()
        .expect("binary should run");
    let requests = mock.requests.join().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("Purchased"), "{stdout}");
    assert!(stdout.contains("$74.75"), "{stdout}");
    // No window means no analytics traffic at all.
    assert!(!requests.contains("analytics"), "{requests}");
}
