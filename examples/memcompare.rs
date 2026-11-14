//! Heap-memory comparison of JSON DOM implementations over silv benchmark files.
//!
//! For each input JSON Lines file, parses every line into:
//! - `serde_json::Map<String, Value>` (silv's current parsed layer, baseline)
//! - `ijson::IValue` (pointer-sized tagged DOM, interned strings)
//! - `sonic_rs::Value` (bump-arena DOM, objects as entry vectors)
//! - `datavalue_rs::DataValue` (bump-arena, zero-copy strings borrowing the input)
//! - nanoserde typed derive (`RecordMixed`, mixed-shape files only): the floor
//!   of a flat typed representation, and `DeJsonState` (streaming, no DOM)
//! - silv's own `LogRecord` row for the internal layer reference
//!
//! Heap is measured with the counting global allocator: live bytes while the
//! DOM is alive, peak, allocation count, live bytes after drop (residue such as
//! interning caches). Build with `--features mem-bench` only; never part of a
//! release build of silv.

use std::error::Error;
use std::fs;
use std::path::Path;

use silv::alloc_count;
use silv::alloc_count::Counting;
use silv::model::record::FieldAliases;
use silv::model::record::LogRecord;
use silv::parse;
use silv::parse::jsonl;

use serde_json::{Map, Value};

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Structural node counts shared by every DOM walker.
#[derive(Default, Clone, Copy, Debug)]
struct Counts {
    maps: u64,
    entries: u64,
    arrays: u64,
    elems: u64,
    strings: u64,
    str_bytes: u64,
    numbers: u64,
    bools: u64,
    nulls: u64,
}

impl Counts {
    fn add_entry(&mut self, key_len: usize) {
        self.entries += 1;
        self.str_bytes += key_len as u64;
    }
}

// The measured run of one implementation: heap snapshots plus structure.
struct Run {
    name: &'static str,
    live: u64,
    peak: u64,
    allocs: u64,
    total: u64,
    residue: u64,
    extra: Option<(String, u64)>,
    counts: Option<Counts>,
    failed: Option<String>,
}

impl Run {
    fn ok(name: &'static str, counts: Option<Counts>) -> Self {
        let mid = alloc_count::snapshot();
        Self {
            name,
            live: mid.live,
            peak: mid.peak_live,
            allocs: mid.allocs,
            total: mid.total,
            residue: 0,
            extra: None,
            counts,
            failed: None,
        }
    }

    fn residue(mut self) -> Self {
        self.residue = alloc_count::snapshot().live;
        self
    }

    fn extra(mut self, label: &str, bytes: u64) -> Self {
        self.extra = Some((label.to_owned(), bytes));
        self
    }

    fn failed(name: &'static str, err: impl std::fmt::Debug) -> Self {
        alloc_count::disable();
        Self {
            name,
            live: 0,
            peak: 0,
            allocs: 0,
            total: 0,
            residue: 0,
            extra: None,
            counts: None,
            failed: Some(format!("{err:?}")),
        }
    }
}

// serde_json baseline

fn walk_serde(value: &Value, c: &mut Counts) {
    match value {
        Value::Object(map) => {
            c.maps += 1;
            for (k, v) in map {
                c.add_entry(k.len());
                walk_serde(v, c);
            }
        }
        Value::Array(items) => {
            c.arrays += 1;
            c.elems += items.len() as u64;
            for v in items {
                walk_serde(v, c);
            }
        }
        Value::String(s) => {
            c.strings += 1;
            c.str_bytes += s.len() as u64;
        }
        Value::Number(_) => c.numbers += 1,
        Value::Bool(_) => c.bools += 1,
        Value::Null => c.nulls += 1,
    }
}

