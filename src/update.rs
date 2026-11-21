//! Self-update: release checks, the update itself, and the `silv update` command.
//!
//! Releases are published by `.github/workflows/release.yml` as raw binaries named
//! `silv-<tag>-<target>[.exe]`; `self_update`'s github backend matches them by target.
//! `update()` performs download, extraction and in-place self-replace (via the
//! self-replace and tempfile crates, internal dependencies of self_update).

use std::fmt::Display;
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
    },
}

/// Event flowing from a background worker thread to the TUI.
#[derive(Clone, Debug)]
pub enum UpdateEvent {
    Checked(UpdateOutcome),
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
        if let Some(triple) = target_triple() {
            builder.target(triple);
        }
        builder
    }

    /// Fetches the releases newer than the current version: drives the update line
    /// and the changelog overlay (the release bodies are the delta).
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

    /// Downloads the newest release and replaces the running binary in place.
    pub fn perform(&self) -> PerformOutcome {
        let Some(triple) = target_triple() else {
            return PerformOutcome::Failed {
                reason: format!(
                    "no release is published for this platform ({}-{})",
                    std::env::consts::ARCH,
                    std::env::consts::OS
                ),
                suggest_repo_url: false,
            };
        };
        let update = self
            .builder(Duration::from_secs(120))
            .target(triple)
            .no_confirm(true)
            .show_output(false)
            .update_strategy(self_update::update::UpdateStrategy::Latest)
            .build();
        let update = match update {
            Ok(update) => update,
            Err(cause) => return perform_failure(&cause),
        };
        let status = match update.update() {
            Ok(status) => status,
            Err(cause) => return perform_failure(&cause),
        };
        if status.is_up_to_date() {
            PerformOutcome::UpToDate
        } else {
            PerformOutcome::Updated {
                version: status.version().to_owned(),
            }
        }
    }
}

fn unreachable_outcome(repo: &RepoRef, cause: &self_update::Error) -> UpdateOutcome {
    UpdateOutcome::Unreachable {
        repo: repo.url(),
        reason: describe(cause),
    }
}

fn perform_failure(cause: &self_update::Error) -> PerformOutcome {
    let reason = describe(cause);
    PerformOutcome::Failed {
        suggest_repo_url: !reason.starts_with("no release asset"),
        reason,
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

/// Runs the `silv update` subcommand.
pub fn run_cli(check: bool, repo: &RepoRef) -> Result<()> {
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
                print_notes(&notes);
                eprintln!("run `silv update` to install it");
                return Ok(());
            }
            eprintln!("updating silv {} -> {} ...", target.current, latest);
            print_notes(&notes);
            match target.perform() {
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
                } => Err(SilvError::Update {
                    message: format!("update failed: {reason}"),
                    help: if suggest_repo_url {
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
}
