//! SILV - simple interactive log viewer.
//!
//! Library entry point is [`run`], the binary target is a thin wrapper over it.

#[cfg(feature = "mem-bench")]
pub mod alloc_count;
pub mod app;
pub mod cli;
pub mod data;
pub mod error;
pub mod filter;
pub mod model;
pub mod parse;
pub mod render;
pub mod update;

use std::io::Write;
use std::time::Instant;

use clap::Parser;
use miette::{Context, IntoDiagnostic};

use crate::app::keymap::KeyMap;
use crate::cli::{Command, Config, RunMode};
use crate::data::LogStore;
use crate::update::RepoRef;

/// Parses arguments, loads input and dispatches to the requested mode.
pub fn run() -> miette::Result<()> {
    let started = Instant::now();
    let cli = cli::Cli::parse();
    let mut config = Config::from_cli(&cli).context("failed to parse command line arguments")?;
    config.validate().context("failed to start interactive mode")?;

    if let Some(Command::Update { check, repo_url }) = &cli.command {
        let repo = repo_url
            .clone()
            .or_else(|| config.update_repo.clone())
            .unwrap_or_else(RepoRef::default_repo);
        return update::run_cli(*check, &repo).context("silv update failed");
    }

    if config.key_map_show {
        return write_key_map(&config);
    }

    #[cfg(feature = "mem-bench")]
    if let Some(layer) = config.memory_report {
        return data::memreport::run(&config, layer).context("memory report failed");
    }

    let bundle = parse::input::read_inputs(&config)
        .with_context(|| format!("failed to read input from {}", config.sources_display()))?;
    let store = LogStore::build(&bundle, &config).context("failed to parse log records")?;

    if config.stats {
        write_stats(&store, &config, started)?;
    }
    warn_about_limits(&store, &config);

    match config.mode {
        RunMode::Interactive => app::run(store, &config),
        RunMode::Print => {
            let written = render::stream::emit(&store, &config).context("failed to write output")?;
            if written == 0 {
                if !config.quiet {
                    eprintln!("silv: no records matched");
                }
                std::process::exit(1);
            }
            Ok(())
        },
    }
}

/// Prints the resolved key map, one `from<TAB>to` line per entry: the `--key-map-show` report.
fn write_key_map(config: &Config) -> miette::Result<()> {
    let map = KeyMap::resolve(&config.key_map);
    let mut out = std::io::stdout();
    writeln!(out, "platform: {}", app::keymap::PLATFORM).into_diagnostic()?;
    writeln!(out, "key map: {}", map.origin()).into_diagnostic()?;
    writeln!(out, "entries: {}", map.len()).into_diagnostic()?;
    for (from, to) in map.entries() {
        writeln!(out, "{from}\t{to}").into_diagnostic()?;
    }
    Ok(())
}

fn warn_about_limits(store: &LogStore, config: &Config) {
    if config.quiet {
        return;
    }
    if store.truncated() {
        eprintln!(
            "silv: warning: input truncated at --max-records {}; raise the limit to load more",
            config.max_records
        );
    }
    if store.fields_capped() {
        eprintln!(
            "silv: warning: field discovery stopped at {} fields; the widest fields may be missing from the picker",
            crate::data::MAX_DISCOVERED_FIELDS
        );
    }
    if !store.issues().is_empty() {
        eprintln!(
            "silv: {} line(s) did not parse{}; use --stats to list them",
            store.issues().len(),
            if config.keep_bad_lines {
                ", kept as INVALID records"
            } else {
                ", dropped"
            }
        );
    }
}

fn write_stats(store: &LogStore, config: &Config, started: Instant) -> miette::Result<()> {
    let mut out = std::io::stderr();
    let total = store.records().len();
    let bad = store.issues().len();
    let mut line = format!(
        "silv: parsed {total} record(s) from {} source(s) in {:?} ({} field(s) discovered, {bad} bad line(s))",
        store.sources().len(),
        started.elapsed(),
        store.fields().len(),
    );
    if store.truncated() {
        line.push_str(&format!("; truncated at --max-records {}", config.max_records));
    }
    writeln!(out, "{line}").into_diagnostic()?;
    for source in store.sources() {
        writeln!(out, "silv:   {} ({} lines, {:?})", source.label, source.lines, source.kind).into_diagnostic()?;
    }
    Ok(())
}
