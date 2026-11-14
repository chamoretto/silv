# Memory benchmarks (v0.2.0)

Measurements of what `silv` costs in RAM for a given JSON Lines input, and proposals
for shrinking the internal representation. Since v0.2.0 the parsed payload is an
`ijson::IValue` DOM (interned strings, 8-byte values) stored directly in
`LogRecord.object`; the v0.1.4 `serde_json::Map` baseline is kept below for
comparison. All numbers are from the built-in `--memory-report` mode, which uses
a counting global allocator (user-requested bytes, not allocator bookkeeping)
plus process RSS from the OS.

Environment: Windows 11 x86_64-pc-windows-msvc, release build, rayon default pool,
serde_json 1.0.154 with `preserve_order` + `raw_value`, ijson 0.1.7.

## How to reproduce

The report mode and the comparison harnesses live behind the non-default
`mem-bench` feature; release builds of silv itself never contain them.

Generate benchmark logs (deterministic, seed 24301):

```powershell
cargo run --release --example benchgen -- --shape {mixed,numbers,strings,nested} --preset {small,medium,large,xl} --out .ai/tmp/bench/<preset>-<shape>.jsonl
cargo run --release --example benchgen -- --shape mixed --preset xl --files 40 --out .ai/tmp/bench/xl-dir-mixed
```

Build the instrumented binaries, then run the report (the flag takes a layer:
`json`, `records` or `all`):

```powershell
cargo build --release --features mem-bench --bins --examples
target\release\silv.exe <file.jsonl> --memory-report all
```

Comparison harnesses (all `required-features = ["mem-bench"]`):

```powershell
target\release\examples\memcompare.exe .ai/tmp/bench/medium-mixed.jsonl        # per-crate DOM table
target\release\examples\pipeline_compare.exe <files...>                        # keep-the-DOM peak test
target\release\examples\strvec_compare.exe .ai/tmp/bench/medium-mixed.jsonl    # compact string/vector types
target\release\examples\arena_compare.exe .ai/tmp/bench/small-mixed.jsonl      # thunderdome arena
```

## Shapes

| shape   | content                                                                      |
|---------|------------------------------------------------------------------------------|
| mixed   | realistic app logs: 8-20 fields, nested objects, tags array, all value kinds |
| numbers | 24 numeric fields: i64, large u64, f64                                       |
| strings | long messages, ids, user-agent, labels; almost no numbers                    |
| nested  | deep objects (6 levels), arrays of objects (tags, resources)                 |

Presets: 1 MiB, 16 MiB, 100 MiB, 500 MiB. The 500 MiB rows for single files are
truncated at `--max-records 500000` (default, applied per source), so their disk
ratios are partial; bytes-per-record values stay valid. The 40-shard directory
run parses all 976k lines.

## Static layout (measured on this platform)

```
IValue = 8 B   IString = 8 B   object entry (IString, IValue) = 16 B
object box = 24 B   array box = 24 B   String = 24 B
LogRecord = 128 B   line index slot = 32 B
```

v0.1.4 for comparison: `Value = 72 B`, `(String, Value) = 96 B`,
`Map(serde) = 72 B` (boxing an `IndexMap`), `LogRecord = 192 B`.

## Volume growth (counting allocator, user-requested bytes)

`json` = `Vec<IValue>` as parsed; `internal` = `Vec<LogRecord>` (the
representation used for viewing, filtering and search). Ratio is versus on-disk
file size. B/rec is bytes per record. Peak RSS includes file load (heap load
below 8 MiB, mmmap at/above), the layer under measurement, and rayon buffers.

