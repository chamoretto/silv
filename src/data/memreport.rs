//! `--memory-report`: measured heap accounting for the file, parsed JSON and
//! internal record layers, with per-kind breakdowns and optimization hints.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::Write;
use std::mem::size_of;

use ijson::IObject;
use ijson::IString;
use ijson::IValue;
use miette::IntoDiagnostic;
use rayon::prelude::*;

use crate::alloc_count;
use crate::cli::Config;
use crate::cli::FormatSelection;
use crate::cli::ReportLayer;
use crate::error::SilvError;
use crate::model::level::LogLevel;
use crate::model::record::LogRecord;
use crate::model::record::UNPARSED_NANOS;
use crate::parse::LineResult;
use crate::parse::LogFormat;
use crate::parse::input::InputBundle;
use crate::parse::input::MMAP_THRESHOLD;
use crate::parse::input::read_inputs;
use crate::parse::jsonl;
use crate::parse::split_lines;

/// Heap vector header behind every ijson object.
const OBJECT_BOX: u64 = size_of::<Vec<(IString, IValue)>>() as u64;
/// Heap vector header behind every ijson array.
const ARRAY_BOX: u64 = size_of::<Vec<IValue>>() as u64;
/// One `(key, value)` slot in an object's entries vector.
const ENTRY_SLOT: u64 = size_of::<(IString, IValue)>() as u64;
/// One element slot in an array's vector, and one slot in the parsed layer's vector.
const VALUE_SLOT: u64 = size_of::<IValue>() as u64;
/// One record slot in the internal layer's vector.
const RECORD_SLOT: u64 = size_of::<LogRecord>() as u64;
/// One `(number, offset, bytes)` entry of the per-source line index.
const LINE_INDEX_SLOT: u64 = size_of::<(u64, usize, &[u8])>() as u64;

/// Node counts and heap bytes of a JSON value forest, measured by walking.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsonWalk {
    pub maps: u64,
    pub map_len_sum: u64,
    pub map_max_width: u32,
    pub entries: u64,
    pub keys_len: u64,
    pub keys_cap: u64,
    pub strings: u64,
    pub strings_len: u64,
    pub strings_cap: u64,
    pub ints: u64,
    pub uints: u64,
    pub floats: u64,
    pub bools: u64,
    pub nulls: u64,
    pub arrays: u64,
    pub array_elems: u64,
    pub array_cap: u64,
    pub depth_sum: u64,
    pub depth_max: u32,
}

impl JsonWalk {
    pub fn merge(&mut self, other: &Self) {
        self.maps += other.maps;
        self.map_len_sum += other.map_len_sum;
        self.map_max_width = self.map_max_width.max(other.map_max_width);
        self.entries += other.entries;
        self.keys_len += other.keys_len;
        self.keys_cap += other.keys_cap;
        self.strings += other.strings;
        self.strings_len += other.strings_len;
        self.strings_cap += other.strings_cap;
        self.ints += other.ints;
        self.uints += other.uints;
        self.floats += other.floats;
        self.bools += other.bools;
        self.nulls += other.nulls;
        self.arrays += other.arrays;
        self.array_elems += other.array_elems;
        self.array_cap += other.array_cap;
        self.depth_sum += other.depth_sum;
        self.depth_max = self.depth_max.max(other.depth_max);
    }

    /// Heap bytes attributed to the forest: object and array vector boxes,
    /// entry and element slots, plus key and string bytes counted per
    /// occurrence. ijson interns strings globally, so key and string heap is
    /// shared across records: the counting allocator measures the shared cost
    /// once, and the gap shows up as the dedup win in the report.
    pub fn heap_est(&self) -> u64 {
        self.maps * OBJECT_BOX
            + self.entries * ENTRY_SLOT
            + self.arrays * ARRAY_BOX
            + self.array_cap * VALUE_SLOT
            + self.keys_cap
            + self.strings_cap
    }

    pub fn nodes(&self) -> u64 {
        self.maps + self.strings + self.ints + self.uints + self.floats + self.bools + self.nulls + self.arrays
    }

    pub fn avg_map_width(&self) -> f64 {
        if self.maps == 0 {
            0.0
        } else {
            self.map_len_sum as f64 / self.maps as f64
        }
    }

    pub fn avg_depth(&self) -> f64 {
        if self.maps == 0 {
            0.0
        } else {
            self.depth_sum as f64 / self.maps as f64
        }
    }
}

/// Builds the walk of one JSON value (public API for tools and tests).
pub fn walk_of(value: &IValue) -> JsonWalk {
    let mut walk = JsonWalk::default();
    add_root(&mut walk, value, 0);
    walk
}

/// Builds the walk of one internal record (public API for tools and tests).
pub fn walk_of_record(record: &LogRecord) -> RecordWalk {
    let mut walk = RecordWalk::default();
    add_record(&mut walk, record);
    walk
}

fn add_root(walk: &mut JsonWalk, value: &IValue, depth: u32) {
    if let Some(map) = value.as_object() {
        add_map(walk, map, depth);
    } else {
        add_value(walk, value, depth);
    }
}

