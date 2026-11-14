//! Full-payload pipeline comparison mirroring silv's record model.
//!
//! silv keeps the parsed DOM inside every `LogRecord` (`object:
//! Map<String, Value>`), so the parsed layer IS the resident payload: peak
//! tracks the final heap, not a parse-time transient. This harness parses
//! every line of a file straight into three payload representations and keeps
//! them all alive (single-threaded, no caps):
//!
//! - `LogRecord` (baseline, serde_json Map payload, real silv pipeline)
//! - payload with `ijson::IValue` object (serde desugar path)
//! - payload with `sonic_rs::Value` object (arena path)
//!
//! and reports live / peak / cumulative / allocations per record, answering
//! the ijson-vs-sonic question for silv's keep-the-DOM architecture.
//!
//! Build with `--features mem-bench` only; never part of a release build.
// Measurement mirrors: fields are kept alive on purpose, reads are not needed.
#![allow(dead_code)]

use std::error::Error;
use std::mem::size_of;
use std::path::Path;

use silv::alloc_count;
use silv::alloc_count::Counting;
use silv::data::memreport;
use silv::model::record::FieldAliases;
use silv::model::record::LogRecord;
use silv::parse;
use silv::parse::jsonl;

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

struct Run {
    name: &'static str,
    live: u64,
    peak: u64,
    total: u64,
    allocs: u64,
    residue: u64,
}

fn finish(name: &'static str) -> Run {
    let snap = alloc_count::snapshot();
    Run {
        name,
        live: snap.live,
        peak: snap.peak_live,
        total: snap.total,
        allocs: snap.allocs,
        residue: 0,
    }
}

fn run_baseline(lines: &[(u64, usize, &[u8])]) -> Run {
    let aliases = FieldAliases::default();
    alloc_count::enable();
    let mut records: Vec<LogRecord> = Vec::with_capacity(lines.len());
    for (index, (number, offset, body)) in lines.iter().enumerate() {
        if let Ok(record) = jsonl::parse_line(index as u32, *number, *offset, body, &aliases) {
            records.push(record);
        }
    }
    let mut run = finish("silv LogRecord (Map)");
    drop(records);
    run.residue = alloc_count::snapshot().live;
    alloc_count::disable();
    run
}

struct IJsonPayload {
    source: u32,
    line: u64,
    offset: usize,
    message: Option<String>,
    object: ijson::IValue,
}

fn run_ijson(lines: &[(u64, usize, &[u8])]) -> Run {
    alloc_count::enable();
    let mut payloads: Vec<IJsonPayload> = Vec::with_capacity(lines.len());
    for (index, (number, offset, body)) in lines.iter().enumerate() {
        let Ok(object) = serde_json::from_slice::<ijson::IValue>(body) else {
            continue;
        };
        let message = object
            .as_object()
            .and_then(|o| o.iter().find(|(k, _)| k.as_str() == "message"))
            .and_then(|(_, v)| v.as_string())
            .map(|s| s.as_str().to_owned());
        payloads.push(IJsonPayload {
            source: index as u32,
            line: *number,
            offset: *offset,
            message,
            object,
        });
    }
    let mut run = finish("payload IValue (ijson)");
    drop(payloads);
    run.residue = alloc_count::snapshot().live;
    alloc_count::disable();
    run
}

struct SonicPayload {
    source: u32,
    line: u64,
    offset: usize,
    message: Option<String>,
    object: sonic_rs::Value,
}

fn run_sonic(lines: &[(u64, usize, &[u8])]) -> Run {
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};
    alloc_count::enable();
    let mut payloads: Vec<SonicPayload> = Vec::with_capacity(lines.len());
    for (index, (number, offset, body)) in lines.iter().enumerate() {
        let Ok(object) = sonic_rs::from_slice::<sonic_rs::Value>(body) else {
            continue;
        };
        let message = object
            .as_object()
            .and_then(|o| o.iter().find(|(k, _)| *k == "message"))
            .and_then(|(_, v)| v.as_str())
            .map(str::to_owned);
        payloads.push(SonicPayload {
            source: index as u32,
            line: *number,
            offset: *offset,
            message,
            object,
        });
    }
    let mut run = finish("payload Value (sonic)");
    drop(payloads);
    run.residue = alloc_count::snapshot().live;
    alloc_count::disable();
    run
}

fn main() -> Result<(), Box<dyn Error>> {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        return Err("usage: pipeline_compare <file.jsonl>...".into());
    }
    println!(
        "static sizes: LogRecord {} IJsonPayload {} SonicPayload {}",
        size_of::<LogRecord>(),
        size_of::<IJsonPayload>(),
        size_of::<SonicPayload>(),
    );

    for path in files {
        let bytes = std::fs::read(Path::new(&path))?;
        let lines = parse::split_lines(&bytes);
        let n = lines.len().max(1) as f64;
        println!();
        println!(
            "== {} | input {} | lines {}",
            path,
            human(bytes.len() as u64),
            lines.len()
        );
        let runs = [run_baseline(&lines), run_ijson(&lines), run_sonic(&lines)];
        println!(
            "{:<24} {:>10} {:>10} {:>11} {:>11} {:>10} {:>10}",
            "payload", "live", "peak", "live/rec", "cumulative", "allocs/r", "residue"
        );
        for run in &runs {
            println!(
                "{:<24} {:>10} {:>10} {:>10.1} {:>11} {:>10.1} {:>10}",
                run.name,
                human(run.live),
                human(run.peak),
                run.live as f64 / n,
                human(run.total),
                run.allocs as f64 / n,
                human(run.residue),
            );
        }
        println!(
            "  process rss {} | peak rss {}",
            memreport::process_rss().map(human).unwrap_or_else(|| "?".into()),
            memreport::process_peak_rss().map(human).unwrap_or_else(|| "?".into()),
        );
    }
    Ok(())
}