| file    | shape    | lines      | json / file | internal / file | B/rec json | B/rec internal | peak RSS  |
|---------|----------|------------|-------------|-----------------|------------|----------------|-----------|
| 1 MiB   | mixed    | 1_948      | 2.57x       | 2.74x           | 1384       | 1473           | 13.5 MiB  |
| 1 MiB   | numbers  | 2_582      | 2.51x       | 2.77x           | 1020       | 1126           | 13.4 MiB  |
| 1 MiB   | strings  | 1_556      | 1.68x       | 1.98x           | 1134       | 1331           | 13.0 MiB  |
| 1 MiB   | nested   | 1_159      | 4.49x       | 4.67x           | 4064       | 4227           | 15.6 MiB  |
| 16 MiB  | mixed    | 31_233     | 2.34x       | 2.73x           | 1256       | 1466           | 85.2 MiB  |
| 16 MiB  | numbers  | 41_298     | 2.28x       | 2.77x           | 928        | 1126           | 84.1 MiB  |
| 16 MiB  | strings  | 24_881     | 1.45x       | 1.97x           | 977        | 1327           | 70.0 MiB  |
| 16 MiB  | nested   | 18_506     | 4.26x       | 4.66x           | 3860       | 4229           | 118.9 MiB |
| 100 MiB | mixed    | 195_227    | 2.28x       | 2.75x           | 1223       | 1475           | 462.2 MiB |
| 100 MiB | numbers  | 258_115    | 2.26x       | 2.59x           | 918        | 1053           | 437.9 MiB |
| 100 MiB | strings  | 155_560    | 1.47x       | 2.07x           | 992        | 1395           | 391.9 MiB |
| 100 MiB | nested   | 115_728    | 4.26x       | 4.60x           | 3860       | 4163           | 655.3 MiB |
| 500 MiB | mixed*   | 976_123    | 1.19x*      | 1.40x*          | 1245       | 1463           | 1.37 GiB  |
| 500 MiB | numbers* | 1_290_583  | 0.88x*      | 1.01x*          | 918        | 1057           | 1.14 GiB  |
| 500 MiB | strings* | 777_607    | 0.90x*      | 1.21x*          | 944        | 1272           | 1.28 GiB  |
| 500 MiB | nested*  | 578_518    | 3.67x*      | 3.96x*          | 3852       | 4148           | 2.74 GiB  |
| 500 MiB | 40 files | 976_403    | 2.32x       | 1.39x/…         | 1245       | 1462           | 1.96 GiB  |

\* first 500k records parsed (cap); ratio versus full 500 MiB file is therefore
low. Per-record cost is stable across scales: mixed ~1.47 KB, numbers ~1.05-1.13
KB, strings ~1.27-1.40 KB, nested ~4.15-4.23 KB (internal, B/record).

## v0.1.4 baseline (serde_json Map DOM)

Same corpus, same tool, v0.1.4 records stored `Map<String, Value>`:

| shape   | B/rec internal (v0.1.4 -> v0.2.0) | saved | 16 MiB peak RSS    | 500 MiB peak RSS |
|---------|-----------------------------------|-------|--------------------|------------------|
| mixed   | 5013 -> 1466                      | -71%  | 215.8 -> 85.2 MiB  | 3.31 -> 1.37 GiB |
| numbers | 3601 -> 1126                      | -69%  | 201.2 -> 84.1 MiB  | 2.47 -> 1.14 GiB |
| strings | 2940 -> 1327                      | -55%  | 122.5 -> 70.0 MiB  | 2.22 -> 1.28 GiB |
| nested  | 12624 -> 4229                     | -67%  | 299.7 -> 118.9 MiB | 7.26 -> 2.74 GiB |

The 40-file directory run dropped from 5.70 GiB to 1.96 GiB peak RSS.

## Peak equals final

The parsed DOM *is* the resident payload (`LogRecord.object` is moved in, there
is no DOM-to-record copy), so peak heap equals final heap by construction.
`pipeline_compare` confirms it at every scale: the 500 MiB mixed run settles at
1.24 GiB payload (1361 B/rec, 13 allocs/rec) with peak identical to live, and
`serde_json::from_slice::<IValue>` deserializes straight into the IValue DOM, so
there is no parse-time transient. ijson beat sonic-rs (2301 B/rec, 2.09 GiB) on
final heap; sonic remains the speed-first option (5 allocs/rec, SIMD parse with
target features). The crate-by-crate comparison (serde_json, ijson, sonic-rs,
datavalue, nanoserde typed) is reproducible with `memcompare`; on medium-mixed
live B/rec: serde_json 4791, ijson 1248, sonic 2184, datavalue 8619 (arena
reserved), nanoserde typed 908, silv LogRecord (full record) 1460.

## Model fit

