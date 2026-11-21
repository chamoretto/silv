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
use std::sync::Arc;
use std::sync::Mutex;

const OWNER: &str = "testowner";
const REPO: &str = "testrepo";
const ASSET_PAYLOAD: &[u8] = b"MOCK-NEW-BINARY-CONTENT";
const NOT_FOUND: &[u8] = br#"{"message":"Not Found"}"#;

/// One release in the mock listing.
struct ReleaseSpec {
    version: &'static str,
    body: &'static str,
    /// False: the release publishes no asset for this platform (`"assets":[]`).
    has_asset: bool,
}

/// Which tag lookups the mock answers: `v<version>` (github convention), bare
/// `<version>` (a fork's scheme), or none (a repository whose tags are unrelated).
enum Tags {
    Prefixed,
    Bare,
    None,
}

/// A github-shaped releases api on a localhost port: listing, per-tag lookups, assets.
/// Every request line (method, target, headers) is recorded for assertions.
struct MockApi {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
}

impl MockApi {
    /// Serves `releases` (in listing order, as the backend returns them) and the default payload asset.
    fn start(releases: &[(&'static str, &'static str)]) -> MockApi {
        Self::start_with(releases, ASSET_PAYLOAD)
    }

    /// Like `start`, with `/files/<asset>` answering by `asset` (every `/files/` route shares
    /// it). Use a binary distinct from the one under test when asserting on the replace.
    fn start_with(releases: &[(&'static str, &'static str)], asset: &[u8]) -> MockApi {
        Self::serve(&specs(releases), asset, Tags::Prefixed)
    }

    /// One release that publishes no asset for this platform.
    fn start_without_asset(version: &'static str) -> MockApi {
        Self::serve(
            &[ReleaseSpec {
                version,
                body: "no asset here",
                has_asset: false,
            }],
            ASSET_PAYLOAD,
            Tags::Prefixed,
        )
    }

    /// Answers the listing, every `/releases/tags/...` lookup and every unknown route
    /// with 404; assets are not reachable (the repository "publishes nothing usable").
    fn unreachable() -> MockApi {
        Self::serve(&[], ASSET_PAYLOAD, Tags::None)
    }

