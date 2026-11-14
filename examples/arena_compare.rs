//! thunderdome generational-arena cost probe against `Vec`.
//!
//! Builds silv `LogRecord`s from corpus files into a `Vec<LogRecord>` and a
//! `thunderdome::Arena<LogRecord>`, measuring live heap, allocations and the
//! arena's structural bookkeeping overhead per record. Isolates the arena
//! bookkeeping with `Arena<u64>` insertions.
//!
//! Build with `--features mem-bench` only; never part of a release build.
// Measurement mirrors: fields are kept alive on purpose, reads are not needed.
#![allow(dead_code)]

use std::error::Error;
use std::mem::size_of;
use std::path::Path;

use silv::alloc_count;
use silv::alloc_count::Counting;
use silv::model::record::FieldAliases;
use silv::model::record::LogRecord;
use silv::parse;
use silv::parse::jsonl;
use thunderdome::Arena;

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

fn build_records(lines: &[(u64, usize, &[u8])]) -> Vec<LogRecord> {
    let aliases = FieldAliases::default();
    let mut records = Vec::with_capacity(lines.len());
    for (index, (number, offset, body)) in lines.iter().enumerate() {
        if let Ok(record) = jsonl::parse_line(index as u32, *number, *offset, body, &aliases) {
            records.push(record);
        }
    }
    records
}

fn main() -> Result<(), Box<dyn Error>> {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        return Err("usage: arena_compare <file.jsonl>...".into());
    }
    println!("static: LogRecord {} B, thunderdome Index {} B", size_of::<LogRecord>(), size_of::<thunderdome::Index>());

    for path in files {
        let bytes = std::fs::read(Path::new(&path))?;
        let lines = parse::split_lines(&bytes);
        let n = lines.len().max(1) as u64;

        // Baseline: Vec<LogRecord> (allocation counting over the build only).
        alloc_count::enable();
        let records = build_records(&lines);
        let vec_snap = alloc_count::snapshot();
        drop(records);
        alloc_count::disable();

        // Arena: same records moved into thunderdome nodes.
        alloc_count::enable();
        let mut arena = Arena::with_capacity(lines.len());
        for record in build_records(&lines) {
            arena.insert(record);
        }
        let arena_snap = alloc_count::snapshot();
        drop(arena);
        alloc_count::disable();

        // Arena bookkeeping floor: fixed probe batch, u64 payloads.
        let probe_count = 200_000u64;
        alloc_count::enable();
        let mut probe: Arena<u64> = Arena::with_capacity(probe_count as usize);
        for i in 0..probe_count {
            probe.insert(i);
        }
        let probe_snap = alloc_count::snapshot();
        drop(probe);
        alloc_count::disable();

        println!(
            "== {} | records {}",
            path,
            lines.len()
        );
        println!(
            "  Vec<LogRecord>   live {} | {:.1} B/rec | {:.1} allocs/rec",
            human(vec_snap.live),
            vec_snap.live as f64 / n as f64,
            vec_snap.allocs as f64 / n as f64,
        );
        println!(
            "  Arena<LogRecord> live {} | {:.1} B/rec | {:.1} allocs/rec | bookkeeping {:.1} B/rec",
            human(arena_snap.live),
            arena_snap.live as f64 / n as f64,
            arena_snap.allocs as f64 / n as f64,
            (arena_snap.live - vec_snap.live) as f64 / n as f64,
        );
        println!(
            "  Arena<u64> probe node cost {:.1} B (live {} over {} inserts | size_of u64 {} B)",
            probe_snap.live as f64 / probe_count as f64,
            human(probe_snap.live),
            probe_count,
            size_of::<u64>(),
        );
    }
    Ok(())
}