`--memory-report` builds its numbers two ways: an exact component walk (boxes,
entry slots, string lengths counted per occurrence) and the counting allocator
measuring everything for real. On medium-mixed the internal-layer model is
32.86 MiB versus 43.67 MiB measured (75% fit). The gap is dominated by allocator
headers (~570k record-layer allocations at >= 8 B each) and fragmentation;
walking counts key/string bytes per occurrence, while the allocator measures
interned text once across all records, which pulls the model the other way. The
old hash-table capacity model is gone: ijson objects are flat entry vectors
(interned keys, pointer-equality probe), with no index tables to model.

## Where the bytes go (medium-mixed, internal layer 43.67 MiB)

| component                                          | size      | share  |
|----------------------------------------------------|-----------|--------|
| entry slots (750_212 x 16 B, key + value words)    | 11.45 MiB | 26.2%  |
| string value heap (137_437 unique, interned)       | 3.68 MiB  | 8.4%   |
| LogRecord slot array (31_233 x 128 B)              | 3.81 MiB  | 8.7%   |
| alias columns (message, timestamp raw)             | 2.75 MiB  | 6.3%   |
| object boxes (108_815 nested+root x 24 B)          | 2.49 MiB  | 5.7%   |
| array boxes (31_233 x 24 B) + element slots        | 1.20 MiB  | 2.8%   |
| key heap (20 unique names, interned)               | 124 B     | ~0%    |
| walk model subtotal (per-occurrence strings/keys)  | 32.86 MiB | 75.2%  |
| allocator headers, fragmentation, interning        | 10.81 MiB | 24.8%  |

Numbers, bools and nulls are inline in the 8 B value half of each entry slot
(192_642 numbers: i64 159_096, f64 33_546). Strings longer than the inline
threshold are interned: 6.88 MiB counted per occurrence, 3.68 MiB stored unique.

Allocation shape: 18.2 allocations per record for the full record layer (v0.1.4:
59.1), mean request ~80 B, 37% of requests 64 B or smaller. Filtering adds no
storage (predicates borrow the stored values; numbers format into a reused
buffer).

## Optimization notes (remaining levers, with recovered share of the internal layer)

1. Alias columns (message/timestamp/level extracted into per-record columns)
   duplicate text already in the object: 2.75 MiB, 6.3%. Store the winning path
   and render from the object; recovers all of it. This is now the largest
   single lever.
2. Arena allocation: 18.2 allocs/record at ~80 B mean; a bump arena over the
   mmapped file (records are immutable after parse) removes allocator headers
   (the 24.8% model gap) and packs records contiguously; pays off in cache
   pressure on every filter pass.
3. LogRecord packing: 128 B fixed slots (8.7%); drop `offset` (redundant with
   source+line), store timestamp nanos as i64 (valid 1677-2262) to shrink the
   slot array (~4%).
4. Line index: 32 B/line transient (976 KiB per 31k lines, 29.8 MB at the
   500k-record cap); parse from (offset, len) 16 B pairs.
5. String capacity: string columns (message) allocate with slack; CompactString
   (inline <= 24 B, zero heap) fits the alias columns - see the strvec harness.

Already realized by ijson (the v0.1.4 proposals these notes used to carry):
key interning (20 unique names own 124 B, was 4.09 MiB of key heap), string
value interning (6.88 -> 3.68 MiB), flat entry vectors for narrow objects (no
box, no index table: 72 B Map + 96 B entry + tables became 24 B box + 16 B
entry), and 8 B value slots replacing 72 B `Value`s.

## Limitations

- Windows MSVC sizes only (IValue/IString are pointer words, 8 B; the report
  computes all constants from `size_of` at runtime, so numbers on other
  platforms are self-consistent).
- Counting allocator counts requested bytes: allocator headers/fragmentation
  show up only in RSS (RSS is typically 10-25% above live bytes).
- The walk counts key/string bytes per occurrence; ijson interns them, so the
  model overstates text and understates allocator overhead (net ~25% gap).
- XL single-file runs measure the first 500k records (default `--max-records`,
  per source); use `--max-records 99999999` to pay the full parse.
- Peak live is racy under parallel parsing (rayon); live at quiescent
  checkpoints is exact.
- ijson's intern table is a per-process cache: dropping all records leaves a
  0-7 KiB residue (measured), negligible next to the payload.