    fn serve(releases: &[ReleaseSpec], asset: &[u8], tags: Tags) -> MockApi {
        let listener = TcpListener::bind("127.0.0.1:0").expect("binds a free port");
        let port = listener.local_addr().expect("local addr").port();
        // The release objects are built with the port baked into the asset download urls.
        let objects: Vec<(String, String)> = releases.iter().map(|release| release_object(port, release)).collect();
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
        // A fork that tags `9.9.8` instead of `v9.9.8`: the bare keys answer, prefixed 404.
        let by_tag = match tags {
            Tags::Prefixed => by_tag,
            Tags::Bare => by_tag
                .into_iter()
                .map(|(tag, body)| (tag.trim_start_matches('v').to_owned(), body))
                .collect(),
            Tags::None => HashMap::new(),
        };
        let listing_body = listing.into_bytes();
        let asset = asset.to_vec();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        // The server thread lives for the whole test process: one mock per test, bound
        // to a free port, dropped connections per request; leaks are bounded by the test count.
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let request = read_request(&mut stream);
                recorded.lock().expect("record the request").push(request.clone());
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
                    (
                        if objects.is_empty() && matches!(tags, Tags::None) {
                            404
                        } else {
                            200
                        },
                        listing_body.clone(),
                    )
                } else if let Some(tag) = target.strip_prefix(&tag_prefix) {
                    match by_tag.get(tag) {
                        Some(body) => (200, body.clone()),
                        None => (404, NOT_FOUND.to_vec()),
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
        MockApi { port, requests }
    }

    /// Url accepted by `--repo-url`: keeps the localhost origin as the api base.
    fn repo_url(&self) -> String {
        format!("http://127.0.0.1:{}/{OWNER}/{REPO}", self.port)
    }

    /// All requests the mock received, headers included.
    fn requests(&self) -> String {
        self.requests.lock().expect("lock").join("\n")
    }
}

fn specs(releases: &[(&'static str, &'static str)]) -> Vec<ReleaseSpec> {
    releases
        .iter()
        .map(|(version, body)| ReleaseSpec {
            version,
            body,
            has_asset: true,
        })
        .collect()
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
/// platform whose download url points back at the mock server on `port` (none when the
/// spec publishes no asset).
fn release_object(port: u16, release: &ReleaseSpec) -> (String, String) {
    let version = release.version;
    let triple = silv::update::target_triple().expect("platform has a release triple");
    let asset_name = format!("silv-v{version}-{triple}{}", std::env::consts::EXE_SUFFIX);
    let tag = format!("v{version}");
    let assets = if release.has_asset {
        format!(r#"[{{"id":1,"name":"{asset_name}","url":"http://127.0.0.1:{port}/files/{asset_name}"}}]"#)
    } else {
        "[]".to_owned()
    };
    let json = format!(
        r#"{{"tag_name":"{tag}","name":"{tag}","body":"{}","created_at":"2026-11-20T10:00:00Z","html_url":"http://127.0.0.1/releases","assets":{assets}}}"#,
        release.body
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
    assert_eq!(exit_code(&output), 2, "a 404 is trouble, exit 2: {text}");
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
    assert_eq!(exit_code(&output), 2, "an unreachable repository is trouble, exit 2: {text}");
    assert!(text.contains("cannot read releases"), "{text}");
}

#[test]
fn check_orders_offers_by_semver_not_by_listing_order() {
    // self_update documents no ordering for the release list (the github backend
    // returns it as paginated). The newest version must win regardless of list order:
    // the listing here is ascending, the "newest-first" assumption must not leak in.
    let api = MockApi::start(&[("9.9.7", "older body"), ("9.9.9", "newest body")]);
    let output = silv(&["update", "--check", "--repo-url", &api.repo_url()]);
    let text = combined(&output);
    assert_eq!(exit_code(&output), 1, "an update is available: {text}");
    assert!(text.contains("-> 9.9.9"), "the newest version is the offer: {text}");
    let newest = text.find("v9.9.9").expect("9.9.9 is listed");
    let older = text.find("v9.9.7").expect("9.9.7 is listed");
    assert!(newest < older, "the changelog is newest-first, not listing order:\n{text}");
}

#[test]
fn check_ignores_releases_without_an_asset_for_this_target() {
    // The listing's newest release has no asset for this platform: it must not drive
    // the update line, the release behind it (with an asset) is the offer.
    let api = MockApi::serve(
        &[
            ReleaseSpec {
                version: "9.9.9",
                body: "no asset release, must not be offered",
                has_asset: false,
            },
            ReleaseSpec {
                version: "9.9.8",
                body: "asset release",
                has_asset: true,
            },
        ],
        ASSET_PAYLOAD,
        Tags::Prefixed,
    );
    let output = silv(&["update", "--check", "--repo-url", &api.repo_url()]);
    let text = combined(&output);
    assert_eq!(exit_code(&output), 1, "the asset release is available: {text}");
    assert!(text.contains("-> 9.9.8"), "{text}");
    assert!(!text.contains("no asset release"), "the asset-less release is filtered out: {text}");

    // A listing of asset-less releases only: nothing installable, "up to date".
    let api = MockApi::start_without_asset("9.9.9");
    let output = silv(&["update", "--check", "--repo-url", &api.repo_url()]);
    let text = combined(&output);
    assert_eq!(exit_code(&output), 0, "no installable release is not an offer: {text}");
    assert!(text.contains("is up to date"), "{text}");
}

#[test]
fn the_github_token_never_travels_to_a_custom_repo_url() {
    // A custom --repo-url host (fork server, local api, plain http) must not receive
    // GITHUB_TOKEN credentials. The default api.github.com path sends them, which no
    // offline mock can observe; the builder's api_base match is the unit-side guarantee.
    let api = MockApi::start(&[("9.9.9", "fresh")]);
    let output = Command::new(env!("CARGO_BIN_EXE_silv"))
        .args(["update", "--check", "--repo-url", &api.repo_url()])
        .env("GITHUB_TOKEN", "super-secret-token")
        .output()
        .expect("runs silv");
    let text = combined(&output);
    assert_eq!(exit_code(&output), 1, "the check works with the token set: {text}");
    let requests = api.requests();
    assert!(!requests.contains("super-secret-token"), "the token leaked to a custom api host:\n{requests}");
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
fn update_falls_back_to_the_listing_when_no_tag_routes_exist() {
    // The pinned path asks `/releases/tags/v9.9.9` and the bare tag asks
    // `/releases/tags/9.9.9`; this mock answers 404 at every tag route (a repository
    // whose tags are plain names). The newest release equals the offered stable one,
    // so the listing-based install is safe and must be taken.
    let api = MockApi::serve(&specs(&[("9.9.9", "fresh")]), ASSET_PAYLOAD, Tags::None);
    let output = silv(&["update", "--repo-url", &api.repo_url(), "--check"]);
    let text = combined(&output);
    assert_eq!(exit_code(&output), 1, "the check works: {text}");

    // Perform on a copy, so the self-replace does not touch cargo's artifact.
    let dir = temp_dir("silv-update-fallback");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let exe = dir.join(format!("silv{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_silv"), &exe).expect("copy of the binary under test");
    let output = Command::new(&exe)
        .args(["update", "--repo-url", &api.repo_url()])
        .output()
        .expect("runs the copied silv");
    let text = combined(&output);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(output.status.success(), "the listing fallback installs: {text}");
    assert!(text.contains("updated to 9.9.9"), "{text}");
}

#[test]
fn update_installs_a_fork_that_tags_releases_without_the_v_prefix() {
    // The fork tags `9.9.8`, not `v9.9.8`: the pinned tag 404s, the bare tag answers.
    // The stable release must be installed through the bare tag, and the rc that sits
    // above it in the listing must never be touched.
    let api = MockApi::serve(
        &specs(&[("9.9.9-rc.1", "rc, must never be installed"), ("9.9.8", "stable")]),
        ASSET_PAYLOAD,
        Tags::Bare,
    );
    let dir = temp_dir("silv-update-bare-tags");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let exe = dir.join(format!("silv{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_silv"), &exe).expect("copy of the binary under test");
    let output = Command::new(&exe)
        .args(["update", "--repo-url", &api.repo_url()])
        .output()
        .expect("runs the copied silv");
    let text = combined(&output);
    let replaced = std::fs::read(&exe).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(output.status.success(), "the bare tag installs the stable release: {text}");
    assert!(text.contains("updated to 9.9.8"), "{text}");
    assert_eq!(replaced, ASSET_PAYLOAD, "the served asset landed in place");
}

#[test]
fn update_refuses_the_listing_fallback_when_a_prerelease_is_newer() {
    // No tag routes answers at all, and the listing's newest release is a prerelease:
    // the listing-based install would pick the rc, so it must be refused and the
    // binary left alone (exit 2, trouble).
    let api = MockApi::serve(
        &specs(&[("9.9.9-rc.1", "rc would win the listing"), ("9.9.8", "stable")]),
        ASSET_PAYLOAD,
        Tags::None,
    );
    let dir = temp_dir("silv-update-refuse-rc");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let exe = dir.join(format!("silv{}", std::env::consts::EXE_SUFFIX));
    std::fs::copy(env!("CARGO_BIN_EXE_silv"), &exe).expect("copy of the binary under test");
    let stock = std::fs::read(env!("CARGO_BIN_EXE_silv")).expect("read the binary under test");
    let output = Command::new(&exe)
        .args(["update", "--repo-url", &api.repo_url()])
        .output()
        .expect("runs the copied silv");
    let text = combined(&output);
    let replaced = std::fs::read(&exe).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(exit_code(&output), 2, "a prerelease-only listing is trouble, exit 2: {text}");
    assert!(text.contains("update failed"), "{text}");
    assert!(!text.contains("updated to"), "nothing was installed: {text}");
    assert_eq!(replaced, stock, "the refused install leaves the binary alone");
}

/// A unique temp directory name for one test (the caller creates it).
fn temp_dir(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ))
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
    assert_eq!(exit_code(&refused), 2, "a refusal is trouble, exit 2: {text}");
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
