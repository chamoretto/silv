//! Memory micro-benchmark of compact string and vector types against std.
//!
//! Measures static size, heap bytes and allocation counts for string types
//! (String, compact_str, smartstring, smol_str, smallstr, cold_string,
//! hipstr) at realistic log field lengths, vector-of-string layouts for tag
//! arrays (Vec/SmallVec/TinyVec), and a combined `CompactRecord` mirror of
//! silv's record layer (entry vector + compact strings + boxed smallvecs,
//! parsed from the corpus JSON) to show the joint lever.
//!
//! Build with `--features mem-bench` only; never part of a release build.
// Measurement mirrors: fields are kept alive on purpose, reads are not needed.
#![allow(dead_code)]

use std::error::Error;
use std::mem::size_of;
use std::path::Path;

use cold_string::ColdString;
use compact_str::CompactString;
use hipstr::HipStr;
use serde_json::{Map, Value};
use silv::alloc_count;
use silv::alloc_count::Counting;
use silv::parse;
use smallstr::SmallString;
use smallvec::SmallVec;
use smartstring::{LazyCompact, SmartString};
use smol_str::SmolStr;
use tinyvec::TinyVec;

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

/// Measure `live` heap delta of a batch: build `count` values, snapshot, drop.
fn measure<T>(count: u64, build: impl Fn(u64) -> Vec<T>) -> (u64, u64, u64) {
    alloc_count::enable();
    let values = build(count);
    let mid = alloc_count::snapshot();
    let live = mid.live;
    let allocs = mid.allocs;
    let total = mid.total;
    drop(values);
    alloc_count::disable();
    (live, allocs, total)
}

fn sample(len: usize, index: u64) -> String {
    let mut s = String::with_capacity(len);
    for i in 0..len {
        s.push((b'a' + ((index + i as u64) % 26) as u8) as char);
    }
    s
}

fn string_table(count: usize, len: usize) -> Vec<String> {
    (0..count).map(|i| sample(len, i as u64 * 7)).collect()
}

fn bench_strings(count: u64) {
    println!("== strings: {count} unique copies per row, heap bytes per string (live)");
    println!("{:<24} {:>5} {:>4} allocs/str", "type", "size", "B/str");
    for len in [4usize, 8, 12, 15, 16, 24, 25, 32, 64, 128] {
        println!("-- string length {len}");
        let table = string_table(count as usize, len);

        let rows: Vec<(&str, usize, (u64, u64, u64))> = vec![
            (
                "String",
                size_of::<String>(),
                measure(count, |_| table.iter().map(String::as_str).map(str::to_owned).collect()),
            ),
            (
                "compact_str",
                size_of::<CompactString>(),
                measure(count, |_| table.iter().map(|s| CompactString::new(s)).collect()),
            ),
            (
                "smartstring",
                size_of::<SmartString<LazyCompact>>(),
                measure(count, |_| {
                    table
                        .iter()
                        .map(|s| SmartString::<LazyCompact>::from(s.as_str()))
                        .collect()
                }),
            ),
            ("smol_str", size_of::<SmolStr>(), measure(count, |_| table.iter().map(SmolStr::new).collect())),
            (
                "SmallString<[u8;32]>",
                size_of::<SmallString<[u8; 32]>>(),
                measure(count, |_| {
                    table
                        .iter()
                        .map(|s| SmallString::<[u8; 32]>::from(s.as_str()))
                        .collect()
                }),
            ),
            (
                "cold_string",
                size_of::<ColdString>(),
                measure(count, |_| table.iter().map(|s| ColdString::from(s.as_str())).collect()),
            ),
            (
                "hipstr",
                size_of::<HipStr<'static>>(),
                measure(count, |_| table.iter().map(|s| HipStr::from(s.as_str())).collect()),
            ),
        ];
        for (name, size, (live, allocs, _)) in rows {
            println!(
                "{:<24} {:>5} {:>4} {:.2}",
                name,
                size,
                (live as f64 / count as f64) - size as f64,
                allocs as f64 / count as f64,
            );
        }
    }
}

