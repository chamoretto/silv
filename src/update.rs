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

    /// The `git clone` url for `cargo install --git`: the github web url, or a custom
    /// base's origin plus owner/repo. The api prefix (`/api/v3` style) is not part of
    /// a clone path, so it is stripped back to `scheme://host`.
    pub fn clone_url(&self) -> String {
        match &self.api_base {
            None => format!("https://github.com/{}/{}.git", self.author, self.repo),
            Some(base) => format!("{}/{}/{}.git", origin_of(base), self.author, self.repo),
        }
    }
}

/// The `scheme://host` prefix of an api base url.
fn origin_of(api_base: &str) -> String {
    match api_base.split_once("://") {
        Some((scheme, rest)) => format!("{scheme}://{}", rest.split('/').next().unwrap_or_default()),
        None => api_base.split('/').next().unwrap_or_default().to_owned(),
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

/// The shared reason for a platform the release workflow does not publish.
fn no_release_reason() -> String {
    format!(
        "no release is published for this platform ({}-{})",
        std::env::consts::ARCH,
        std::env::consts::OS
    )
}

/// The running binary's version, as embedded at build time.
pub const fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// The cargo install directory inside a cargo home: `cargo install` puts binaries in
/// `<cargo home>/bin` and tracks their versions in its install list.
/// `CARGO_HOME` wins over the home directory, like cargo itself; an empty environment
/// variable counts as unset (it is set and empty in common container images).
/// `home1`/`home2` are the home variables in platform order (`USERPROFILE` first on
/// Windows, `HOME` first on unix).
fn cargo_bin_dir(
    cargo_home_env: Option<&std::ffi::OsStr>,
    home1: Option<&std::ffi::OsStr>,
    home2: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    let home = match non_empty_os(cargo_home_env) {
        Some(cargo_home) => PathBuf::from(cargo_home),
        None => PathBuf::from(non_empty_os(home1).or_else(|| non_empty_os(home2))?).join(".cargo"),
    };
    Some(home.join("bin"))
}

fn non_empty_os(value: Option<&std::ffi::OsStr>) -> Option<&std::ffi::OsStr> {
    value.filter(|value| !value.is_empty())
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
    let (home1, home2) = if cfg!(windows) {
        (std::env::var_os("USERPROFILE"), std::env::var_os("HOME"))
    } else {
        (std::env::var_os("HOME"), std::env::var_os("USERPROFILE"))
    };
    let cargo_bin =
        cargo_bin_dir(cargo_home.as_deref(), home1.as_deref(), home2.as_deref()).and_then(|dir| canonical(&dir));
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

    /// The release api builder. `GITHUB_TOKEN` credentials go to the default
    /// api.github.com target only: a custom `--repo-url` host (a fork server, a local
    /// test api, plain http) must never receive them. GitHub Enterprise users run
    /// unauthenticated and see the api's own rate limits.
    fn builder(&self, timeout: Duration) -> self_update::backends::github::UpdateBuilder {
        let mut builder = self_update::backends::github::Update::configure();
        builder
            .repo_owner(&self.repo.author)
            .repo_name(&self.repo.repo)
            .current_version(&self.current)
            .bin_name("silv")
            .timeout(timeout);
        match &self.repo.api_base {
            None => {
                builder.auth_token_from_env();
            },
            Some(base) => {
                builder.api_base_url(base.clone());
            },
        }
        builder
    }

    /// Fetches the releases newer than the current version: drives the update line
    /// and the changelog overlay (the release bodies are the delta).
    /// Semver prereleases are filtered out: a stable release train may publish
    /// release candidates, and silv users must not be pulled onto one by the update line.
    /// Releases without an asset for this platform's target are filtered out too, and
    /// the list is sorted by semver descending: self_update documents no ordering
    /// guarantee, so the newest release must not depend on the backend's list order.
    pub fn check(&self) -> UpdateOutcome {
        let Some(triple) = target_triple() else {
            return UpdateOutcome::Unreachable {
                repo: self.repo.url(),
                reason: no_release_reason(),
            };
        };
        let releases = match self.builder(Duration::from_secs(10)).target(triple).build() {
            Ok(update) => match update.get_newer_releases() {
                Ok(releases) => releases,
                Err(cause) => return unreachable_outcome(&self.repo, &cause),
            },
            Err(cause) => return unreachable_outcome(&self.repo, &cause),
        };
        let mut notes: Vec<ReleaseNote> = releases
            .all()
            .iter()
            .filter(|release| !is_prerelease(release.version()) && release.has_target_asset(triple))
            .map(|release| ReleaseNote {
                version: release.version().to_owned(),
                published: release.date().split('T').next().unwrap_or_default().to_owned(),
                body: release.body().unwrap_or_default().trim().to_owned(),
            })
            .collect();
        // Every version here passed the semver filter, so the keys parse.
        notes.sort_by_key(|note| std::cmp::Reverse(version_key(&note.version)));
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
    /// `v<version>` by `release.yml`). A repository that tags differently answers 404
    /// for the pinned tag: `fallback_perform` installs the same stable release through
    /// the fork's tag scheme, or through the listing when that cannot pick a prerelease.
    pub fn perform(&self, version: Option<&str>) -> PerformOutcome {
        let Some(triple) = target_triple() else {
            return PerformOutcome::Failed {
                reason: no_release_reason(),
                suggest_repo_url: false,
                suggest_sudo: false,
            };
        };
        let pinned = version.map(|version| format!("v{version}"));
        match self.try_perform(triple, pinned.as_deref()) {
            Ok(outcome) => outcome,
            Err(cause) => {
                if pinned.is_some() && matches!(cause, self_update::Error::NotFound { .. }) {
                    self.fallback_perform(triple, &cause)
                } else {
                    perform_failure(&cause)
                }
            },
        }
    }

    /// The pinned `v<version>` tag 404s: the repository does not tag releases the way
    /// `release.yml` does (forks use their own schemes). Bounded fallback: re-check for
    /// the newest installable stable, try the bare `<version>` tag (the `9.9.8` scheme),
    /// and use the backend's listing-based choice only when that choice is the very
    /// stable version the check presented (the listing counts prereleases, so the
    /// newest release over all must not be newer). No recursion, at most one install.
    fn fallback_perform(&self, triple: &str, cause: &self_update::Error) -> PerformOutcome {
        match self.check() {
            UpdateOutcome::UpToDate => PerformOutcome::UpToDate,
            // The re-check failed too: report the original install failure, it is the concrete one.
            UpdateOutcome::Unreachable { .. } => perform_failure(cause),
            UpdateOutcome::Available { latest, .. } => match self.try_perform(triple, Some(&latest)) {
                Ok(outcome) => outcome,
                Err(bare) => {
                    if !matches!(bare, self_update::Error::NotFound { .. }) {
                        return perform_failure(&bare);
                    }
                    // The tags address releases by ids only: the listing strategy is the
                    // last option, safe only when the newest release over all is the
                    // presented stable one (string-equal: both names come from this api).
                    if self.newest_overall(triple).as_deref() == Some(latest.as_str()) {
                        match self.try_perform(triple, None) {
                            Ok(outcome) => outcome,
                            Err(listing) => perform_failure(&listing),
                        }
                    } else {
                        perform_failure(&bare)
                    }
                },
            },
        }
    }

    /// The newest release carrying an asset for `triple`, prereleases included
    /// (the listing-based `Latest` strategy picks prereleases too): exactly the
    /// release that strategy would install.
    fn newest_overall(&self, triple: &str) -> Option<String> {
        let update = self.builder(Duration::from_secs(10)).target(triple).build().ok()?;
        let releases = update.get_newer_releases().ok()?;
        releases
            .all()
            .iter()
            .filter(|release| release.has_target_asset(triple))
            .map(|release| release.version())
            .max_by_key(|version| version_key(version))
            .map(|version| version.to_owned())
    }

    /// One install attempt. `tag` pins the release with that exact tag; `None` lets
    /// the backend choose the newest release from the listing.
    fn try_perform(&self, triple: &str, tag: Option<&str>) -> Result<PerformOutcome, self_update::Error> {
        let mut builder = self.builder(Duration::from_secs(120));
        builder.target(triple).no_confirm(true).show_output(false);
        match tag {
            Some(tag) => {
                builder.release_tag(tag.to_owned());
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
/// parse as semver at all; such releases never drive the update line. Build metadata
/// (`1.2.3+build`) is not a prerelease and passes.
fn is_prerelease(version: &str) -> bool {
    semver::Version::parse(version).map_or(true, |version| !version.pre.is_empty())
}

/// Semver sort key for the newest-first ordering of the filtered notes; every
/// version passed the semver filter before reaching it.
fn version_key(version: &str) -> semver::Version {
    semver::Version::parse(version).unwrap_or_else(|_| semver::Version::new(0, 0, 0))
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
        // Not repository problems: a missing asset for this target, a write permission
        // denial, api credentials being rejected, or the api rate limit. Pointing at
        // another url would not help any of them.
        suggest_repo_url: !suggest_sudo
            && !matches!(
                cause,
                self_update::Error::NoReleaseFound { target: Some(_), .. }
                    | self_update::Error::RateLimited { .. }
                    | self_update::Error::Unauthorized { .. }
            ),
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
    format!("cargo install --git {} --force", repo.clone_url())
}

/// Runs the `silv update` subcommand. `force` overrides the cargo-install refusal.
///
/// Exit codes (mapped by the caller, grep-like): 0 = up to date / installed, 1 = an
/// update was found (`--check` signal, `silv::update::failed`), 2 = trouble: the
/// repository is unreachable, the install failed, a cargo install was refused
/// (`silv::update::unavailable`).
pub fn run_cli(check: bool, force: bool, repo: &RepoRef) -> Result<()> {
    let cargo_managed = cargo_managed_install();
    let target = UpdateTarget::new(repo.clone());
    eprintln!("silv: checking releases at {repo} (running v{})", target.current);
    let outcome = target.check();
    match outcome {
        UpdateOutcome::Unreachable { repo, reason } => Err(SilvError::UpdateUnavailable {
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
                    help: "run `silv update` to install it; exit code: 1 = update available, \
                           0 = up to date, 2 = the check failed"
                        .to_string(),
                }
                .into());
            }
            if cargo_managed && !force {
                // Replacing a cargo-tracked binary on its own is a known breakage of
                // cargo's bookkeeping; refuse it, the cargo command is the clean path.
                print_notes(&notes);
                return Err(SilvError::UpdateUnavailable {
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
                } => Err(SilvError::UpdateUnavailable {
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
            cargo_bin_dir(Some(std::ffi::OsStr::new("/opt/cargo")), Some(std::ffi::OsStr::new("/home/x")), None),
            Some(PathBuf::from("/opt/cargo/bin"))
        );
        assert_eq!(
            cargo_bin_dir(None, Some(std::ffi::OsStr::new("/home/x")), None),
            Some(PathBuf::from("/home/x/.cargo/bin"))
        );
        // An empty variable counts as unset: set-and-empty CARGO_HOME falls back to the home.
        assert_eq!(
            cargo_bin_dir(Some(std::ffi::OsStr::new("")), Some(std::ffi::OsStr::new("/home/x")), None),
            Some(PathBuf::from("/home/x/.cargo/bin")),
            "an empty CARGO_HOME must not resolve to `bin`"
        );
        assert_eq!(
            cargo_bin_dir(None, None, Some(std::ffi::OsStr::new("/home/y"))),
            Some(PathBuf::from("/home/y/.cargo/bin")),
            "the second home variable is the fallback"
        );
        assert_eq!(cargo_bin_dir(None, None, None), None, "no home, no cargo bin dir");
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
        assert!(is_prerelease("1.2.0-rc.1+build"));
        assert!(is_prerelease("not-a-version"));
        assert!(!is_prerelease("1.2.0"));
        assert!(!is_prerelease("0.3.0"));
        assert!(!is_prerelease("1.2.3+build"), "build metadata is not a prerelease");
    }

    #[test]
    fn clone_urls_address_the_repository_for_cargo() {
        let github = RepoRef::from_str("chamoretto/silv").expect("parses");
        assert_eq!(github.clone_url(), "https://github.com/chamoretto/silv.git");
        let ghe = RepoRef::from_str("https://git.corp.example/api/v3/team/tool").expect("parses");
        assert_eq!(
            ghe.clone_url(),
            "https://git.corp.example/team/tool.git",
            "the api prefix is stripped back to the origin"
        );
        let mock = RepoRef::from_str("http://127.0.0.1:8877/you/fork").expect("parses");
        assert_eq!(mock.clone_url(), "http://127.0.0.1:8877/you/fork.git", "the scheme is kept");
    }

    #[test]
    fn perform_failure_does_not_suggest_a_repo_for_api_rejections() {
        // Credential rejections and rate limits are the same api on any url: pointing
        // at another repository would not help.
        for (status, label) in [(401, "unauthorized"), (403, "forbidden"), (429, "rate limited")] {
            let cause = self_update::Error::http_status_error(status, "u");
            match perform_failure(&cause) {
                PerformOutcome::Failed {
                    suggest_repo_url,
                    suggest_sudo,
                    reason,
                } => {
                    assert!(!suggest_repo_url, "a {label} api answer is not a repo problem: {reason}");
                    assert!(!suggest_sudo);
                },
                other => panic!("expected a failure, got {other:?}"),
            }
        }
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
