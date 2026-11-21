//! Self-update: release checks, the update itself, and the `silv update` command.
//!
//! Releases are published by `.github/workflows/release.yml` as raw binaries named
//! `silv-<tag>-<target>[.exe]`; `self_update`'s github backend matches them by target.
//! `update()` performs download, extraction and in-place self-replace (via the
//! self-replace and tempfile crates, internal dependencies of self_update).

use std::fmt::Display;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

use miette::Result;

use crate::error::SilvError;

/// A github repository addressed by owner and name.
///
/// The `api_base` carries a non-github.com origin (a test server or a GitHub
/// Enterprise API), passed to `self_update` as `api_base_url`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoRef {
    pub author: String,
    pub repo: String,
    pub api_base: Option<String>,
}

impl RepoRef {
    /// The repository this binary was built from (`package.repository` in Cargo.toml).
    pub fn default_repo() -> Self {
        let url = env!("CARGO_PKG_REPOSITORY");
        Self::from_str(url).unwrap_or_else(|_| Self {
            author: "chamoretto".to_owned(),
            repo: "silv".to_owned(),
            api_base: None,
        })
    }

    /// The api origin the release listing is read from.
    pub fn api_base(&self) -> String {
        self.api_base
            .clone()
            .unwrap_or_else(|| "https://api.github.com".to_owned())
    }

    pub fn url(&self) -> String {
        match &self.api_base {
            None => format!("https://github.com/{}/{}", self.author, self.repo),
            Some(base) => format!("{base}/{}/{}", self.author, self.repo),
        }
    }
}

impl Display for RepoRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.url())
    }
}

/// Parses `https://github.com/owner/repo`, `git@github.com:owner/repo.git`
/// or the bare `owner/repo` form. A url on any other host (a local test server,
/// a GitHub Enterprise api) keeps its origin, with the path prefix, as the api base.
/// The ssh forms carry no api origin: `git@host:owner/repo` on a host that is not
/// github.com is read as `owner/repo` on api.github.com (the release api is https).
impl FromStr for RepoRef {
    type Err = String;

    fn from_str(input: &str) -> std::result::Result<Self, Self::Err> {
        let token = input.trim().trim_end_matches('/');
        let invalid = || Err(format!("`{input}` is not a repository url: expected https://github.com/owner/repo"));
        let (origin, path) = if let Some(rest) = token.strip_prefix("ssh://") {
            (None, rest.split_once('/').map(|x| x.1).unwrap_or_default().to_owned())
        } else if let Some(rest) = token.strip_prefix("git@") {
            (None, rest.split_once(':').map(|x| x.1).unwrap_or_default().to_owned())
        } else if let Some(rest) = token.strip_prefix("https://").or_else(|| token.strip_prefix("http://")) {
            let scheme = if token.starts_with("http://") { "http" } else { "https" };
            let mut segments = rest.splitn(3, '/');
            let host = segments.next().unwrap_or_default();
            if host.is_empty() {
                return invalid();
            }
            let path = segments.collect::<Vec<_>>().join("/");
            if host.eq_ignore_ascii_case("github.com") {
                (None, path)
            } else {
                (Some(format!("{scheme}://{host}")), path)
            }
        } else {
            (None, token.to_owned())
        };
        let path = path.strip_suffix(".git").map(str::to_owned).unwrap_or(path);
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        // A github.com url addresses exactly `owner/repo`; a custom base may carry
        // an api prefix before it (GitHub Enterprise style `.../api/v3`).
        if parts.len() < 2 || (origin.is_none() && parts.len() != 2) {
            return invalid();
        }
        let author = parts[parts.len() - 2];
        let repo = parts[parts.len() - 1];
        let valid = |s: &str| {
            s != "."
                && s != ".."
                && !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        };
        if !valid(author) || !valid(repo) {
            return invalid();
        }
        let api_base = origin.map(|base| {
            let prefix = parts[..parts.len() - 2]
                .iter()
                .map(|part| format!("/{part}"))
                .collect::<String>();
            format!("{base}{prefix}")
        });
        Ok(Self {
            author: author.to_owned(),
            repo: repo.to_owned(),
            api_base,
        })
    }
}

