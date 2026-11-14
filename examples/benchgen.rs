//! Deterministic JSON Lines benchmark log generator for the `--memory-report`
//! benchmarks. Writes files with a flat memory profile (streaming writer).
//!
//! Usage: `cargo run --release --example benchgen -- --shape mixed --preset xl --out .ai/tmp/bench/xl-mixed.jsonl`

use std::fmt::Write as _;
use std::io::BufWriter;
use std::io::Write;
use std::path::PathBuf;

use clap::Parser;
use clap::ValueEnum;

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
enum Shape {
    /// Realistic application logs: all value kinds, nested objects, arrays.
    Mixed,
    /// Digit heavy: many i64, u64 and f64 fields per record.
    Numbers,
    /// String heavy: long messages, ids, repeated labels.
    Strings,
    /// Deep nesting: nested objects and arrays of objects.
    Nested,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
enum Preset {
    /// 1 MiB.
    Small,
    /// 16 MiB.
    Medium,
    /// 100 MiB.
    Large,
    /// 500 MiB.
    Xl,
}

impl Preset {
    const fn bytes(self) -> u64 {
        match self {
            Self::Small => 1 << 20,
            Self::Medium => 16 << 20,
            Self::Large => 100 << 20,
            Self::Xl => 500 << 20,
        }
    }
}

#[derive(Parser)]
#[command(name = "benchgen", about = "Generate deterministic benchmark JSON Lines for --memory-report")]
struct Args {
    /// Record shape family
    #[arg(long, value_enum, default_value = "mixed")]
    shape: Shape,

    /// Target size preset, overridden by --bytes
    #[arg(long, value_enum)]
    preset: Option<Preset>,

    /// Exact target size in bytes
    #[arg(long)]
    bytes: Option<u64>,

    /// Write N shard files into the output directory (directory input scenario)
    #[arg(long, default_value_t = 1)]
    files: usize,

    /// Output file (files=1) or output directory (files>1)
    #[arg(long)]
    out: PathBuf,

    /// Generator seed
    #[arg(long, default_value_t = 0x5EED)]
    seed: u64,
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }

    fn pick(&mut self, items: &[&'static str]) -> &'static str {
        items[self.below(items.len() as u64) as usize]
    }

    fn hex(&mut self, nibbles: usize) -> u64 {
        self.next() & ((1u128 << (nibbles * 4)) - 1) as u64
    }
}

const WORDS: [&str; 42] = [
    "request", "response", "connection", "timeout", "retry", "queue", "batch", "segment", "partition", "offset",
    "commit", "cache", "eviction", "handshake", "certificate", "rotation", "snapshot", "compaction", "replica",
    "leader", "election", "heartbeat", "deadline", "backpressure", "buffer", "flush", "fsync", "wal", "checkpoint",
    "shard", "replication", "vacuum", "index", "reindex", "vacuuming", "ingest", "pipeline", "stage", "drain",
    "spill", "archive", "retention",
];

const SERVICES: [&str; 8] = [
    "auth-service",
    "billing-api",
    "search-gateway",
    "ingest-worker",
    "cache-proxy",
    "scheduler-core",
    "notifier",
    "stream-consumer",
];

const HOSTS: [&str; 7] = ["web-01", "web-02", "web-03", "node-a1", "node-b2", "runner-7", "runner-8"];

const LOGGERS: [&str; 7] = [
    "app.handlers.auth",
    "aiokafka.consumer",
    "sqlx.pool",
    "hyper.proto",
    "tower.retry",
    "rustls.conn",
    "reqwest.client",
];

const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

const PATHS: [&str; 6] = [
    "/api/v1/orders",
    "/api/v2/search",
    "/internal/health",
    "/metrics",
    "/api/v1/users/session",
    "/graphql",
];

const NAMES: [&str; 8] = [
    "ada", "bruno", "clara", "dmitri", "elena", "boris", "greta", "hugo",
];

const LEVELS: [(u64, &str); 6] = [
    (55, "INFO"),
    (70, "DEBUG"),
    (85, "WARN"),
    (93, "ERROR"),
    (97, "TRACE"),
    (100, "FATAL"),
];

fn level(rng: &mut Rng) -> &'static str {
    let roll = rng.below(100);
    for (edge, name) in LEVELS {
        if roll < edge {
            return name;
        }
    }
    "INFO"
}

fn message(rng: &mut Rng, out: &mut String) {
    let words = 4 + rng.below(10) as usize;
    for index in 0..words {
        if index > 0 {
            out.push(' ');
        }
        out.push_str(rng.pick(&WORDS));
    }
}

/// RFC3339 milliseconds text from epoch milliseconds (Howard Hinnant's civil algorithm).
fn write_ts(out: &mut String, epoch_ms: u64) {
    let days = (epoch_ms / 1000 / 86_400) as i64;
    let secs_day = (epoch_ms / 1000 % 86_400) as u32;
    let millis = (epoch_ms % 1000) as u32;
    let (year, month, day) = civil_from_days(days);
    let _ = write!(
        out,
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year,
        month,
        day,
        secs_day / 3600,
        secs_day / 60 % 60,
        secs_day % 60,
        millis
    );
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { y + 1 } else { y }, month, day)
}