fn add_map(walk: &mut JsonWalk, map: &IObject, depth: u32) {
    walk.maps += 1;
    let len = map.len() as u64;
    walk.map_len_sum += len;
    walk.entries += len;
    walk.map_max_width = walk.map_max_width.max(len as u32);
    walk.depth_sum += depth as u64;
    walk.depth_max = walk.depth_max.max(depth);
    for (key, value) in map.iter() {
        walk.keys_len += key.len() as u64;
        walk.keys_cap += key.len() as u64;
        add_value(walk, value, depth + 1);
    }
}

fn add_value(walk: &mut JsonWalk, value: &IValue, depth: u32) {
    if value.is_null() {
        walk.nulls += 1;
        return;
    }
    if value.is_bool() {
        walk.bools += 1;
        return;
    }
    if let Some(number) = value.as_number() {
        if number.to_i64().is_some() {
            walk.ints += 1;
        } else if number.to_u64().is_some() {
            walk.uints += 1;
        } else {
            walk.floats += 1;
        }
        return;
    }
    if let Some(text) = value.as_string() {
        walk.strings += 1;
        walk.strings_len += text.len() as u64;
        walk.strings_cap += text.len() as u64;
        return;
    }
    if let Some(items) = value.as_array() {
        walk.arrays += 1;
        walk.array_elems += items.len() as u64;
        walk.array_cap += items.len() as u64;
        for item in items.iter() {
            add_value(walk, item, depth + 1);
        }
        return;
    }
    if let Some(map) = value.as_object() {
        add_map(walk, map, depth);
    }
}

/// What the internal layer owns on top of the stored JSON object: the
/// extracted timestamp, severity and message columns, and raw line text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecordWalk {
    pub records: u64,
    pub json: JsonWalk,
    pub messages: u64,
    pub messages_len: u64,
    pub messages_cap: u64,
    pub stamps: u64,
    pub stamps_raw_len: u64,
    pub stamps_unparsed: u64,
    pub levels_known: u64,
    pub levels_custom: u64,
    pub levels_custom_cap: u64,
    pub invalids: u64,
    pub invalids_raw_len: u64,
    pub raw_lines: u64,
    pub raw_lines_len: u64,
}

impl RecordWalk {
    pub fn merge(&mut self, other: &Self) {
        self.records += other.records;
        self.json.merge(&other.json);
        self.messages += other.messages;
        self.messages_len += other.messages_len;
        self.messages_cap += other.messages_cap;
        self.stamps += other.stamps;
        self.stamps_raw_len += other.stamps_raw_len;
        self.stamps_unparsed += other.stamps_unparsed;
        self.levels_known += other.levels_known;
        self.levels_custom += other.levels_custom;
        self.levels_custom_cap += other.levels_custom_cap;
        self.invalids += other.invalids;
        self.invalids_raw_len += other.invalids_raw_len;
        self.raw_lines += other.raw_lines;
        self.raw_lines_len += other.raw_lines_len;
    }

    /// Heap owned outside the stored object: alias column text and raw plain
    /// lines. Invalid records keep their line text inside the object payload,
    /// counted in the json walk.
    pub fn extras_heap(&self) -> u64 {
        self.messages_cap + self.stamps_raw_len + self.levels_custom_cap + self.raw_lines_len
    }
}

fn add_record(walk: &mut RecordWalk, record: &LogRecord) {
    walk.records += 1;
    add_root(&mut walk.json, &record.object, 0);
    if let Some(message) = &record.message {
        walk.messages += 1;
        walk.messages_len += message.len() as u64;
        walk.messages_cap += message.capacity() as u64;
    }
    if let Some(stamp) = &record.timestamp {
        walk.stamps += 1;
        walk.stamps_raw_len += stamp.raw().len() as u64;
        if stamp.nanos() == UNPARSED_NANOS {
            walk.stamps_unparsed += 1;
        }
    }
    match &record.level {
        Some(LogLevel::Known(_)) => walk.levels_known += 1,
        Some(LogLevel::Custom(text)) => {
            walk.levels_custom += 1;
            walk.levels_custom_cap += text.capacity() as u64;
        },
        None => {},
    }
    if let Some(raw) = &record.raw {
        walk.raw_lines += 1;
        walk.raw_lines_len += raw.len() as u64;
    }
    if record.is_invalid() {
        walk.invalids += 1;
        walk.invalids_raw_len += record.raw_text().len() as u64;
    }
}

/// Repetition analysis: bytes spent on field names and string values that
/// repeat across records, with the unique-bytes floor for interning.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dedup {
    pub keys_total: u64,
    pub keys_bytes: u64,
    pub keys_unique: u64,
    pub keys_unique_bytes: u64,
    pub strings_total: u64,
    pub strings_bytes: u64,
    pub strings_unique: u64,
    pub strings_unique_bytes: u64,
}

impl Dedup {
    fn merge(&mut self, other: Self) {
        self.keys_total += other.keys_total;
        self.keys_bytes += other.keys_bytes;
        self.keys_unique = self.keys_unique.max(other.keys_unique);
        self.keys_unique_bytes = self.keys_unique_bytes.max(other.keys_unique_bytes);
        self.strings_total += other.strings_total;
        self.strings_bytes += other.strings_bytes;
        self.strings_unique = self.strings_unique.max(other.strings_unique);
        self.strings_unique_bytes = self.strings_unique_bytes.max(other.strings_unique_bytes);
    }
}