fn run_serde_json(lines: &[(u64, usize, &[u8])]) -> Run {
    alloc_count::enable();
    let mut dom: Vec<Map<String, Value>> = Vec::with_capacity(lines.len());
    let mut c = Counts::default();
    let mut first_err: Option<String> = None;
    for (_, _, body) in lines {
        match serde_json::from_str::<Map<String, Value>>(str_of(body)) {
            Ok(map) => {
                for (k, v) in &map {
                    c.add_entry(k.len());
                    walk_serde(v, &mut c);
                }
                dom.push(map);
            }
            Err(e) => {
                first_err.get_or_insert_with(|| e.to_string());
            }
        }
    }
    c.maps += dom.len() as u64;
    if let Some(err) = first_err {
        return Run::failed("serde_json Map", err);
    }
    let mut run = Run::ok("serde_json Map", Some(c));
    drop(dom);
    run = run.residue();
    alloc_count::disable();
    run
}

// ijson

fn walk_ijson(value: &ijson::IValue, c: &mut Counts) {
    match value.type_() {
        ijson::ValueType::Object => {
            c.maps += 1;
            if let Some(obj) = value.as_object() {
                for (k, v) in obj.iter() {
                    c.add_entry(k.len());
                    walk_ijson(v, c);
                }
            }
        }
        ijson::ValueType::Array => {
            c.arrays += 1;
            if let Some(arr) = value.as_array() {
                c.elems += arr.len() as u64;
                for v in arr.iter() {
                    walk_ijson(v, c);
                }
            }
        }
        ijson::ValueType::String => {
            c.strings += 1;
            if let Some(s) = value.as_string() {
                c.str_bytes += s.len() as u64;
            }
        }
        ijson::ValueType::Number => c.numbers += 1,
        ijson::ValueType::Bool => c.bools += 1,
        ijson::ValueType::Null => c.nulls += 1,
    }
}

fn run_ijson(lines: &[(u64, usize, &[u8])]) -> Run {
    alloc_count::enable();
    let mut dom: Vec<ijson::IValue> = Vec::with_capacity(lines.len());
    let mut c = Counts::default();
    let mut first_err: Option<String> = None;
    for (_, _, body) in lines {
        match serde_json::from_str::<ijson::IValue>(str_of(body)) {
            Ok(v) => {
                walk_ijson(&v, &mut c);
                dom.push(v);
            }
            Err(e) => {
                first_err.get_or_insert_with(|| e.to_string());
            }
        }
    }
    c.maps += dom.len() as u64;
    if let Some(err) = first_err {
        return Run::failed("ijson", err);
    }
    let mut run = Run::ok("ijson IValue", Some(c));
    drop(dom);
    run = run.residue();
    alloc_count::disable();
    run
}

// sonic-rs

fn walk_sonic(value: &sonic_rs::Value, c: &mut Counts) {
    use sonic_rs::{JsonContainerTrait, JsonValueTrait};
    if value.is_object() {
        c.maps += 1;
        if let Some(obj) = value.as_object() {
            for (k, v) in obj.iter() {
                c.add_entry(k.len());
                walk_sonic(v, c);
            }
        }
    } else if value.is_array() {
        c.arrays += 1;
        if let Some(arr) = value.as_array() {
            c.elems += arr.len() as u64;
            for v in arr.iter() {
                walk_sonic(v, c);
            }
        }
    } else if value.is_str() {
        c.strings += 1;
        if let Some(s) = value.as_str() {
            c.str_bytes += s.len() as u64;
        }
    } else if value.is_number() {
        c.numbers += 1;
    } else if value.is_boolean() {
        c.bools += 1;
    } else {
        c.nulls += 1;
    }
}

fn run_sonic(lines: &[(u64, usize, &[u8])]) -> Run {
    alloc_count::enable();
    let mut dom: Vec<sonic_rs::Value> = Vec::with_capacity(lines.len());
    let mut c = Counts::default();
    let mut first_err: Option<String> = None;
    for (_, _, body) in lines {
        match sonic_rs::from_str::<sonic_rs::Value>(str_of(body)) {
            Ok(v) => {
                walk_sonic(&v, &mut c);
                dom.push(v);
            }
            Err(e) => {
                first_err.get_or_insert_with(|| e.to_string());
            }
        }
    }
    c.maps += dom.len() as u64;
    if let Some(err) = first_err {
        return Run::failed("sonic-rs", err);
    }
    let mut run = Run::ok("sonic-rs Value", Some(c));
    drop(dom);
    run = run.residue();
    alloc_count::disable();
    run
}