/// The release asset target matching `.github/workflows/release.yml`;
/// `None` for platforms the workflow does not publish.
pub fn target_triple() -> Option<&'static str> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("x86_64-pc-windows-msvc")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("aarch64-apple-darwin")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("x86_64-unknown-linux-musl")
    } else {
        None
    }
}

/// The running binary's version, as embedded at build time.
pub const fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The cargo install directory inside a cargo home: `cargo install` puts binaries in
/// `<cargo home>/bin` and tracks their versions in its install list.
/// `CARGO_HOME` wins over the home directory, like cargo itself.
fn cargo_bin_dir(cargo_home_env: Option<&std::ffi::OsStr>, home_env: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let home = match cargo_home_env {
        Some(cargo_home) => PathBuf::from(cargo_home),
        None => PathBuf::from(home_env?).join(".cargo"),
    };
    Some(home.join("bin"))
}

/// True when the directory of the running binary is the cargo install dir.
fn is_cargo_install_dir(exe_dir: &Path, cargo_bin: &Path) -> bool {
    exe_dir == cargo_bin
}

/// True when the running binary is a `cargo install` artifact (it sits in the cargo
/// bin dir): cargo tracks its version, so a self-update desyncs cargo's bookkeeping
/// and the update line points at cargo instead of the self-update.
pub fn cargo_managed_install() -> bool {
    let exe = std::env::current_exe().ok();
    let exe_dir = exe.as_deref().and_then(|exe| exe.parent()).and_then(canonical);
    let cargo_home = std::env::var_os("CARGO_HOME");
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    let cargo_bin = cargo_bin_dir(cargo_home.as_deref(), home.as_deref()).and_then(|dir| canonical(&dir));
    match (exe_dir, cargo_bin) {
        (Some(exe_dir), Some(cargo_bin)) => is_cargo_install_dir(&exe_dir, &cargo_bin),
        _ => false,
    }
}

/// Canonicalizes for path comparison; a path that does not exist cannot match.
fn canonical(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// Release notes of one release, newest-first in the delta list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseNote {
    pub version: String,
    pub published: String,
    pub body: String,
}

/// Outcome of a release check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// Releases newer than the current version, newest first.
    Available { latest: String, notes: Vec<ReleaseNote> },
    /// The running version is the newest published one.
    UpToDate,
    /// The repository could not be read: offline, moved, renamed, deleted, forked away.
    Unreachable { repo: String, reason: String },
}

/// Outcome of a performed update.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PerformOutcome {
    Updated {
        version: String,
    },
    UpToDate,
    Failed {
        reason: String,
        /// Suggest pointing at another repository url (network, api and 404 failures).
        suggest_repo_url: bool,
        /// The failure is a permission denial on the binary's directory: suggest `sudo silv update`.
        suggest_sudo: bool,
    },
}

/// Event flowing from a background worker thread to the TUI.
/// `Checked` carries the check generation that was active when the worker started;
/// the TUI drops events from a superseded generation (a repository url was submitted
/// while the old check was in flight).
#[derive(Clone, Debug)]
pub enum UpdateEvent {
    Checked(u64, UpdateOutcome),
    Performed(PerformOutcome),
}

/// The update target: repository, running version and release asset triple.
#[derive(Clone, Debug)]
pub struct UpdateTarget {
    pub repo: RepoRef,
    pub current: String,
}

impl UpdateTarget {
    pub fn new(repo: RepoRef) -> Self {
        Self {
            repo,
            current: current_version().to_owned(),
        }
    }

    fn builder(&self, timeout: Duration) -> self_update::backends::github::UpdateBuilder {
        let mut builder = self_update::backends::github::Update::configure();
        builder
            .repo_owner(&self.repo.author)
            .repo_name(&self.repo.repo)
            .current_version(&self.current)
            .bin_name("silv")
            .auth_token_from_env()
            .timeout(timeout);
        if self.repo.api_base.is_some() {
            builder.api_base_url(self.repo.api_base());
        }
        builder
    }