fn dedup_maps<'a>(roots: impl IntoIterator<Item = &'a IValue>) -> Dedup {
    let mut dedup = Dedup::default();
    let mut keys: HashSet<&str> = HashSet::new();
    let mut strings: HashSet<&str> = HashSet::new();
    for root in roots {
        let Some(map) = root.as_object() else {
            continue;
        };
        for (key, value) in map.iter() {
            dedup.keys_total += 1;
            dedup.keys_bytes += key.len() as u64;
            if keys.insert(key.as_str()) {
                dedup.keys_unique += 1;
                dedup.keys_unique_bytes += key.len() as u64;
            }
            collect_strings(value, &mut dedup, &mut strings);
        }
    }
    dedup
}

fn collect_strings<'a>(value: &'a IValue, dedup: &mut Dedup, strings: &mut HashSet<&'a str>) {
    if let Some(text) = value.as_string() {
        dedup.strings_total += 1;
        dedup.strings_bytes += text.len() as u64;
        if strings.insert(text.as_str()) {
            dedup.strings_unique += 1;
            dedup.strings_unique_bytes += text.len() as u64;
        }
        return;
    }
    if let Some(items) = value.as_array() {
        for item in items.iter() {
            collect_strings(item, dedup, strings);
        }
        return;
    }
    if let Some(map) = value.as_object() {
        for value in map.values() {
            collect_strings(value, dedup, strings);
        }
    }
}

/// A measured heap stage of the report run.
#[derive(Debug)]
pub struct Checkpoint {
    pub stage: &'static str,
    pub live: u64,
    pub allocs: u64,
    pub rss: Option<u64>,
}

fn checkpoint(stage: &'static str) -> Checkpoint {
    let stats = alloc_count::snapshot();
    Checkpoint {
        stage,
        live: stats.live,
        allocs: stats.allocs,
        rss: process_rss(),
    }
}

/// Current process resident memory, `None` on platforms without a probe.
pub fn process_rss() -> Option<u64> {
    process_counters().map(|(_, working, _)| working)
}

/// Current process peak resident memory, `None` where not supported.
pub fn process_peak_rss() -> Option<u64> {
    process_counters().map(|(_, _, peak)| peak)
}

#[cfg(windows)]
fn process_counters() -> Option<(u64, u64, u64)> {
    use std::ffi::c_void;
    use std::mem::size_of;
    use windows_sys::Win32::System::ProcessStatus::GetProcessMemoryInfo;
    use windows_sys::Win32::System::ProcessStatus::PROCESS_MEMORY_COUNTERS;
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let ok = unsafe {
        let process: *mut c_void = GetCurrentProcess();
        GetProcessMemoryInfo(process, &mut counters, size_of::<PROCESS_MEMORY_COUNTERS>() as u32)
    };
    (ok != 0).then_some((
        counters.PagefileUsage as u64,
        counters.WorkingSetSize as u64,
        counters.PeakWorkingSetSize as u64,
    ))
}

#[cfg(target_os = "linux")]
fn process_counters() -> Option<(u64, u64, u64)> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let hwm = status
        .lines()
        .find_map(|line| line.strip_prefix("VmHWM:"))
        .and_then(|rest| rest.trim().strip_suffix(" kB"))?
        .parse::<u64>()
        .ok()?;
    Some((resident * 4096, resident * 4096, hwm * 1024))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn process_counters() -> Option<(u64, u64, u64)> {
    None
}

/// Bookkeeping for one input source in a report run.
#[derive(Debug)]
pub struct SourceRow {
    pub label: String,
    pub disk: u64,
    pub mmapped: bool,
    pub lines: u64,
    pub consumed: u64,
    pub parsed_json: u64,
    pub records: u64,
    pub bad_lines: u64,
    pub json: JsonWalk,
    pub records_walk: RecordWalk,
}

/// Everything a `--memory-report` run measured.
pub struct MemoryReport {
    pub layer: ReportLayer,
    pub sources: Vec<SourceRow>,
    pub checkpoints: Vec<Checkpoint>,
    pub peak_live: u64,
    pub peak_rss: Option<u64>,
    pub allocs_total: u64,
    pub live_class_bytes: [u64; alloc_count::SIZE_CLASSES],
    pub live_class_count: [u64; alloc_count::SIZE_CLASSES],
    pub json_measured: Option<u64>,
    pub json_model: u64,
    pub json_walk: JsonWalk,
    pub json_parsed_lines: u64,
    pub json_consumed_bytes: u64,
    pub json_maps_slots: u64,
    pub records_measured: Option<u64>,
    pub records_model: u64,
    pub records_walk: RecordWalk,
    pub records_slots: u64,
    pub records_slots_used: u64,
    pub records_parsed_lines: u64,
    pub records_consumed_bytes: u64,
    pub dedup: Dedup,
    pub truncated: bool,
}

impl MemoryReport {
    pub fn disk_total(&self) -> u64 {
        self.sources.iter().map(|row| row.disk).sum()
    }

    pub fn mapped_total(&self) -> u64 {
        self.sources.iter().filter(|row| row.mmapped).map(|row| row.disk).sum()
    }

    pub fn lines_total(&self) -> u64 {
        self.sources.iter().map(|row| row.lines).sum()
    }

    pub fn records_total(&self) -> u64 {
        self.sources.iter().map(|row| row.records).sum()
    }
}

