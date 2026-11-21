//! End-to-end `silv update` against a mock of the github releases api.
//!
//! `--repo-url` accepts a non-github origin and forwards it to self_update as
//! `api_base_url`, so a localhost TCP server can impersonate github: it answers
//! `/repos/{owner}/{repo}/releases` (the listing) and
//! `/repos/{owner}/{repo}/releases/tags/{tag}` (a pinned release, the path `perform`
//! takes) with github-shaped json, and serves the asset bytes under `/files/<asset>`.
//!
//! The perform test runs a COPY of the built binary from a temp directory, so the
//! self-replace lands on the copy and never touches cargo's artifacts.

use std::collections::HashMap;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

const OWNER: &str = "testowner";
const REPO: &str = "testrepo";
const ASSET_PAYLOAD: &[u8] = b"MOCK-NEW-BINARY-CONTENT";
const NOT_FOUND: &[u8] = br#"{"message":"Not Found"}"#;

/// A github-shaped releases api on a localhost port: listing, per-tag lookups, assets.
struct MockApi {
    port: u16,
}

impl MockApi {
    /// Serves `releases` (newest first, as github lists them) and the default payload asset.
    fn start(releases: &[(&str, &str)]) -> MockApi {
        Self::start_with(releases, ASSET_PAYLOAD)
    }

    /// Like `start`, with `/files/<asset>` answering by `asset` (every `/files/` route shares
    /// it). Use a binary distinct from the one under test when asserting on the replace.
    fn start_with(releases: &[(&str, &str)], asset: &[u8]) -> MockApi {
        Self::serve(releases, asset, false)
    }

    /// Answers the listing, every `/releases/tags/...` lookup and every unknown route
    /// with 404; assets are not reachable (the repository "publishes nothing usable").
    fn unreachable() -> MockApi {
        Self::serve(&[], ASSET_PAYLOAD, true)
    }

