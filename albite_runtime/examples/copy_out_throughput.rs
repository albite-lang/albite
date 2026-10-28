//! ADR 13.10.26q D7: the copy-out's cost, G3's deferral.
//!
//! Builds a `List<Nat>` of `n` nodes in a region (each node the 20-byte
//! `{ i32, [16 x i8] }` blob the code generator folds), detaches it, and times
//! `copy_out_with` moving it into `malloc(3)` — the walk, the forwarding table
//! and the allocations, without process start-up. Each size is timed as
//! **9 runs** and reported as the median nanoseconds per copied block.
//! Run with
//! `cargo run --release -p albite_runtime --example copy_out_throughput`;
//! the record is `benchmarks/results/13.10.26q.region-copy-out-rss.md`.

#![allow(unsafe_code)] // Reason: the probe builds raw region blocks and drives the copier, as the crate it measures does (ADR 30.9.26l D3)

use std::time::Instant;

use albite_runtime::copy_out::{
    copy_out_with, ChunkIndex, KIND_BOX, KIND_STRUCT, KIND_TAGGED, NO_NODE,
};
use albite_runtime::{RegionStack, REGION_CHUNK_BYTES};

const RUNS: usize = 9;
const SIZES: [u64; 3] = [1_000, 100_000, 1_000_000];

/// `List<Nat>`'s layout, as `copy_layout` encodes it.
const LIST_LAYOUT: [u64; 15] = [
    15,
    KIND_BOX,
    20,
    4,
    KIND_TAGGED,
    4,
    2,
    0,
    NO_NODE,
    16,
    11,
    KIND_STRUCT,
    1,
    8,
    1,
];

static NIL: [u32; 5] = [0; 5];

fn malloc(size: usize) -> *mut u8 {
    // SAFETY: `malloc` contract; the probe leaks what it copies, as a
    // program's parent allocator would hold it.
    unsafe { libc::malloc(size) }.cast::<u8>()
}

/// Nanoseconds per block for one copy of an `n`-node list.
fn one_run(n: u64) -> f64 {
    let mut regions = RegionStack::new();
    regions.enter();
    let mut head = NIL.as_ptr().cast::<u8>().cast_mut();
    for value in 0..n {
        let block = regions
            .alloc_in_top(20, 16, REGION_CHUNK_BYTES, &mut malloc)
            .expect("a region is entered");
        // SAFETY: a fresh 20-byte block.
        unsafe {
            block.cast::<u32>().write_unaligned(1);
            block.add(4).cast::<u64>().write_unaligned(value);
            block.add(12).cast::<*mut u8>().write_unaligned(head);
        }
        head = block;
    }
    regions.detach().expect("entered");
    let index = ChunkIndex::new(regions.detached_chunk_spans());
    let mut slot = [head as u64];
    let start = Instant::now();
    // SAFETY: `slot` holds a list built to `LIST_LAYOUT`.
    let tally = unsafe {
        copy_out_with(
            slot.as_mut_ptr().cast(),
            &LIST_LAYOUT,
            &|addr| index.contains(addr),
            &mut malloc,
        )
    }
    .expect("well-formed");
    let elapsed = start.elapsed().as_nanos() as f64;
    assert_eq!(tally.copied as u64, n);
    regions
        .release(&mut |base, _| {
            // SAFETY: every chunk came from `malloc`.
            unsafe { libc::free(base.cast()) }
        })
        .expect("detached");
    elapsed / n as f64
}

fn main() {
    println!("| nodes | ns per copied block (median of {RUNS}) |");
    println!("|---:|---:|");
    for n in SIZES {
        let mut samples: Vec<f64> = (0..RUNS).map(|_| one_run(n)).collect();
        samples.sort_by(f64::total_cmp);
        println!("| {n} | {:.1} |", samples[RUNS / 2]);
    }
}