/// Reads every configured source and writes the memory report to stdout.
pub fn run(config: &Config, layer: ReportLayer) -> miette::Result<()> {
    if matches!(config.format, FormatSelection::Forced(LogFormat::Plain)) {
        return Err(SilvError::Argument {
            flag: "--format".to_owned(),
            reason: "the memory report reads JSON Lines input only".to_owned(),
        }
        .into());
    }
    alloc_count::enable();
    let bundle = read_inputs(config)?;
    let report = build(&bundle, config, layer);
    alloc_count::disable();
    let mut out = std::io::stdout();
    let text = format_report(&report);
    out.write_all(text.as_bytes()).into_diagnostic()?;
    out.flush().into_diagnostic()
}

/// Runs the measured passes over an already loaded bundle.
pub fn build(bundle: &InputBundle, config: &Config, layer: ReportLayer) -> MemoryReport {
    let mut sources: Vec<SourceRow> = bundle
        .sources
        .iter()
        .map(|source| SourceRow {
            label: source.label.clone(),
            disk: source.len() as u64,
            mmapped: source.len() as u64 >= MMAP_THRESHOLD,
            lines: 0,
            consumed: 0,
            parsed_json: 0,
            records: 0,
            bad_lines: 0,
            json: JsonWalk::default(),
            records_walk: RecordWalk::default(),
        })
        .collect();

    let mut checkpoints = vec![checkpoint("after load")];
    let after_load = checkpoints[0].live;

    let mut json_measured = None;
    let mut json_walk = JsonWalk::default();
    let mut json_parsed_lines = 0u64;
    let mut json_consumed = 0u64;
    let mut json_maps_slots = 0u64;
    let mut dedup = Dedup::default();

    if layer.json_layer() {
        let mut all_values: Vec<Vec<IValue>> = Vec::with_capacity(bundle.sources.len());
        for (index, source) in bundle.sources.iter().enumerate() {
            let lines = split_lines(source.bytes());
            let consumed = lines
                .last()
                .map(|(_, offset, bytes)| *offset + bytes.len())
                .unwrap_or(0);
            let take = lines.len().min(config.max_records);
            let parsed: Vec<Option<IValue>> = lines[..take]
                .par_iter()
                .map(|(_, _, bytes)| serde_json::from_slice(bytes).ok())
                .collect();
            let mut values: Vec<IValue> = Vec::new();
            for value in parsed.into_iter().flatten() {
                add_root(&mut sources[index].json, &value, 0);
                values.push(value);
            }
            sources[index].lines = lines.len() as u64;
            sources[index].consumed = consumed as u64;
            sources[index].parsed_json = values.len() as u64;
            json_walk.merge(&sources[index].json);
            json_parsed_lines += values.len() as u64;
            json_consumed += consumed as u64;
            json_maps_slots += values.capacity() as u64 * VALUE_SLOT;
            all_values.push(values);
        }
        dedup.merge(dedup_maps(all_values.iter().flatten()));
        checkpoints.push(checkpoint("json layer"));
        json_measured = Some(checkpoints[1].live.saturating_sub(after_load));
        drop(all_values);
        checkpoints.push(checkpoint("json dropped"));
    }

    let mut records_measured = None;
    let mut records_walk = RecordWalk::default();
    let mut records_parsed_lines = 0u64;
    let mut records_consumed = 0u64;
    let mut records_slots = 0u64;
    let mut records_slots_used = 0u64;
    let mut truncated = false;
    let records_base = checkpoints.last().map(|c| c.live).unwrap_or(after_load);

    if layer.records_layer() {
        let aliases = config.aliases();
        let mut records: Vec<LogRecord> = Vec::new();
        for (index, source) in bundle.sources.iter().enumerate() {
            let source_id = u32::try_from(index).unwrap_or(u32::MAX);
            let lines = split_lines(source.bytes());
            let consumed = lines
                .last()
                .map(|(_, offset, bytes)| *offset + bytes.len())
                .unwrap_or(0);
            let take = lines.len().min(config.max_records.saturating_sub(records.len()));
            let results: Vec<LineResult> = lines[..take]
                .par_iter()
                .map(|(number, offset, bytes)| jsonl::parse_line(source_id, *number, *offset, bytes, aliases))
                .collect();
            for result in results {
                match result {
                    Ok(record) => {
                        if records.len() >= config.max_records {
                            truncated = true;
                            break;
                        }
                        add_record(&mut sources[index].records_walk, &record);
                        records.push(record);
                    },
                    Err(issue) => {
                        sources[index].bad_lines += 1;
                        if config.keep_bad_lines && records.len() < config.max_records {
                            let record =
                                LogRecord::invalid(issue.source, issue.line, issue.offset, &issue.reason, &issue.raw);
                            add_record(&mut sources[index].records_walk, &record);
                            records.push(record);
                        }
                    },
                }
            }
            sources[index].lines = sources[index].lines.max(lines.len() as u64);
            sources[index].consumed = sources[index].consumed.max(consumed as u64);
            sources[index].records = sources[index].records_walk.records;
            records_parsed_lines += u64::try_from(take).unwrap_or(u64::MAX);
            records_consumed += consumed as u64;
            records_walk.merge(&sources[index].records_walk);
            if truncated {
                break;
            }
        }
        records_slots = records.capacity() as u64 * RECORD_SLOT;
        records_slots_used = records.len() as u64 * RECORD_SLOT;
        checkpoints.push(checkpoint("record layer"));
        records_measured = Some(checkpoints.last().unwrap().live.saturating_sub(records_base));
        dedup.merge(dedup_maps(records.iter().map(|record| &record.object)));
    }

    let final_stats = alloc_count::snapshot();
    MemoryReport {
        layer,
        sources,
        checkpoints,
        peak_live: final_stats.peak_live,
        peak_rss: process_peak_rss(),
        allocs_total: final_stats.allocs,
        live_class_bytes: final_stats.live_by_class,
        live_class_count: final_stats.allocs_by_class,
        json_measured,
        json_model: json_walk.heap_est() + json_maps_slots,
        json_walk,
        json_parsed_lines,
        json_consumed_bytes: json_consumed,
        json_maps_slots,
        records_measured,
        records_model: records_walk.json.heap_est() + records_walk.extras_heap() + records_slots,
        records_walk,
        records_slots,
        records_slots_used,
        records_parsed_lines,
        records_consumed_bytes: records_consumed,
        dedup,
        truncated,
    }
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.2} {}", UNITS[unit])
}