fn write_mixed(rng: &mut Rng, line: &mut String, epoch_ms: &mut u64) {
    *epoch_ms += 1 + rng.below(750);
    line.clear();
    line.push('{');
    let _ = write!(line, r#""@timestamp":""#);
    write_ts(line, *epoch_ms);
    let _ = write!(
        line,
        r#"","level":"{}","logger":"{}","service":"{}","host":"{}","pid":{},"#,
        level(rng),
        rng.pick(&LOGGERS),
        rng.pick(&SERVICES),
        rng.pick(&HOSTS),
        1000 + rng.below(64_000),
    );
    line.push_str(r#""message":""#);
    message(rng, line);
    let _ = write!(
        line,
        r#"","trace_id":"{:016x}","span_id":"{:08x}","duration_ms":{:.2},"status":{},"bytes_sent":{},"ok":{},"#,
        rng.hex(16),
        rng.hex(8),
        rng.below(120_000) as f64 / 100.0,
        rng.pick(&["200", "201", "204", "400", "401", "404", "429", "500", "503"]),
        rng.below(4_000_000),
        rng.chance(9, 10),
    );
    let _ = write!(
        line,
        r#""user":{{"id":{},"name":"{}","email":"{}@{}.example.com"}},"http":{{"method":"{}","path":"{}","status":{}}},"#,
        rng.below(900_000) + 1000,
        rng.pick(&NAMES),
        rng.pick(&NAMES).to_ascii_lowercase(),
        rng.pick(&SERVICES),
        rng.pick(&METHODS),
        rng.pick(&PATHS),
        rng.pick(&["200", "400", "404", "500"]),
    );
    if rng.chance(2, 5) {
        let _ = write!(
            line,
            r#""k8s":{{"pod":"{}-{}","namespace":"prod","container":"main"}},"#,
            rng.pick(&SERVICES),
            rng.hex(4),
        );
    }
    let tags = rng.below(5) as usize;
    line.push_str(r#""tags":["#);
    for index in 0..tags {
        if index > 0 {
            line.push(',');
        }
        let _ = write!(line, r#""{}""#, rng.pick(&WORDS));
    }
    line.push_str("]");
    if rng.chance(1, 12) {
        line.push_str(r#","error":null,"sample_rate":0.10,"context":{"region":"eu-central-1","shard":3}"#);
    }
    line.push('}');
    line.push('\n');
}

fn write_numbers(rng: &mut Rng, line: &mut String, epoch_ms: &mut u64) {
    *epoch_ms += 1 + rng.below(750);
    line.clear();
    line.push('{');
    line.push_str(r#""@timestamp":"#);
    let _ = write!(line, "{}.423", *epoch_ms / 1000);
    let _ = write!(line, r#","id":{},"code":{}"#, rng.below(100_000_000_000_000_000) + 10, 200 + rng.below(5));
    for index in 0..8 {
        let _ = write!(line, r#","n{}":{}"#, index, (rng.below(1_800_000_000) as i64) - 900_000_000);
    }
    for index in 8..14 {
        let _ = write!(line, r#","n{}":{}"#, index, 1_000_000_000_000 + rng.below(9_000_000_000_000));
    }
    for index in 14..20 {
        let _ = write!(
            line,
            r#","n{}":{:.6}"#,
            index,
            rng.below(100_000_000) as f64 / 100_000.0
        );
    }
    line.push('}');
    line.push('\n');
}

fn write_strings(rng: &mut Rng, line: &mut String, epoch_ms: &mut u64) {
    *epoch_ms += 1 + rng.below(750);
    line.clear();
    line.push('{');
    line.push_str(r#""@timestamp":""#);
    write_ts(line, *epoch_ms);
    let _ = write!(
        line,
        r#"","level":"{}","service":"{}","message":""#,
        level(rng),
        rng.pick(&SERVICES),
    );
    message(rng, line);
    for _ in 0..4 + rng.below(20) {
        line.push(' ');
        line.push_str(rng.pick(&WORDS));
    }
    let _ = write!(
        line,
        r#"","trace_id":"{:016x}","span_id":"{:016x}","pod":"{}-{}","node":"{}","request_path":"{}","user_agent":"Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/129.0.0.0 Safari/537.36","email":"{}@{}corp.example.com","session":"{:08x}-{:04x}-{:04x}","cluster":"prod-eu","labels":["{}","{}","{}"]"#,
        rng.hex(16),
        rng.hex(16),
        rng.pick(&SERVICES),
        rng.hex(5),
        rng.pick(&HOSTS),
        rng.pick(&PATHS),
        rng.pick(&NAMES).to_ascii_lowercase(),
        rng.pick(&SERVICES),
        rng.hex(8),
        rng.hex(4),
        rng.hex(4),
        rng.pick(&WORDS),
        rng.pick(&WORDS),
        rng.pick(&WORDS),
    );
    line.push('}');
    line.push('\n');
}

fn write_nested(rng: &mut Rng, line: &mut String, epoch_ms: &mut u64) {
    *epoch_ms += 1 + rng.below(750);
    line.clear();
    line.push('{');
    line.push_str(r#""@timestamp":""#);
    write_ts(line, *epoch_ms);
    line.push_str(r#"","level":""#);
    line.push_str(level(rng));
    line.push_str(r#"","msg":""#);
    message(rng, line);
    for _ in 0..4 + rng.below(12) {
        line.push(' ');
        line.push_str(rng.pick(&WORDS));
    }
    line.push_str(r#"","event":{"id":"#);
    let _ = write!(line, "{}", rng.below(1_000_000));
    line.push_str(r#","attrs":{"a":{"b":{"c":{"d":{"e":"#);
    let _ = write!(line, "{}", rng.below(2_000_000) as i64 - 1_000_000);
    line.push_str(r#","f":""#);
    line.push_str(rng.pick(&WORDS));
    line.push_str(r#""}}}}},"tags":["#);
    let tags = 2 + rng.below(4) as usize;
    for index in 0..tags {
        if index > 0 {
            line.push(',');
        }
        line.push_str(r#"{"k":""#);
        line.push_str(rng.pick(&WORDS));
        line.push_str(r#"","v":""#);
        line.push_str(rng.pick(&WORDS));
        line.push_str(r#""}"#);
    }
    line.push_str(r#"],"ctx":{"req":{"headers":{"ua":""#);
    line.push_str(rng.pick(&WORDS));
    line.push_str(r#"","path":""#);
    line.push_str(rng.pick(&PATHS));
    line.push_str(r#"","auth":"bearer "#);
    let _ = write!(line, "{:016x}", rng.hex(16));
    line.push_str(r#""},"timings":{"t1":"#);
    for t in 2..=5u8 {
        let _ = write!(line, "{:.2}", rng.below(50_000) as f64 / 100.0);
        let _ = write!(line, r#","t{}":"#, t);
    }
    let _ = write!(line, "{:.2}", rng.below(50_000) as f64 / 100.0);
    line.push_str(r#"}},"resources":{"res":["#);
    let resources = 2 + rng.below(3) as usize;
    for index in 0..resources {
        if index > 0 {
            line.push(',');
        }
        line.push_str(r#"{"name":""#);
        line.push_str(rng.pick(&WORDS));
        line.push_str(r#"","kind":""#);
        line.push_str(rng.pick(&["pod", "service", "configmap", "secret"]));
        line.push_str(r#"","attrs":{"n":""#);
        line.push_str(rng.pick(&NAMES));
        line.push_str(r#"","q":"#);
        let _ = write!(line, "{}", rng.below(1000));
        line.push_str(r#"}}"#);
    }
    line.push_str(r#"]}},"meta":{"deep":{"deeper":{"deepest":{"n":"#);
    let _ = write!(line, "{}", rng.below(10_000));
    line.push_str(r#","s":""#);
    line.push_str(rng.pick(&NAMES));
    line.push_str(r#""}}}},"links":["#);
    let links = 1 + rng.below(3) as usize;
    for index in 0..links {
        if index > 0 {
            line.push(',');
        }
        let _ = write!(line, r#""{:016x}""#, rng.hex(16));
    }
    line.push_str("]}");
    line.push_str("}\n");
}

fn main() {
    let args = Args::parse();
    let total = args
        .bytes
        .or(args.preset.map(Preset::bytes))
        .unwrap_or(Preset::Small.bytes());
    let files = args.files.max(1);

    if files > 1 {
        std::fs::create_dir_all(&args.out).expect("create output directory");
    } else if let Some(parent) = args.out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).expect("create output parent");
        }
    }

    let per_file = total / files as u64;
    let mut grand_bytes = 0u64;
    let mut grand_lines = 0u64;
    for index in 0..files {
        let target = per_file + if index == 0 { total - per_file * files as u64 } else { 0 };
        let path = if files > 1 {
            args.out.join(format!("shard-{:02}.jsonl", index + 1))
        } else {
            args.out.clone()
        };
        let file = std::fs::File::create(&path).expect("create output file");
        let mut writer = BufWriter::with_capacity(1 << 20, file);
        let mut rng = Rng(args.seed ^ ((index as u64).wrapping_add(1) * 0x9E37_79B9_7F4A_7C15));
        let mut epoch_ms: u64 = 1_700_000_000_000 + index as u64 * 10;
        let mut line = String::with_capacity(1 << 16);
        let mut bytes = 0u64;
        let mut lines = 0u64;
        while bytes < target {
            line.clear();
            match args.shape {
                Shape::Mixed => write_mixed(&mut rng, &mut line, &mut epoch_ms),
                Shape::Numbers => write_numbers(&mut rng, &mut line, &mut epoch_ms),
                Shape::Strings => write_strings(&mut rng, &mut line, &mut epoch_ms),
                Shape::Nested => write_nested(&mut rng, &mut line, &mut epoch_ms),
            }
            writer.write_all(line.as_bytes()).expect("write line");
            bytes += line.len() as u64;
            lines += 1;
        }
        writer.flush().expect("flush");
        grand_bytes += bytes;
        grand_lines += lines;
        println!("{}: {} bytes, {} lines", path.display(), bytes, lines);
    }
    println!(
        "total {} bytes, {} lines, shape {:?}, seed {}",
        grand_bytes, grand_lines, args.shape, args.seed
    );
}