    /// Fetches the releases newer than the current version: drives the update line
    /// and the changelog overlay (the release bodies are the delta).
    /// Semver prereleases are filtered out: a stable release train may publish
    /// release candidates, and silv users must not be pulled onto one by the update line.
    pub fn check(&self) -> UpdateOutcome {
        let Some(triple) = target_triple() else {
            return UpdateOutcome::Unreachable {
                repo: self.repo.url(),
                reason: format!(
                    "no release is published for this platform ({}-{})",
                    std::env::consts::ARCH,
                    std::env::consts::OS
                ),
            };
        };
        let releases = match self.builder(Duration::from_secs(10)).target(triple).build() {
            Ok(update) => match update.get_newer_releases() {
                Ok(releases) => releases,
                Err(cause) => return unreachable_outcome(&self.repo, &cause),
            },
            Err(cause) => return unreachable_outcome(&self.repo, &cause),
        };
        let notes: Vec<ReleaseNote> = releases
            .all()
            .iter()
            .filter(|release| !is_prerelease(release.version()))
            .map(|release| ReleaseNote {
                version: release.version().to_owned(),
                published: release.date().split('T').next().unwrap_or_default().to_owned(),
                body: release.body().unwrap_or_default().trim().to_owned(),
            })
            .collect();
        match notes.first() {
            None => UpdateOutcome::UpToDate,
            Some(newest) => UpdateOutcome::Available {
                latest: newest.version.clone(),
                notes,
            },
        }
    }

    /// Downloads a release and replaces the running binary in place.
    /// `version` pins that exact release (the one the check presented, tagged
    /// `v<version>` by `release.yml`); `None` falls back to the newest release overall.
    /// A repository that tags differently answers 404 for the pinned tag: then the
    /// unpinned path runs, so forks with their own tag scheme still update.
    pub fn perform(&self, version: Option<&str>) -> PerformOutcome {
        let Some(triple) = target_triple() else {
            return PerformOutcome::Failed {
                reason: format!(
                    "no release is published for this platform ({}-{})",
                    std::env::consts::ARCH,
                    std::env::consts::OS
                ),
                suggest_repo_url: false,
                suggest_sudo: false,
            };
        };
        let pinned = self.try_perform(triple, version);
        match pinned {
            Ok(outcome) => outcome,
            Err(cause) => {
                let pinned_tag_missing = matches!(cause, self_update::Error::NotFound { .. });
                if version.is_some() && pinned_tag_missing {
                    match self.try_perform(triple, None) {
                        Ok(outcome) => outcome,
                        Err(fallback) => perform_failure(&fallback),
                    }
                } else {
                    perform_failure(&cause)
                }
            },
        }
    }

    fn try_perform(&self, triple: &str, version: Option<&str>) -> Result<PerformOutcome, self_update::Error> {
        let mut builder = self.builder(Duration::from_secs(120));
        builder.target(triple).no_confirm(true).show_output(false);
        match version {
            Some(version) => {
                builder.release_tag(format!("v{version}"));
            },
            None => {
                builder.update_strategy(self_update::update::UpdateStrategy::Latest);
            },
        }
        let status = builder.build()?.update()?;
        if status.is_up_to_date() {
            Ok(PerformOutcome::UpToDate)
        } else {
            Ok(PerformOutcome::Updated {
                version: status.version().to_owned(),
            })
        }
    }
}

/// True when a release version is a semver prerelease (`1.2.0-rc.1`), or does not
/// parse as semver at all; such releases never drive the update line.
fn is_prerelease(version: &str) -> bool {
    semver::Version::parse(version).map_or(true, |version| !version.pre.is_empty())
}

/// True when the io error is a permission denial: the binary's directory is not
/// writable by this user (a root-owned system install).
fn is_permission_error(cause: &std::io::Error) -> bool {
    cause.kind() == std::io::ErrorKind::PermissionDenied
}

fn unreachable_outcome(repo: &RepoRef, cause: &self_update::Error) -> UpdateOutcome {
    UpdateOutcome::Unreachable {
        repo: repo.url(),
        reason: describe(cause),
    }
}

fn perform_failure(cause: &self_update::Error) -> PerformOutcome {
    let reason = describe(cause);
    let suggest_sudo = chain_permission_denied(cause);
    PerformOutcome::Failed {
        reason,
        suggest_sudo,
        // A missing asset for this target, or a write permission denial, is not a
        // repository problem: pointing at another url would not help.
        suggest_repo_url: !suggest_sudo && !matches!(cause, self_update::Error::NoReleaseFound { target: Some(_), .. }),
    }
}