// datavalue-rs

fn walk_datavalue(value: &datavalue_rs::DataValue<'_>, c: &mut Counts) {
    use datavalue_rs::DataValue;
    match value {
        DataValue::Object(entries) => {
            c.maps += 1;
            for (k, v) in *entries {
                c.add_entry(k.len());
                walk_datavalue(v, c);
            }
        }
        DataValue::Array(items) => {
            c.arrays += 1;
            c.elems += items.len() as u64;
            for v in items.iter() {
                walk_datavalue(v, c);
            }
        }
        DataValue::String(s) => {
            c.strings += 1;
            c.str_bytes += s.len() as u64;
        }
        DataValue::Number(_) => c.numbers += 1,
        DataValue::Bool(_) => c.bools += 1,
        _ => c.nulls += 1,
    }
}

fn run_datavalue(lines: &[(u64, usize, &[u8])]) -> Run {
    alloc_count::enable();
    let arena = bumpalo::Bump::new();
    let mut dom: Vec<datavalue_rs::DataValue<'_>> = Vec::with_capacity(lines.len());
    let mut c = Counts::default();
    let mut first_err: Option<String> = None;
    for (_, _, body) in lines {
        match datavalue_rs::DataValue::from_str(str_of(body), &arena) {
            Ok(v) => {
                walk_datavalue(&v, &mut c);
                dom.push(v);
            }
            Err(e) => {
                first_err.get_or_insert_with(|| format!("{e:?}"));
            }
        }
    }
    c.maps += dom.len() as u64;
    if let Some(err) = first_err {
        return Run::failed("datavalue-rs", err);
    }
    let arena_bytes = arena.allocated_bytes() as u64;
    let mut run = Run::ok("datavalue DataValue", Some(c));
    drop(dom);
    drop(arena);
    run = run.residue();
    alloc_count::disable();
    run.extra("arena reserved", arena_bytes)
}

// nanoserde: typed derive floor for the mixed shape

mod nano {
    #![allow(dead_code)] // measurement structs: fields exist to be parsed, not read
    use nanoserde::DeJson;

    #[derive(DeJson, Default)]
    pub struct User {
        pub id: u64,
        pub name: String,
        pub email: String,
    }

    #[derive(DeJson, Default)]
    pub struct Http {
        pub method: String,
        pub path: String,
        pub status: u64,
    }

    #[derive(DeJson, Default)]
    pub struct K8s {
        pub pod: String,
        pub namespace: String,
        pub container: String,
    }

    #[derive(DeJson, Default)]
    pub struct Ctx {
        pub region: String,
        pub shard: i64,
    }

    #[derive(DeJson, Default)]
    pub struct RecordMixed {
        #[nserde(rename = "@timestamp")]
        pub timestamp: String,
        pub level: String,
        pub logger: String,
        pub service: String,
        pub host: String,
        pub pid: u64,
        pub message: String,
        pub trace_id: String,
        pub span_id: String,
        pub duration_ms: f64,
        pub status: u64,
        pub bytes_sent: u64,
        pub ok: bool,
        pub user: User,
        pub http: Http,
        #[nserde(default)]
        pub k8s: Option<K8s>,
        pub tags: Vec<String>,
        #[nserde(default)]
        pub error: Option<String>,
        #[nserde(default)]
        pub sample_rate: f64,
        #[nserde(default)]
        pub context: Option<Ctx>,
    }
}

fn run_nanoserde(lines: &[(u64, usize, &[u8])]) -> Run {
    use nanoserde::DeJson;
    alloc_count::enable();
    let mut dom: Vec<nano::RecordMixed> = Vec::with_capacity(lines.len());
    let mut first_err: Option<String> = None;
    for (_, _, body) in lines {
        match nano::RecordMixed::deserialize_json(str_of(body)) {
            Ok(r) => dom.push(r),
            Err(e) => {
                first_err.get_or_insert_with(|| format!("line {}: {:?}", e.line, e.msg));
            }
        }
    }
    if let Some(err) = first_err {
        return Run::failed("nanoserde typed", err);
    }
    let mut run = Run::ok("nanoserde typed", None);
    drop(dom);
    run = run.residue();
    alloc_count::disable();
    run.extra(
        "struct size",
        std::mem::size_of::<nano::RecordMixed>() as u64,
    )
}

