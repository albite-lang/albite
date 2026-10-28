//! The optimised concats write each new byte once (ADR 9.10.26n AC 4): over
//! the real embedded core after its `default<O2>` pipeline, neither join
//! fills its block, and each still copies its last operand through a
//! `getelementptr` at an offset — 7.10.26p's non-vacuity check, so a fill
//! that vanished because the copies did is not mistaken for D2.

use super::tests::{core_ir, defining_line, definition_body, memcpy_destination};
use albite_codegen::NATIVE_CORE_PREFIX;

#[test]
fn the_optimised_concats_fill_nothing_and_still_copy_at_an_offset() {
    let ir = core_ir(None);
    for (definition, copies) in [("string_concat", 2), ("string_concat_owned", 1)] {
        let symbol = format!("{NATIVE_CORE_PREFIX}{definition}$direct_mt");
        let body = definition_body(&ir, &symbol);
        assert!(!body.is_empty(), "{symbol} is defined:\n{ir}");
        let fills: Vec<&&str> = body.iter().filter(|line| line.contains("memset")).collect();
        assert!(fills.is_empty(), "{symbol} fills: {fills:?}");
        let destinations: Vec<&str> = body
            .iter()
            .copied()
            .filter_map(memcpy_destination)
            .collect();
        assert!(destinations.len() >= copies, "{symbol}: {destinations:?}");
        let last = destinations.last().expect("a copy");
        let defined = defining_line(&body, last).expect("the last copy's destination");
        assert!(defined.contains("getelementptr"), "{symbol}: {defined}");
    }
}
