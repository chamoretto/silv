//! SILV - simple interactive log viewer.
//!
//! Library entry point is [`run`], the binary target is a thin wrapper over it.

pub mod app;
pub mod cli;
pub mod data;
pub mod error;
pub mod filter;
pub mod model;
pub mod parse;
pub mod render;

use std::io::Write;
use std::time::Instant;

use clap::Parser;
use miette::{Context, IntoDiagnostic, Result};

use crate::cli::{Config, RunMode};
use crate::data::LogStore;

/// Parses arguments, loads input and dispatches to the requested mode.
pub fn run() -> Result<()> {
    let started = Instant::now();
    let cli = cli::Cli::parse();
    let mut config = Config::from_cli(&cli).context("failed to parse command line arguments")?;
    config.validate().context("failed to start interactive mode")?;

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
