//! End-to-end `silv update` against a mock of the github releases api.
//!
//! `--repo-url` accepts a non-github origin and forwards it to self_update as
//! `api_base_url`, so a localhost TCP server can impersonate github: it answers
//! `/repos/{owner}/{repo}/releases` with a github-shaped listing (one asset whose
//! download url points back at the mock) and serves the asset bytes.
//!
//! The perform test runs a COPY of the built binary from a temp directory, so the
//! self-replace lands on the copy and never touches cargo's artifacts.

use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

const OWNER: &str = "testowner";
const REPO: &str = "testrepo";
const ASSET_PAYLOAD: &[u8] = b"MOCK-NEW-BINARY-CONTENT";

/// Answers two routes: the release listing and `/files/<asset>`; then closes each connection.
struct MockApi {
    port: u16,
}

impl MockApi {
    /// The server binds first, then `build(port)` produces the listing response, so the
    /// asset download url inside the listing can carry the real port. `/files/<asset>`
    /// answers with the fixed default payload.
    fn start(build: impl FnOnce(u16) -> (u16, Vec<u8>)) -> MockApi {
        Self::start_with(build, ASSET_PAYLOAD.to_vec())
    }

    /// Like `start`, but `/files/<asset>` answers with `asset` (every /files/ route shares
    /// it). Use a binary distinct from the one under test when asserting on the replace.
    fn start_with<F>(build: F, asset: Vec<u8>) -> MockApi
    where
        F: FnOnce(u16) -> (u16, Vec<u8>),
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("binds a free port");
        let port = listener.local_addr().expect("local addr").port();
        let (listing_status, listing_body) = build(port);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let request = read_request(&mut stream);
                let target = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .split('?')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let (status, body) = if target == format!("/repos/{OWNER}/{REPO}/releases") {
                    (listing_status, listing_body.clone())
                } else if target.starts_with("/files/") {
                    (200, asset.clone())
                } else {
                    (404, br#"{"message":"Not Found"}"#.to_vec())
                };
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream
                    .write_all(header.as_bytes())
                    .and_then(|_| stream.write_all(&body));
                let _ = stream.flush();
            }
        });
        MockApi { port }
    }

    /// Url accepted by `--repo-url`: keeps the localhost origin as the api base.
    fn repo_url(&self) -> String {
        format!("http://127.0.0.1:{}/{OWNER}/{REPO}", self.port)
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 1024];
    while let Ok(read) = stream.read(&mut chunk) {
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.windows(4).any(|window| window == b"\r\n\r\n") || buffer.len() > 8192 {
            break;
        }
    }
    String::from_utf8_lossy(&buffer).into_owned()
}

/// github-shaped release listing: one release, one raw-binary asset for this platform,
/// whose download url points back at the mock server on `port`.
fn listing(port: u16, version: &str, body: &str) -> (u16, Vec<u8>) {
    let triple = silv::update::target_triple().expect("platform has a release triple");
    let asset_name = format!("silv-v{version}-{triple}{}", std::env::consts::EXE_SUFFIX);
    let json = format!(
        r#"[{{"tag_name":"v{version}","name":"v{version}","body":"{body}","created_at":"2026-11-20T10:00:00Z","html_url":"http://127.0.0.1/releases","assets":[{{"id":1,"name":"{asset_name}","url":"http://127.0.0.1:{port}/files/{asset_name}"}}]}}]"#
    );
    (200, json.into_bytes())
}

fn silv(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_silv"))
        .args(args)
        .output()
        .expect("runs silv")
}

fn combined(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

#[test]
fn check_reports_available_release_with_changelog() {
    let api = MockApi::start(|port| listing(port, "9.9.9", "## Fixed\\n- mock changelog line"));
    let output = silv(&["update", "--check", "--repo-url", &api.repo_url()]);
    let text = combined(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains(&format!("update available: silv {} -> 9.9.9", silv::update::current_version())), "{text}");
    assert!(text.contains("mock changelog line"), "{text}");
    assert!(text.contains("run `silv update`"), "{text}");
}

#[test]
fn check_reports_up_to_date_for_older_release() {
    let api = MockApi::start(|port| listing(port, "0.0.1", "ancient"));
    let output = silv(&["update", "--check", "--repo-url", &api.repo_url()]);
    let text = combined(&output);
    assert!(output.status.success(), "{text}");
    assert!(text.contains("is up to date"), "{text}");
}

#[test]
fn unreachable_repo_suggests_repo_url() {
    let api = MockApi::start(|_port| (404, br#"{"message":"Not Found"}"#.to_vec()));
    let output = silv(&["update", "--check", "--repo-url", &api.repo_url()]);
    let text = combined(&output);
    assert!(!output.status.success(), "a 404 must fail the run: {text}");
    assert!(text.contains("cannot read releases"), "{text}");
    assert!(text.contains("silv update --repo-url https://github.com/owner/repo"), "{text}");
}

#[test]
fn connection_refused_is_reported_as_unreachable() {
    // Bind, read the port, drop: nothing listens there.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
        listener.local_addr().expect("addr").port()
    };
    let url = format!("http://127.0.0.1:{port}/{OWNER}/{REPO}");
    let output = silv(&["update", "--check", "--repo-url", &url]);
    let text = combined(&output);
    assert!(!output.status.success(), "{text}");
    assert!(text.contains("cannot read releases"), "{text}");
}

#[test]
fn update_performs_self_replace_on_a_temp_copy() {
    // Serve a binary distinct from the one under test, otherwise "copy == asset" is
    // trivially true (the copy is a copy of the same artifact). Appended trailing bytes
    // keep a PE/ELF runnable, so the replaced copy can also be executed below.
    let stock = std::fs::read(env!("CARGO_BIN_EXE_silv")).expect("read the binary under test");
    let mut asset = stock.clone();
    asset.extend_from_slice(b"-MOCK-RELEASE-ASSET-MARKER");
    let api = MockApi::start_with(|port| listing(port, "9.9.9", "fresh"), asset.clone());

    let dir: PathBuf = std::env::temp_dir().join(format!(
        "silv-update-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let exe = dir.join(format!("silv{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_silv"), &exe).expect("copy of the binary under test");

    let output = Command::new(&exe)
        .args(["update", "--repo-url", &api.repo_url()])
        .output()
        .expect("runs the copied silv");
    let text = combined(&output);
    let replaced = std::fs::read(&exe).unwrap_or_default();

    let runs = Command::new(&exe).arg("--version").output().expect("the replaced copy runs");
    let version_out = String::from_utf8_lossy(&runs.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(output.status.success(), "{text}");
    assert!(text.contains("updated to 9.9.9"), "{text}");
    assert_eq!(replaced, asset, "the copy must become exactly the served asset");
    assert_ne!(replaced, stock, "the served asset differs from the runner, so the replace changed the binary");
    assert!(runs.status.success(), "the replaced copy must be executable");
    assert!(version_out.contains("0.3.0"), "the replaced copy runs the real program: {version_out}");
    let now = std::fs::read(env!("CARGO_BIN_EXE_silv")).unwrap_or_default();
    assert_eq!(now, stock, "cargo's artifact must stay untouched");
}
