//! End-to-end checks of the binary against a local mock of GET /api/v1/credits.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::thread;

const BIN: &str = env!("CARGO_BIN_EXE_open-router-credits");

struct Mock {
    base: String,
    request: thread::JoinHandle<String>,
}

fn mock(status_line: &'static str, body: &'static str) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let request = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut raw = Vec::new();
        let mut chunk = [0u8; 1024];
        while !raw.windows(4).any(|window| window == b"\r\n\r\n") {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => raw.extend_from_slice(&chunk[..n]),
            }
        }
        let response = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
        String::from_utf8_lossy(&raw).to_string()
    });
    Mock {
        base: format!("http://{addr}"),
        request,
    }
}

fn config_file(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "open-router-credits-cli-{}-{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    std::fs::write(
        &path,
        "api_key = \"sk-or-v1-test-key-1234\"\nrefresh_secs = 60\n",
    )
    .unwrap();
    path
}

fn run_once(config: &PathBuf, base: &str) -> Output {
    Command::new(BIN)
        .args(["--once", "--config"])
        .arg(config)
        .env("OPENROUTER_API_BASE", base)
        .output()
        .expect("binary should run")
}

#[test]
fn once_prints_the_balance() {
    let mock = mock(
        "200 OK",
        r#"{"data":{"total_credits":100.5,"total_usage":25.75}}"#,
    );
    let output = run_once(&config_file("balance"), &mock.base);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "stderr: {stderr}");
    assert!(stdout.contains("Purchased"), "{stdout}");
    assert!(stdout.contains("$100.50"), "{stdout}");
    assert!(stdout.contains("$25.75"), "{stdout}");
    assert!(stdout.contains("$74.75"), "{stdout}");
    assert!(stdout.contains("25.6%"), "{stdout}");

    let request = mock.request.join().unwrap();
    let lower = request.to_ascii_lowercase();
    assert!(lower.starts_with("get /credits"), "{request}");
    assert!(
        lower.contains("authorization: bearer sk-or-v1-test-key-1234"),
        "{request}"
    );
}

#[test]
fn forbidden_key_explains_the_management_key_requirement() {
    let mock = mock("403 Forbidden", r#"{"error":{"message":"forbidden"}}"#);
    let output = run_once(&config_file("forbidden"), &mock.base);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("management key"), "{stderr}");
    assert!(
        stderr.contains("openrouter.ai/settings/management-keys"),
        "{stderr}"
    );
}

#[test]
fn unauthorized_key_points_at_the_settings_page() {
    let mock = mock("401 Unauthorized", r#"{"error":{"message":"no auth"}}"#);
    let output = run_once(&config_file("unauthorized"), &mock.base);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("401"), "{stderr}");
}

#[test]
fn refuses_to_send_the_key_over_plain_http() {
    let path = config_file("plaintext");
    let output = Command::new(BIN)
        .args(["--once", "--config"])
        .arg(&path)
        .env("OPENROUTER_API_BASE", "http://openrouter.ai/api/v1")
        .output()
        .expect("binary should run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cleartext"), "{stderr}");
}

#[test]
fn stops_reading_an_oversized_response() {
    // A hostile or broken server must not be able to stream until memory runs out.
    // Valid JSON, so only the size limit can make this fail.
    let mut big = String::from(r#"{"data":{"total_credits":1.0,"total_usage":0.5}}"#);
    big.push_str(&" ".repeat(200 * 1024));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let request = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut raw = Vec::new();
        let mut chunk = [0u8; 1024];
        while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
            match stream.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => raw.extend_from_slice(&chunk[..n]),
            }
        }
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            big.len(),
            big
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
        String::new()
    });

    let output = run_once(&config_file("oversized"), &format!("http://{addr}"));
    let _ = request.join();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("larger than request limit"),
        "expected the size cap to fire, got: {stderr}"
    );
}

#[test]
fn unknown_argument_fails_with_a_hint() {
    let output = Command::new(BIN)
        .arg("--nope")
        .output()
        .expect("binary should run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unknown argument"), "{stderr}");
    assert!(stderr.contains("--help"), "{stderr}");
}

#[test]
fn missing_config_without_a_tty_says_to_run_setup() {
    let dir = std::env::temp_dir().join(format!(
        "open-router-credits-cli-{}-missing",
        std::process::id()
    ));
    let path = dir.join("config.toml");
    let _ = std::fs::remove_file(&path);

    let output = Command::new(BIN)
        .args(["--config"])
        .arg(&path)
        .output()
        .expect("binary should run");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--setup"), "{stderr}");
}