    fn serve(releases: &[(&str, &str)], asset: &[u8], no_tag_routes: bool) -> MockApi {
        let listener = TcpListener::bind("127.0.0.1:0").expect("binds a free port");
        let port = listener.local_addr().expect("local addr").port();
        // The release objects are built with the port baked into the asset download urls.
        let objects: Vec<(String, String)> = releases
            .iter()
            .map(|(version, body)| release_object(port, version, body))
            .collect();
        let listing = format!(
            "[{}]",
            objects
                .iter()
                .map(|(_, json)| json.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );
        let by_tag: HashMap<String, Vec<u8>> = objects
            .iter()
            // The tag route answers with a single release object, not a list.
            .map(|(tag, json)| (tag.clone(), json.clone().into_bytes()))
            .collect();
        let listing_body = listing.into_bytes();
        let asset = asset.to_vec();
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
                let tag_prefix = format!("/repos/{OWNER}/{REPO}/releases/tags/");
                let (status, body) = if target == format!("/repos/{OWNER}/{REPO}/releases") {
                    (if objects.is_empty() && no_tag_routes { 404 } else { 200 }, listing_body.clone())
                } else if let Some(tag) = target.strip_prefix(&tag_prefix) {
                    match by_tag.get(tag) {
                        Some(body) if !no_tag_routes => (200, body.clone()),
                        _ => (404, NOT_FOUND.to_vec()),
                    }
                } else if target.starts_with("/files/") {
                    (200, asset.clone())
                } else {
                    (404, NOT_FOUND.to_vec())
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

/// One github-shaped release json object: tag `v<version>`, one raw-binary asset for this
/// platform whose download url points back at the mock server on `port`.
fn release_object(port: u16, version: &str, body: &str) -> (String, String) {
    let triple = silv::update::target_triple().expect("platform has a release triple");
    let asset_name = format!("silv-v{version}-{triple}{}", std::env::consts::EXE_SUFFIX);
    let tag = format!("v{version}");
    let json = format!(
        r#"{{"tag_name":"{tag}","name":"{tag}","body":"{body}","created_at":"2026-11-20T10:00:00Z","html_url":"http://127.0.0.1/releases","assets":[{{"id":1,"name":"{asset_name}","url":"http://127.0.0.1:{port}/files/{asset_name}"}}]}}"#
    );
    (tag, json)
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

fn exit_code(output: &Output) -> i32 {
    output.status.code().expect("the process exited normally (no signal)")
}

#[test]
fn check_reports_available_release_with_changelog_and_exits_one() {
    let api = MockApi::start(&[("9.9.9", "## Fixed\\n- mock changelog line")]);
    let output = silv(&["update", "--check", "--repo-url", &api.repo_url()]);
    let text = combined(&output);
    assert_eq!(exit_code(&output), 1, "an available update is signalled by exit 1: {text}");
    assert!(text.contains(&format!("update available: silv {} -> 9.9.9", silv::update::current_version())), "{text}");
    assert!(text.contains("changelog, the releases between silv"), "{text}");
    assert!(text.contains("mock changelog line"), "{text}");
    assert!(text.contains("run `silv update`"), "{text}");
}

#[test]
fn check_reports_up_to_date_for_older_release_and_exits_zero() {
    let api = MockApi::start(&[("0.0.1", "ancient")]);
    let output = silv(&["update", "--check", "--repo-url", &api.repo_url()]);
    let text = combined(&output);
    assert_eq!(exit_code(&output), 0, "up to date is a plain success: {text}");
    assert!(text.contains("is up to date"), "{text}");
}

#[test]
fn check_skips_prerelease_releases() {
    // The newest release is a release candidate; the stable one behind it must be offered.
    let api = MockApi::start(&[
        ("9.9.9-rc.1", "rc body, must never be offered"),
        ("9.9.8", "stable body"),
        ("9.9.7-rc.2", "older rc"),
    ]);
    let output = silv(&["update", "--check", "--repo-url", &api.repo_url()]);
    let text = combined(&output);
    assert_eq!(exit_code(&output), 1, "a stable release is available: {text}");
    assert!(text.contains("-> 9.9.8"), "the newest stable release is the offer: {text}");
    assert!(text.contains("stable body"), "{text}");
    assert!(!text.contains("9.9.9-rc.1"), "the rc is filtered out: {text}");
    assert!(!text.contains("rc body"), "the rc notes are filtered out: {text}");
}

#[test]
fn check_is_up_to_date_when_only_prereleases_are_newer() {
    let api = MockApi::start(&[("99.0.0-alpha.1", "future rc")]);
    let output = silv(&["update", "--check", "--repo-url", &api.repo_url()]);
    let text = combined(&output);
    assert_eq!(exit_code(&output), 0, "a prerelease is not an offer: {text}");
    assert!(text.contains("is up to date"), "{text}");
}

#[test]
fn unreachable_repo_suggests_repo_url() {
    let api = MockApi::unreachable();
    let output = silv(&["update", "--check", "--repo-url", &api.repo_url()]);
    let text = combined(&output);
    assert_eq!(exit_code(&output), 1, "a 404 must fail the run: {text}");
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
    assert_eq!(exit_code(&output), 1, "{text}");
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
    // An rc sits above the stable release: the update must install the stable 9.9.8
    // the check offered, not the rc the backend would call "latest".
    let api = MockApi::start_with(&[("9.9.9-rc.1", "rc"), ("9.9.8", "stable")], &asset);

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

    let runs = Command::new(&exe)
        .arg("--version")
        .output()
        .expect("the replaced copy runs");
    let version_out = String::from_utf8_lossy(&runs.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(output.status.success(), "{text}");
    assert!(text.contains("updating silv") && text.contains("-> 9.9.8"), "the stable version is the offer: {text}");
    assert!(text.contains("updated to 9.9.8"), "the pinned release was installed: {text}");
    assert_eq!(replaced, asset, "the copy must become exactly the served asset");
    assert_ne!(replaced, stock, "the served asset differs from the runner, so the replace changed the binary");
    assert!(runs.status.success(), "the replaced copy must be executable");
    assert!(version_out.contains("0.3.0"), "the replaced copy runs the real program: {version_out}");
    let now = std::fs::read(env!("CARGO_BIN_EXE_silv")).unwrap_or_default();
    assert_eq!(now, stock, "cargo's artifact must stay untouched");
}

#[test]
fn update_falls_back_to_the_latest_release_when_the_pinned_tag_is_missing() {
    // The pinned path asks `/releases/tags/v9.9.9`; this mock answers 404 there,
    // a repository that tags its releases differently. The update must still install
    // the newest release through the listing.
    let api = MockApi::serve(&[("9.9.9", "fresh")], ASSET_PAYLOAD, true);
    let output = silv(&["update", "--repo-url", &api.repo_url(), "--check"]);
    let text = combined(&output);
    assert_eq!(exit_code(&output), 1, "the check works: {text}");

    // Perform on a copy, so the self-replace does not touch cargo's artifact.
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "silv-update-fallback-{}-{}",
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
    let _ = std::fs::remove_dir_all(&dir);
    assert!(output.status.success(), "the fallback path installs: {text}");
    assert!(text.contains("updated to 9.9.9"), "{text}");
}

#[test]
fn a_cargo_managed_binary_refuses_the_self_update_without_force() {
    // Fake a cargo install: CARGO_HOME points at a temp dir and the copy runs from its bin/.
    let root: PathBuf = std::env::temp_dir().join(format!(
        "silv-cargo-home-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("temp cargo home/bin");
    let exe = bin.join(format!("silv{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_silv"), &exe).expect("copy of the binary under test");
    let stock = std::fs::read(env!("CARGO_BIN_EXE_silv")).expect("read the binary under test");
    let api = MockApi::start_with(&[("9.9.9", "fresh")], ASSET_PAYLOAD);

    let on_cargo_home = |args: &[&str]| {
        Command::new(&exe)
            .args(args)
            .env("CARGO_HOME", &root)
            .env("HOME", &root)
            .env("USERPROFILE", &root)
            .output()
            .expect("runs the copied silv")
    };

    let refused = on_cargo_home(&["update", "--repo-url", &api.repo_url()]);
    let text = combined(&refused);
    assert_eq!(exit_code(&refused), 1, "a cargo install refuses the self-update: {text}");
    assert!(text.contains("refusing to replace"), "{text}");
    assert!(text.contains("cargo install --git"), "the cargo command is the remedy: {text}");
    assert_eq!(std::fs::read(&exe).unwrap_or_default(), stock, "the refusal leaves the installed binary alone");

    let checked = on_cargo_home(&["update", "--check", "--repo-url", &api.repo_url()]);
    let text = combined(&checked);
    assert_eq!(exit_code(&checked), 1, "the check reports the update (grep-like): {text}");
    assert!(text.contains("cargo install --git"), "the check names the cargo way: {text}");
    assert_eq!(std::fs::read(&exe).unwrap_or_default(), stock, "--check changes nothing");

    let forced = on_cargo_home(&["update", "--force", "--repo-url", &api.repo_url()]);
    let text = combined(&forced);
    assert!(forced.status.success(), "--force performs the update: {text}");
    assert!(text.contains("updated to 9.9.9"), "{text}");
    assert_eq!(
        std::fs::read(&exe).unwrap_or_default(),
        ASSET_PAYLOAD,
        "--force replaced the binary, the cargo bookkeeping is left behind"
    );
    let _ = std::fs::remove_dir_all(&root);
}