// silv internal layer (reference)

fn run_silv_records(lines: &[(u64, usize, &[u8])]) -> Run {
    let aliases = FieldAliases::default();
    alloc_count::enable();
    let mut dom: Vec<LogRecord> = Vec::with_capacity(lines.len());
    for (index, (number, offset, body)) in lines.iter().enumerate() {
        if let Ok(record) = jsonl::parse_line(index as u32, *number, *offset, *body, &aliases) {
            dom.push(record);
        }
    }
    let mut run = Run::ok("silv LogRecord", None);
    drop(dom);
    run = run.residue();
    alloc_count::disable();
    run.extra("record size", std::mem::size_of::<LogRecord>() as u64)
}

fn str_of(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap_or("")
}

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

fn main() -> Result<(), Box<dyn Error>> {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        return Err("usage: memcompare <file.jsonl>...".into());
    }

    println!(
        "static sizes (bytes): serde_json::Value={} ijson::IValue={} sonic_rs::Value={} datavalue DataValue={} DeJsonState={} LogRecord={}",
        std::mem::size_of::<Value>(),
        std::mem::size_of::<ijson::IValue>(),
        std::mem::size_of::<sonic_rs::Value>(),
        std::mem::size_of::<datavalue_rs::DataValue<'_>>(),
        std::mem::size_of::<nanoserde::DeJsonState>(),
        std::mem::size_of::<LogRecord>(),
    );

    for path in files {
        let path = Path::new(&path);
        let bytes = fs::read(path)?;
        let lines = parse::split_lines(&bytes);
        println!();
        println!(
            "== {} | input {} | lines {}",
            path.display(),
            human(bytes.len() as u64),
            lines.len()
        );

        let runs = [
            run_serde_json(&lines),
            run_ijson(&lines),
            run_sonic(&lines),
            run_datavalue(&lines),
            run_nanoserde(&lines),
            run_silv_records(&lines),
        ];

        let n = lines.len().max(1) as f64;
        println!(
            "{:<20} {:>10} {:>10} {:>11} {:>8} {:>10} {:>10} {:>10}  {:<}",
            "impl", "live", "peak", "cumulative", "live/rec", "allocs", "allocs/rec", "residue", "extra"
        );
        for run in &runs {
            if let Some(err) = &run.failed {
                println!("{:<20} FAILED: {}", run.name, err);
                continue;
            }
            let extra = run
                .extra
                .as_ref()
                .map(|(l, b)| format!("{l} {}", human(*b)))
                .unwrap_or_default();
            println!(
                "{:<20} {:>10} {:>10} {:>11} {:>8.1} {:>10} {:>10.1} {:>10}  {}",
                run.name,
                human(run.live),
                human(run.peak),
                human(run.total),
                run.live as f64 / n,
                run.allocs,
                run.allocs as f64 / n,
                human(run.residue),
                extra,
            );
        }

        // structural sanity: shared node counts across DOM walkers
        let mut base: Option<(&str, Counts)> = None;
        for run in &runs {
            let Some(c) = &run.counts else { continue };
            match base {
                None => base = Some((run.name, *c)),
                Some((_, b)) => {
                    if b.entries != c.entries || b.strings != c.strings || b.numbers != c.numbers {
                        println!(
                            "NOTE {}: entries {} vs {}, strings {} vs {}, numbers {} vs {}",
                            run.name,
                            c.entries,
                            b.entries,
                            c.strings,
                            b.strings,
                            c.numbers,
                            b.numbers
                        );
                    }
                }
            }
        }
        if let Some((_, c)) = base {
            println!(
                "structure: maps={} entries={} arrays={} elems={} strings={} string_bytes={} numbers={} bools={} nulls={}",
                c.maps,
                c.entries,
                c.arrays,
                c.elems,
                c.strings,
                c.str_bytes,
                c.numbers,
                c.bools,
                c.nulls
            );
        }
    }
    Ok(())
}