fn ratio(bytes: u64, base: u64) -> String {
    if base == 0 {
        return "-".to_owned();
    }
    format!("{:.2}x", bytes as f64 / base as f64)
}

fn per_item(bytes: u64, items: u64) -> String {
    if items == 0 {
        return "-".to_owned();
    }
    format!("{:.0} B", bytes as f64 / items as f64)
}

fn signed_delta(current: u64, previous: u64) -> String {
    if current >= previous {
        format!("+{}", human(current - previous))
    } else {
        format!("-{}", human(previous - current))
    }
}

fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push('_');
        }
        out.push(ch);
    }
    out
}

fn pct(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "-".to_owned();
    }
    format!("{:.1}%", 100.0 * part as f64 / whole as f64)
}

/// Builds the full report text.
pub fn format_report(report: &MemoryReport) -> String {
    let disk = report.disk_total();
    let mapped = report.mapped_total();
    let lines = report.lines_total();
    let records = report.records_total();
    let json_measured = report.json_measured;
    let records_measured = report.records_measured;
    let walk = if report.records_measured.is_some() {
        &report.records_walk.json
    } else {
        &report.json_walk
    };
    let extras = report.records_walk.extras_heap();
    let internal_model = if report.records_measured.is_some() {
        report.records_model
    } else {
        report.json_model
    };
    let internal_measured = report.records_measured.unwrap_or(json_measured.unwrap_or(0));
    let dedup = &report.dedup;

    let target_env = if cfg!(target_env = "msvc") {
        "msvc"
    } else if cfg!(target_env = "gnu") {
        "gnu"
    } else if cfg!(target_env = "musl") {
        "musl"
    } else if cfg!(target_env = "macabi") {
        "macabi"
    } else {
        "unknown-env"
    };
    let mut out = String::with_capacity(8192);
    let _ = writeln!(
        out,
        "silv memory report v{} on {}-{}-{}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::ARCH,
        std::env::consts::OS,
        target_env,
    );
    let _ = writeln!(
        out,
        "layers measured: {}",
        match report.layer {
            ReportLayer::Json => "parsed json",
            ReportLayer::Records => "internal records",
            ReportLayer::All => "parsed json + internal records",
        }
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "static sizes (bytes):");
    let _ = writeln!(
        out,
        "  IValue={:<3} IString={:<3} object entry={:<3} object box={:<3} array box={:<3} String={:<3} LogRecord={:<3} line index slot={}",
        size_of::<IValue>(),
        size_of::<IString>(),
        ENTRY_SLOT as usize,
        OBJECT_BOX as usize,
        ARRAY_BOX as usize,
        size_of::<String>(),
        RECORD_SLOT as usize,
        LINE_INDEX_SLOT,
    );
    let _ = writeln!(out);

    let _ = writeln!(
        out,
        "input: {} source(s), {} on disk, {} line(s); {} mmapped (>= {}), heap load {}",
        report.sources.len(),
        human(disk),
        count(lines),
        human(mapped),
        human(MMAP_THRESHOLD),
        human(disk - mapped),
    );
    let _ = writeln!(
        out,
        "line index built per source during parsing: {} x {} B = {} (transient)",
        count(lines),
        LINE_INDEX_SLOT,
        human(lines * LINE_INDEX_SLOT),
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "heap checkpoints (user-requested bytes, counting allocator):");
    let _ = writeln!(out, "  {:<14} {:>12} {:>14} {:>14} {:>12}", "stage", "live", "delta", "allocs", "rss");
    let mut previous = 0u64;
    for stage in &report.checkpoints {
        let rss = stage.rss.map(human).unwrap_or_else(|| "n/a".to_owned());
        let delta = if stage.stage == "after load" {
            "-".to_owned()
        } else {
            signed_delta(stage.live, previous)
        };
        let _ = writeln!(
            out,
            "  {:<14} {:>12} {:>14} {:>14} {:>12}",
            stage.stage,
            human(stage.live),
            delta,
            count(stage.allocs),
            rss,
        );
        previous = stage.live;
    }
    let _ = writeln!(out, "  peak live {}", human(report.peak_live));
    if let Some(peak) = report.peak_rss {
        let _ = writeln!(out, "  peak rss {}", human(peak));
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "layer volumes:");
    let _ = writeln!(
        out,
        "  file (disk)                     {:>12}  {:>7}  {:>9}/line",
        human(disk),
        ratio(disk, disk),
        per_item(disk, lines),
    );
    if let Some(measured) = json_measured {
        let _ = writeln!(
            out,
            "  parsed Vec<IValue> (ijson)   {:>12}  {:>7}  {:>9}/record  model {}",
            human(measured),
            ratio(measured, disk),
            per_item(measured, report.json_parsed_lines.max(1)),
            human(report.json_model),
        );
    }
    if let Some(measured) = records_measured {
        let _ = writeln!(
            out,
            "  internal Vec<LogRecord>       {:>12}  {:>7}  {:>9}/record  model {}",
            human(measured),
            ratio(measured, disk),
            per_item(measured, report.records_walk.records.max(1)),
            human(report.records_model),
        );
        let _ = writeln!(
            out,
            "  internal minus parsed (alias columns, record slots): {}",
            human(measured.saturating_sub(json_measured.unwrap_or(0))),
        );
    }
    let _ = writeln!(
        out,
        "  model vs measured (internal): {} (allocator headers, fragmentation, model drift)",
        human(internal_model.abs_diff(internal_measured))
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "json value anatomy (walk, exact components plus table model):");
    let _ = writeln!(
        out,
        "  objects: {} node(s), avg width {:.1}, max width {}, avg depth {:.1}, max depth {}",
        count(walk.maps),
        walk.avg_map_width(),
        walk.map_max_width,
        walk.avg_depth(),
        walk.depth_max,
    );
    let _ = writeln!(
        out,
        "    boxes {} B x {} = {}, entry slots {} x {} B = {}, array boxes {} x {} = {}",
        OBJECT_BOX,
        count(walk.maps),
        human(walk.maps * OBJECT_BOX),
        count(walk.entries),
        ENTRY_SLOT,
        human(walk.entries * ENTRY_SLOT),
        ARRAY_BOX,
        count(walk.arrays),
        human(walk.arrays * ARRAY_BOX),
    );
    let _ = writeln!(
        out,
        "  keys: {} slot(s), counted {} (len {}, interned and shared across records), avg key {:.1} B",
        count(walk.entries),
        human(walk.keys_cap),
        human(walk.keys_len),
        walk.keys_len as f64 / walk.entries.max(1) as f64,
    );
    let _ = writeln!(
        out,
        "  strings: {} node(s), counted {} (len {}, interned above the inline threshold), avg len {:.0} B",
        count(walk.strings),
        human(walk.strings_cap),
        human(walk.strings_len),
        walk.strings_len as f64 / walk.strings.max(1) as f64,
    );
    let _ = writeln!(
        out,
        "  numbers: {} node(s) (i64 {}, u64 {}, f64 {}), {} inline in slots",
        count(walk.ints + walk.uints + walk.floats),
        count(walk.ints),
        count(walk.uints),
        count(walk.floats),
        human((walk.ints + walk.uints + walk.floats) * VALUE_SLOT),
    );
    let _ = writeln!(
        out,
        "  arrays: {} node(s), {} element(s), slots {}; bools {}; nulls {}",
        count(walk.arrays),
        count(walk.array_elems),
        human(walk.array_cap * VALUE_SLOT),
        count(walk.bools),
        count(walk.nulls),
    );
    let _ = writeln!(out, "  parsed layer model total {}", human(walk.heap_est()));
    let _ = writeln!(out);

    if report.records_measured.is_some() {
        let records_walk = &report.records_walk;
        let _ = writeln!(out, "internal layer extras (beyond the stored objects):");
        let _ = writeln!(
            out,
            "  LogRecord slots: {} x {} B = {} (vec capacity {}, growth waste {})",
            count(records_walk.records),
            RECORD_SLOT,
            human(report.records_slots_used),
            human(report.records_slots),
            human(report.records_slots.saturating_sub(report.records_slots_used)),
        );
        let _ = writeln!(
            out,
            "  message column: {} String(s), heap {} (len {})",
            count(records_walk.messages),
            human(records_walk.messages_cap),
            human(records_walk.messages_len),
        );
        let _ = writeln!(
            out,
            "  timestamp column: {} stamp(s), raw text heap {} (unparsed {}), i128 nanos inline",
            count(records_walk.stamps),
            human(records_walk.stamps_raw_len),
            count(records_walk.stamps_unparsed),
        );
        let _ = writeln!(
            out,
            "  level column: {} known (inline tag), {} custom String(s) heap {}",
            count(records_walk.levels_known),
            count(records_walk.levels_custom),
            human(records_walk.levels_custom_cap),
        );
        let _ = writeln!(
            out,
            "  invalid lines: {}, line text stored once in the object payload ({}, counted in the json walk)",
            count(records_walk.invalids),
            human(records_walk.invalids_raw_len),
        );
        let _ = writeln!(
            out,
            "  plain format raw lines: {} Box<str>, heap {}",
            count(records_walk.raw_lines),
            human(records_walk.raw_lines_len),
        );
        let _ = writeln!(
            out,
            "  extras total (alias columns, plain raw lines): {} ({} of the internal layer)",
            human(extras),
            pct(extras, internal_measured.max(1)),
        );
        let _ = writeln!(out);
    }

    if dedup.keys_total > 0 {
        let key_saving = walk.keys_cap.saturating_sub(dedup.keys_unique_bytes);
        let string_saving = walk.strings_cap.saturating_sub(dedup.strings_unique_bytes);
        let _ = writeln!(out, "repetition (dedup potential):");
        let _ = writeln!(
            out,
            "  field names: {} slot(s), heap {}, {} unique ({}), interning saves {} ({} of internal)",
            count(dedup.keys_total),
            human(walk.keys_cap),
            count(dedup.keys_unique),
            human(dedup.keys_unique_bytes),
            human(key_saving),
            pct(key_saving, internal_measured.max(1)),
        );
        let _ = writeln!(
            out,
            "  string values: {} node(s), heap {}, {} unique ({}), an arena saves up to {} ({} of internal)",
            count(dedup.strings_total),
            human(walk.strings_cap),
            count(dedup.strings_unique),
            human(dedup.strings_unique_bytes),
            human(string_saving),
            pct(string_saving, internal_measured.max(1)),
        );
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "allocation shape (counting allocator):");
    let final_live = report.checkpoints.last().map(|stage| stage.live).unwrap_or(0);
    let _ = writeln!(
        out,
        "  {} allocation(s), mean request {:.0} B, live at last checkpoint {}",
        count(report.allocs_total),
        final_live as f64 / report.allocs_total.max(1) as f64,
        human(final_live),
    );
    let small = report.live_class_count[..=6].iter().sum::<u64>();
    let _ = writeln!(
        out,
        "  allocations up to 64 B: {} ({} of all)",
        count(small),
        pct(small, report.allocs_total.max(1)),
    );
    let mut classes: Vec<(usize, u64, u64)> = report
        .live_class_bytes
        .iter()
        .enumerate()
        .filter(|(_, bytes)| **bytes > 0)
        .map(|(class, bytes)| (class, *bytes, report.live_class_count[class]))
        .collect();
    classes.sort_by_key(|(_, bytes, _)| std::cmp::Reverse(*bytes));
    for (class, bytes, allocs) in classes.iter().take(5) {
        let low = if *class == 0 { 1 } else { (1u64 << class) / 2 + 1 };
        let high = 1u64 << *class;
        let _ = writeln!(
            out,
            "    {}..={} B: {:>12} live in {:>14} allocation(s)",
            human(low),
            human(high),
            human(*bytes),
            count(*allocs),
        );
    }
    let _ = writeln!(out);

    if report.sources.len() > 1 {
        let _ = writeln!(out, "per source (model derived from the walks):");
        let _ = writeln!(
            out,
            "  {:<44} {:>10} {:>10} {:>10} {:>10} {:>10}",
            "label", "disk", "lines", "records", "json", "internal"
        );
        for row in &report.sources {
            let label = row.label.replace('\\', "/");
            let label = if label.len() > 44 {
                label[label.len() - 44..].to_owned()
            } else {
                label
            };
            let json_est = row.json.heap_est() + row.parsed_json * VALUE_SLOT;
            let internal_est =
                row.records_walk.json.heap_est() + row.records_walk.extras_heap() + row.records * RECORD_SLOT;
            let _ = writeln!(
                out,
                "  {:<44} {:>10} {:>10} {:>10} {:>10} {:>10}",
                label,
                human(row.disk),
                count(row.lines),
                count(row.records),
                human(json_est),
                human(internal_est),
            );
        }
        let _ = writeln!(out);
    }

    if report.truncated {
        let parsed = report.records_parsed_lines.max(1);
        let factor = lines.max(1) as f64 / parsed as f64;
        let _ = writeln!(
            out,
            "truncated at --max-records: layers measured on {} of {} line(s); full-input estimates (uniform lines):",
            count(parsed),
            count(lines),
        );
        let _ = writeln!(
            out,
            "  internal {} x {:.2} = {}",
            human(internal_measured),
            factor,
            human((internal_measured as f64 * factor) as u64),
        );
        let _ = writeln!(out);
    }

    let key_saving = walk.keys_cap.saturating_sub(dedup.keys_unique_bytes);
    let string_saving = walk.strings_cap.saturating_sub(dedup.strings_unique_bytes);
    let object_cost = walk.maps * (OBJECT_BOX + ARRAY_BOX / 4) + walk.entries * ENTRY_SLOT;
    let _ = writeln!(out, "optimization notes for the internal layout:");
    let _ = writeln!(
        out,
        "  1. field names: {} unique names own {} of interned text shared across all records; the {} counted per occurrence collapses to the unique set ({} realized, {} of the internal layer). ijson's global intern table already delivers this saving.",
        count(dedup.keys_unique),
        human(dedup.keys_unique_bytes),
        human(walk.keys_cap),
        human(key_saving),
        pct(key_saving, internal_measured.max(1)),
    );
    let _ = writeln!(
        out,
        "  2. string values: {} unique strings own {} vs {} counted per occurrence; values above the inline threshold are interned by ijson, realizing up to {} ({}).",
        count(dedup.strings_unique),
        human(dedup.strings_unique_bytes),
        human(walk.strings_cap),
        human(string_saving),
        pct(string_saving, internal_measured.max(1)),
    );
    let _ = writeln!(
        out,
        "  3. alias columns duplicate text already inside the record object (message {}, timestamp raw {}, level custom {}): {} ({} of the internal layer). Store the winning alias path (bytes) and render from the object; recovers all of it.",
        human(report.records_walk.messages_cap),
        human(report.records_walk.stamps_raw_len),
        human(report.records_walk.levels_custom_cap),
        human(extras),
        pct(extras, internal_measured.max(1)),
    );
    let _ = writeln!(
        out,
        "  4. objects: {} boxes + {} entry slots = {} of parsed cost (avg width {:.1}); ijson stores objects as flat entry vectors of interned keys (no hash table, pointer-equality probe), so this is the floor for a self-describing DOM.",
        human(walk.maps * OBJECT_BOX),
        human(walk.entries * ENTRY_SLOT),
        human(object_cost),
        walk.avg_map_width(),
    );
    let _ = writeln!(
        out,
        "  5. allocation churn: {} allocations, {:.0} per record, mean request {:.0} B; records are immutable after parse, so arena-allocating map boxes, entry vectors and string heaps removes allocator headers (>= 8 B per alloc) and packs records contiguously for scanning.",
        count(report.allocs_total),
        report.allocs_total as f64 / records.max(1).max(1) as f64,
        final_live as f64 / report.allocs_total.max(1) as f64,
    );
    let _ = writeln!(
        out,
        "  6. LogRecord is {} B fixed: slot array {} for {} records; `offset` is redundant with source+line, and the timestamp `i128` fits in `i64` nanos for years 1677-2262: packing shrinks the slot array.",
        RECORD_SLOT,
        human(report.records_slots),
        count(records.max(1)),
    );
    let _ = writeln!(
        out,
        "  7. line index (split_lines) is {} B per line ({} transient per multi-source run); parse from (offset, len) pairs of 16 B to halve it. Files >= {} are mmapped, so on-disk bytes stay off-heap and the record representation is the whole story.",
        LINE_INDEX_SLOT,
        human(lines * LINE_INDEX_SLOT),
        human(MMAP_THRESHOLD),
    );
    let _ = writeln!(
        out,
        "  8. filtering adds no storage: predicates evaluate against the stored values (text_of borrows strings, formats numbers into a reused buffer), so layout savings pay back as cache pressure on every search and filter pass."
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(text: &str) -> IValue {
        serde_json::from_str(text).expect("fixture object")
    }

    #[test]
    fn walks_counts_all_kinds() {
        let mut walk = JsonWalk::default();
        add_root(&mut walk, &map(r#"{"a": 1, "b": -2, "c": 1.5, "d": "x", "e": [true, null, {"f": 3}]}"#), 0);
        assert_eq!(walk.maps, 2);
        assert_eq!(walk.entries, 6);
        assert_eq!(walk.ints, 3);
        assert_eq!(walk.floats, 1);
        assert_eq!(walk.strings, 1);
        assert_eq!(walk.arrays, 1);
        assert_eq!(walk.array_elems, 3);
        assert_eq!(walk.bools, 1);
        assert_eq!(walk.nulls, 1);
        assert!(walk.heap_est() > 0);
    }

    #[test]
    fn merges_walks() {
        let mut a = JsonWalk::default();
        let mut b = JsonWalk::default();
        add_root(&mut a, &map(r#"{"a": 1}"#), 0);
        add_root(&mut b, &map(r#"{"bb": "s", "cc": 2}"#), 0);
        a.merge(&b);
        assert_eq!(a.maps, 2);
        assert_eq!(a.strings, 1);
        assert_eq!(a.map_max_width, 2);
    }

    #[test]
    fn dedup_finds_repetition() {
        let maps = [map(r#"{"level": "INFO"}"#), map(r#"{"level": "WARN"}"#)];
        let dedup = dedup_maps(maps.iter());
        assert_eq!(dedup.keys_total, 2);
        assert_eq!(dedup.keys_unique, 1);
        assert_eq!(dedup.keys_unique_bytes, 5);
        assert_eq!(dedup.strings_total, 2);
        assert_eq!(dedup.strings_unique, 2);
    }

    #[test]
    fn human_formats_units() {
        assert_eq!(human(512), "512 B");
        assert_eq!(human(2048), "2.00 KiB");
        assert_eq!(human(5 * 1024 * 1024), "5.00 MiB");
    }

    #[test]
    fn record_walk_extracts_columns() {
        let aliases = crate::model::record::FieldAliases::with_defaults(&[], &[], &[]).expect("aliases");
        let record = crate::parse::jsonl::parse_line(
            0,
            1,
            0,
            br#"{"@timestamp":"2023-11-14T22:13:20.123Z","level":"WARN","message":"disk full"}"#,
            &aliases,
        )
        .expect("parses");
        let mut walk = RecordWalk::default();
        add_record(&mut walk, &record);
        assert_eq!(walk.records, 1);
        assert_eq!(walk.messages, 1);
        assert_eq!(walk.stamps, 1);
        assert_eq!(walk.levels_known, 1);
        assert!(walk.extras_heap() >= "disk full".len() as u64);
    }
}