fn bench_tags(count: u64) {
    // Tag-array model of the mixed corpus: 0..4 short strings (6-9 chars),
    // average 2.5 tags per record.
    println!();
    println!("== tag arrays: {count} arrays, live bytes per array (static slot included)");
    let words = [
        "apm", "auth", "cache", "db", "edge", "http", "kafka", "queue", "redis", "shard", "tls", "worker",
    ];
    let tag_sets: Vec<Vec<String>> = (0..count)
        .map(|i| {
            let n = (i % 5) as usize;
            (0..n)
                .map(|k| words[(i as usize + k * 5) % words.len()].to_owned())
                .collect()
        })
        .collect();
    let rows: Vec<(&str, usize, (u64, u64, u64))> = vec![
        ("Vec<String>", size_of::<Vec<String>>(), measure(count, |_| tag_sets.to_vec())),
        (
            "SmallVec<[String;4]>",
            size_of::<SmallVec<[String; 4]>>(),
            measure(count, |_| {
                tag_sets
                    .iter()
                    .map(|s| s.iter().cloned().collect::<SmallVec<[String; 4]>>())
                    .collect()
            }),
        ),
        (
            "TinyVec<[String;4]>",
            size_of::<TinyVec<[String; 4]>>(),
            measure(count, |_| {
                tag_sets
                    .iter()
                    .map(|s| s.iter().cloned().collect::<TinyVec<[String; 4]>>())
                    .collect()
            }),
        ),
        (
            "Vec<CompactString>",
            size_of::<Vec<CompactString>>(),
            measure(count, |_| {
                tag_sets
                    .iter()
                    .map(|s| s.iter().map(|w| CompactString::new(w)).collect::<Vec<_>>())
                    .collect()
            }),
        ),
        (
            "SmallVec<[CompactString;4]>",
            size_of::<SmallVec<[CompactString; 4]>>(),
            measure(count, |_| {
                tag_sets
                    .iter()
                    .map(|s| {
                        s.iter()
                            .map(|w| CompactString::new(w))
                            .collect::<SmallVec<[CompactString; 4]>>()
                    })
                    .collect()
            }),
        ),
        (
            "TinyVec<[CompactString;4]>",
            size_of::<TinyVec<[CompactString; 4]>>(),
            measure(count, |_| {
                tag_sets
                    .iter()
                    .map(|s| {
                        s.iter()
                            .map(|w| CompactString::new(w))
                            .collect::<TinyVec<[CompactString; 4]>>()
                    })
                    .collect()
            }),
        ),
    ];
    println!("{:<30} {:>5} {:>7} {:>11}", "type", "size", "B/array", "allocs/array");
    for (name, size, (live, allocs, _)) in rows {
        println!("{:<30} {:>5} {:>7.1} {:>11.2}", name, size, live as f64 / count as f64, allocs as f64 / count as f64,);
    }
}

// Combined lever: compact record mirror parsed from real corpus JSON.

#[derive(Debug)]
enum MiniValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(CompactString),
    Arr(Box<SmallVec<[MiniValue; 4]>>),
    Obj(Box<SmallVec<[(CompactString, MiniValue); 4]>>),
}

#[derive(Debug)]
struct MiniRecord {
    source: u32,
    line: u64,
    offset: usize,
    message: Option<CompactString>,
    fields: SmallVec<[(CompactString, MiniValue); 4]>,
}

fn mini_of(value: Value) -> MiniValue {
    match value {
        Value::Null => MiniValue::Null,
        Value::Bool(b) => MiniValue::Bool(b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                MiniValue::Int(i)
            } else {
                MiniValue::Float(n.as_f64().unwrap_or(0.0))
            }
        },
        Value::String(s) => MiniValue::Str(CompactString::from(s)),
        Value::Array(items) => MiniValue::Arr(Box::new(items.into_iter().map(mini_of).collect())),
        Value::Object(map) => {
            let mut fields: SmallVec<[(CompactString, MiniValue); 4]> = SmallVec::with_capacity(map.len());
            for (k, v) in map {
                fields.push((CompactString::from(k), mini_of(v)));
            }
            MiniValue::Obj(Box::new(fields))
        },
    }
}

fn mini_record(source: u32, line: u64, offset: usize, map: Map<String, Value>) -> MiniRecord {
    let message = map.get("message").and_then(|v| v.as_str()).map(CompactString::from);
    MiniRecord {
        source,
        line,
        offset,
        message,
        fields: mini_obj_fields(map),
    }
}

fn mini_obj_fields(map: Map<String, Value>) -> SmallVec<[(CompactString, MiniValue); 4]> {
    let mut fields = SmallVec::with_capacity(map.len());
    for (k, v) in map {
        fields.push((CompactString::from(k), mini_of(v)));
    }
    fields
}

fn bench_records(files: &[String]) -> Result<(), Box<dyn Error>> {
    println!();
    println!("== compact record mirror (entry SmallVec + CompactString + boxed smallvec arrays)");
    for path in files {
        let bytes = std::fs::read(Path::new(path))?;
        let lines = parse::split_lines(&bytes);

        alloc_count::enable();
        let mini: Vec<MiniRecord> = lines
            .iter()
            .enumerate()
            .filter_map(|(i, (number, offset, body))| {
                let map = serde_json::from_slice::<Map<String, Value>>(body).ok()?;
                Some(mini_record(i as u32, *number, *offset, map))
            })
            .collect();
        let mid = alloc_count::snapshot();
        drop(mini);
        alloc_count::disable();

        let n = lines.len().max(1) as u64;
        println!(
            "{:<40} live {} | {:.1} B/rec | {:.1} allocs/rec | static MiniRecord {} B, MiniValue {} B, field pair {} B",
            path,
            human(mid.live),
            mid.live as f64 / n as f64,
            mid.allocs as f64 / n as f64,
            size_of::<MiniRecord>(),
            size_of::<MiniValue>(),
            size_of::<(CompactString, MiniValue)>(),
        );
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let files: Vec<String> = std::env::args().skip(1).collect();
    bench_strings(20_000);
    bench_tags(40_000);
    bench_records(&files)?;
    Ok(())
}