/// Walks the `std::error::Error` source chain for a `PermissionDenied` io error
/// (self_update nests the io error of the failed replace, ureq nests its own).
fn chain_permission_denied(cause: &(dyn std::error::Error + 'static)) -> bool {
    if let Some(io) = cause.downcast_ref::<std::io::Error>() {
        return is_permission_error(io);
    }
    match std::error::Error::source(cause) {
        Some(source) => chain_permission_denied(source),
        None => false,
    }
}

/// Turns a self_update error into a one-line human reason.
fn describe(cause: &self_update::Error) -> String {
    match cause {
        self_update::Error::NotFound { url, .. } => format!(
            "github answered 404 for {url}: the repository has no releases, was renamed, deleted, is private, or is a fork that publishes none"
        ),
        self_update::Error::Unauthorized { status, .. } => {
            format!("github api answered {status}: the credentials were rejected; check GITHUB_TOKEN")
        },
        self_update::Error::RateLimited { .. } => {
            "github api rate limit reached (unauthenticated requests are limited); set GITHUB_TOKEN to raise it"
                .to_owned()
        },
        self_update::Error::HttpStatus { status, .. } => format!("github api answered {status}"),
        self_update::Error::NoReleaseFound {
            target: Some(target), ..
        } => {
            format!("no release asset for target `{target}`")
        },
        self_update::Error::NoReleaseFound { target: None, .. } => "the repository publishes no releases".to_owned(),
        _ => {
            let mut reason = cause.to_string();
            let mut source = std::error::Error::source(cause);
            while let Some(cause) = source {
                reason.push_str(": ");
                reason.push_str(&cause.to_string());
                source = std::error::Error::source(cause);
            }
            reason
        },
    }
}

/// The cargo command that updates a `cargo install` binary in place, keeping cargo's
/// install list in sync.
fn cargo_install_command(repo: &RepoRef) -> String {
    format!("cargo install --git {} --force", repo.url())
}

/// Runs the `silv update` subcommand. `force` overrides the cargo-install refusal.
pub fn run_cli(check: bool, force: bool, repo: &RepoRef) -> Result<()> {
    let cargo_managed = cargo_managed_install();
    let target = UpdateTarget::new(repo.clone());
    eprintln!("silv: checking releases at {repo} (running v{})", target.current);
    let outcome = target.check();
    match outcome {
        UpdateOutcome::Unreachable { repo, reason } => Err(SilvError::Update {
            message: format!("cannot read releases at {repo}: {reason}"),
            help: "if silv moved, was renamed, or you track a fork, point at the repository: \
                   `silv update --repo-url https://github.com/owner/repo`, set GITHUB_TOKEN to avoid api rate limits"
                .to_string(),
        }
        .into()),
        UpdateOutcome::UpToDate => {
            eprintln!("silv v{} is up to date", target.current);
            Ok(())
        },
        UpdateOutcome::Available { latest, notes } => {
            if check {
                eprintln!("update available: silv {} -> {}", target.current, latest);
                eprintln!();
                eprintln!("changelog, the releases between silv {} and {latest}:", target.current);
                print_notes(&notes);
                eprintln!();
                if cargo_managed {
                    // cargo tracks this binary's version: a self-update desyncs its install list.
                    eprintln!("this binary is a cargo install, update it with: `{}`", cargo_install_command(repo));
                    eprintln!("(`silv update` works too, it only leaves cargo's install list behind)");
                } else {
                    eprintln!("run `silv update` to install it");
                }
                // grep-like semantics: exit 1 signals "an update was found", so scripts
                // can branch on the exit code; 0 means up to date.
                return Err(SilvError::Update {
                    message: format!("update available: silv {} -> {latest}", target.current),
                    help: "run `silv update` to install it; --check exits 1 when an update is available, 0 when up to date"
                        .to_string(),
                }
                .into());
            }
            if cargo_managed && !force {
                // Replacing a cargo-tracked binary on its own is a known breakage of
                // cargo's bookkeeping; refuse it, the cargo command is the clean path.
                print_notes(&notes);
                return Err(SilvError::Update {
                    message: "this binary is a cargo install, refusing to replace it".to_string(),
                    help: format!(
                        "update it with `{}`: cargo tracks this binary's version, a self-update leaves that \
                         install list behind; pass --force to replace it anyway (`{}` re-syncs cargo)",
                        cargo_install_command(repo),
                        cargo_install_command(repo)
                    ),
                }
                .into());
            }
            if cargo_managed {
                eprintln!(
                    "note: --force on a cargo install: cargo's install list keeps the old version, `{}` re-syncs it",
                    cargo_install_command(repo)
                );
            }
            eprintln!("updating silv {} -> {} ...", target.current, latest);
            print_notes(&notes);
            match target.perform(Some(&latest)) {
                PerformOutcome::Updated { version } => {
                    eprintln!("silv updated to {version}; restart silv to use it");
                    Ok(())
                },
                PerformOutcome::UpToDate => {
                    eprintln!("silv v{} is up to date", target.current);
                    Ok(())
                },
                PerformOutcome::Failed {
                    reason,
                    suggest_repo_url,
                    suggest_sudo,
                } => Err(SilvError::Update {
                    message: format!("update failed: {reason}"),
                    help: if suggest_sudo {
                        "the directory containing this binary is not writable: system installs need \
                         `sudo silv update` (with sudo -E to pass GITHUB_TOKEN)"
                            .to_owned()
                    } else if suggest_repo_url {
                        "if silv moved, was renamed, or you track a fork, retry with: \
                         `silv update --repo-url https://github.com/owner/repo`, set GITHUB_TOKEN to avoid api rate limits"
                            .to_owned()
                    } else {
                        "check that the release publishes an asset for this platform".to_owned()
                    },
                }
                .into()),
            }
        },
    }
}

fn print_notes(notes: &[ReleaseNote]) {
    for note in notes {
        eprintln!("  v{}{}", note.version, date_suffix(&note.published));
        for line in note.body.lines() {
            eprintln!("    {line}");
        }
    }
}

fn date_suffix(published: &str) -> String {
    if published.is_empty() {
        String::new()
    } else {
        format!(" ({published})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_ref_accepts_the_documented_forms() {
        let expected = RepoRef {
            author: "chamoretto".to_owned(),
            repo: "silv".to_owned(),
            api_base: None,
        };
        for input in [
            "https://github.com/chamoretto/silv",
            "https://github.com/chamoretto/silv/",
            "https://github.com/chamoretto/silv.git",
            "http://github.com/chamoretto/silv",
            "git@github.com:chamoretto/silv.git",
            "ssh://git@github.com/chamoretto/silv.git",
            "chamoretto/silv",
            "  chamoretto/silv  ",
        ] {
            assert_eq!(RepoRef::from_str(input).ok().as_ref(), Some(&expected), "accepts {input}");
        }
        assert_eq!(expected.url(), "https://github.com/chamoretto/silv");
        assert_eq!(expected.api_base(), "https://api.github.com");
    }

    #[test]
    fn repo_ref_keeps_a_custom_api_origin() {
        let repo = RepoRef::from_str("http://127.0.0.1:8877/you/fork").expect("parses");
        assert_eq!(repo.author, "you");
        assert_eq!(repo.repo, "fork");
        assert_eq!(repo.api_base().as_str(), "http://127.0.0.1:8877");
        assert_eq!(repo.url(), "http://127.0.0.1:8877/you/fork");
        let ghe = RepoRef::from_str("https://git.corp.example/api/v3/team/tool").expect("parses");
        assert_eq!(ghe.api_base(), "https://git.corp.example/api/v3");
        assert_eq!(ghe.author, "team");
        assert_eq!(ghe.repo, "tool");
        let deep = RepoRef::from_str("https://example.com/a/b/c/d").expect("parses");
        assert_eq!(deep.api_base(), "https://example.com/a/b");
        assert_eq!(deep.url(), "https://example.com/a/b/c/d");
    }

    #[test]
    fn repo_ref_rejects_non_repo_strings() {
        for input in [
            "https://github.com/chamoretto",
            "https://github.com/chamoretto/silv/extra",
            "http://127.0.0.1:8877/only-one-segment",
            "http:///you/fork",
            "",
            "silv",
            "https://github.com/../silv",
            "chamoretto/../silv",
        ] {
            assert!(RepoRef::from_str(input).is_err(), "rejects {input}");
        }
    }

    #[test]
    fn default_repo_is_the_cargo_repository() {
        let repo = RepoRef::default_repo();
        assert_eq!(repo.author, "chamoretto");
        assert_eq!(repo.repo, "silv");
    }

    #[test]
    fn target_matches_the_release_matrix() {
        if cfg!(target_os = "windows") {
            assert_eq!(target_triple(), Some("x86_64-pc-windows-msvc"));
        }
    }

    #[test]
    fn current_version_is_the_crate_version() {
        assert_eq!(current_version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn cargo_bin_dir_follows_the_cargo_home_rules() {
        // CARGO_HOME wins over the home directory, like cargo itself.
        assert_eq!(
            cargo_bin_dir(Some(std::ffi::OsStr::new("/opt/cargo")), Some(std::ffi::OsStr::new("/home/x"))),
            Some(PathBuf::from("/opt/cargo/bin"))
        );
        assert_eq!(
            cargo_bin_dir(None, Some(std::ffi::OsStr::new("/home/x"))),
            Some(PathBuf::from("/home/x/.cargo/bin"))
        );
        assert_eq!(cargo_bin_dir(None, None), None, "no home, no cargo bin dir");
    }

    #[test]
    fn cargo_install_dir_matches_only_the_cargo_bin_dir() {
        let cargo_bin = PathBuf::from("/home/x/.cargo/bin");
        assert!(is_cargo_install_dir(Path::new("/home/x/.cargo/bin"), &cargo_bin));
        assert!(!is_cargo_install_dir(Path::new("/home/x/.local/bin"), &cargo_bin), "a user bin dir is self-managed");
        assert!(
            !is_cargo_install_dir(Path::new("/home/x/project/target/debug"), &cargo_bin),
            "a dev build is self-managed"
        );
    }

    #[test]
    fn the_test_binary_is_not_a_cargo_install() {
        // The test binary runs from the cargo target dir, never from the cargo bin dir.
        assert!(!cargo_managed_install(), "cargo test runs from target/, not from ~/.cargo/bin");
    }

    #[test]
    fn describe_maps_api_errors_to_reasons() {
        let not_found = self_update::Error::http_status_error(404, "u");
        assert!(describe(&not_found).contains("404"));
        let limited = self_update::Error::http_status_error(403, "u");
        assert!(describe(&limited).contains("403"));
        let no_asset = self_update::Error::no_release_found_for_target("x86_64-pc-windows-msvc");
        assert!(describe(&no_asset).contains("x86_64-pc-windows-msvc"));
        let no_release = self_update::Error::no_release_found();
        assert!(describe(&no_release).contains("publishes no releases"));
    }

    #[test]
    fn prerelease_filter_follows_semver() {
        assert!(is_prerelease("1.2.0-rc.1"));
        assert!(is_prerelease("0.3.0-alpha.2026-11-01"));
        assert!(is_prerelease("not-a-version"));
        assert!(!is_prerelease("1.2.0"));
        assert!(!is_prerelease("0.3.0"));
    }

    #[test]
    fn perform_failure_flags_a_permission_denied_chain() {
        // self_update nests the io error of the failed replace under its own error.
        let denied = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "read-only dir");
        let wrapped = self_update::Error::Io(denied);
        let outcome = perform_failure(&wrapped);
        match outcome {
            PerformOutcome::Failed {
                suggest_sudo,
                suggest_repo_url,
                reason,
            } => {
                assert!(suggest_sudo, "a nested PermissionDenied suggests sudo: {reason}");
                assert!(!suggest_repo_url, "a permission failure is not a repo problem");
            },
            other => panic!("expected a failure, got {other:?}"),
        }
        let plain = std::io::Error::new(std::io::ErrorKind::TimedOut, "network");
        match perform_failure(&self_update::Error::Io(plain)) {
            PerformOutcome::Failed {
                suggest_sudo,
                suggest_repo_url,
                ..
            } => {
                assert!(!suggest_sudo);
                assert!(suggest_repo_url, "a network failure may be a moved repo");
            },
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn perform_failure_does_not_suggest_a_repo_for_a_missing_asset() {
        let missing = self_update::Error::no_release_found_for_target("x86_64-pc-windows-msvc");
        match perform_failure(&missing) {
            PerformOutcome::Failed {
                suggest_repo_url,
                suggest_sudo,
                reason,
            } => {
                assert!(!suggest_repo_url, "the repo is fine, the asset for this target is not: {reason}");
                assert!(!suggest_sudo);
            },
            other => panic!("expected a failure, got {other:?}"),
        }
    }
}
