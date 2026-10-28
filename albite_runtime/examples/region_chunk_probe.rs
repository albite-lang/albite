//! ADR 7.10.26s D5: the region chunk-size probe.
//!
//! One *cycle* is enter → `k` allocations of 48 bytes (14.9.26b AC 1's
//! 46.6-byte mean request, rounded to the block alignment) → detach →
//! release, through the pure `RegionStack` over `malloc(3)`/`free(3)` — the
//! chunk source and release the runtime's exports bind. Each allocation is
//! written to, so a chunk's pages fault in as a real region's would.
//!
//! Every cell (`k` × chunk size) is timed as **N = 40 runs**, each run a batch
//! of cycles, and reported as the median per-cycle time with its
//! interquartile range. The runs repeat in-process rather than under
//! `hyperfine`: at `k` = 1 a cycle is tens of nanoseconds, which process
//! start-up would swamp. A `chunk-only` row times a bare chunk `malloc` +
//! touch + `free` per size — what D5's one lever, a per-thread cache of one
//! released chunk, would save — against the `k` = 1 cycle.
//!
//! The rule (D5): the smallest size whose `k` = 4096 median is within 5% of
//! the best; two sizes whose medians differ by less than either's IQR tie, and
//! a tie goes to the smaller. Run with
//! `cargo run --release -p albite_runtime --example region_chunk_probe`;
//! the record is `benchmarks/results/7.10.26s.region-runtime-cost.md`.

#![allow(unsafe_code)] // Reason: the probe drives malloc/free and raw region blocks, as the crate it measures does (ADR 30.9.26l D3)

use std::hint::black_box;
use std::time::Instant;

use albite_runtime::RegionStack;

const REQUEST_BYTES: usize = 48;
const BLOCK_ALIGN: usize = 16;
const RUNS: usize = 40;
const KS: [usize; 3] = [1, 64, 4096];
const CHUNK_SIZES: [usize; 5] = [16 << 10, 64 << 10, 256 << 10, 1 << 20, 4 << 20];

fn chunk_source(size: usize) -> *mut u8 {
    // SAFETY: `malloc` contract; released by `chunk_release`.
    unsafe { libc::malloc(size) }.cast::<u8>()
}

fn chunk_release(base: *mut u8, _len: usize) {
    // SAFETY: every chunk came from `chunk_source`.
    unsafe { libc::free(base.cast()) };
}

/// One enter → `k` allocations → detach → release cycle.
fn region_cycle(stack: &mut RegionStack, k: usize, chunk_bytes: usize) {
    stack.enter();
    for i in 0..k {
        let block = stack
            .alloc_in_top(REQUEST_BYTES, BLOCK_ALIGN, chunk_bytes, &mut chunk_source)
            .expect("a region is entered");
        assert!(!block.is_null(), "out of memory");
        // SAFETY: a fresh 48-byte block.
        #[allow(clippy::cast_possible_truncation)] // Reason: a marker byte; wrapping is intended
        unsafe {
            black_box(block).write(i as u8);
        }
    }
    stack.detach().expect("detach");
    stack.release(&mut chunk_release).expect("release");
}

/// One bare chunk `malloc` + first-byte touch + `free`.
fn chunk_cycle(chunk_bytes: usize) {
    let chunk = chunk_source(chunk_bytes);
    assert!(!chunk.is_null(), "out of memory");
    // SAFETY: a fresh chunk of at least one byte.
    unsafe { black_box(chunk).write(1) };
    chunk_release(chunk, chunk_bytes);
}

/// Cycles per run: enough that a run lasts milliseconds at every `k`.
fn cycles_for(k: usize) -> usize {
    (200_000 / k).max(200)
}

/// Median and interquartile range, in nanoseconds per cycle, of `RUNS` runs
/// of `cycles` calls to `cycle`.
fn measure(cycles: usize, mut cycle: impl FnMut()) -> (f64, f64) {
    let mut per_cycle: Vec<f64> = (0..RUNS)
        .map(|_| {
            let start = Instant::now();
            for _ in 0..cycles {
                cycle();
            }
            #[allow(clippy::cast_precision_loss)] // Reason: nanosecond timings, far below 2^52
            let ns = start.elapsed().as_nanos() as f64 / cycles as f64;
            ns
        })
        .collect();
    per_cycle.sort_by(f64::total_cmp);
    let quartile = |q: usize| per_cycle[(RUNS - 1) * q / 4];
    (quartile(2), quartile(3) - quartile(1))
}

/// D5's rule over `(size, median, iqr)` rows for `k` = 4096.
fn choose(rows: &[(usize, f64, f64)]) -> usize {
    let best = rows.iter().map(|r| r.1).fold(f64::INFINITY, f64::min);
    let best_iqr = rows.iter().find(|r| r.1 == best).map_or(0.0, |r| r.2);
    rows.iter()
        .find(|(_, median, iqr)| {
            *median <= best * 1.05 || (median - best).abs() < iqr.max(best_iqr)
        })
        .map_or(rows[0].0, |r| r.0)
}

fn main() {
    let mut stack = RegionStack::new();
    println!("| k | chunk | median ns/cycle | IQR ns |");
    println!("|---:|---:|---:|---:|");
    let mut k4096 = Vec::new();
    let mut k1 = Vec::new();
    for k in KS {
        for chunk in CHUNK_SIZES {
            let (median, iqr) = measure(cycles_for(k), || region_cycle(&mut stack, k, chunk));
            println!("| {k} | {} KiB | {median:.1} | {iqr:.1} |", chunk >> 10);
            if k == 4096 {
                k4096.push((chunk, median, iqr));
            }
            if k == 1 {
                k1.push((chunk, median));
            }
        }
    }
    println!();
    println!("| chunk | malloc+free median ns | IQR ns | k=1 cycle ns | > half? |");
    println!("|---:|---:|---:|---:|:---:|");
    for (chunk, cycle_median) in &k1 {
        let (median, iqr) = measure(cycles_for(1), || chunk_cycle(*chunk));
        let over = if median > cycle_median / 2.0 {
            "yes"
        } else {
            "no"
        };
        println!(
            "| {} KiB | {median:.1} | {iqr:.1} | {cycle_median:.1} | {over} |",
            chunk >> 10
        );
    }
    println!();
    println!("REGION_CHUNK_BYTES = {} KiB", choose(&k4096) >> 10);
}
