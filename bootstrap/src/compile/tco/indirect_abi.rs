//! `info codegen indirect-abi <fn> <file>` — Class-P indirect-parameter lowering
//! view (ADR 1.7.26e §2.1/§2.2).
//!
//! Shows, per source parameter, how it is lowered into the internal `$direct_mt`
//! entry — **by-value** (scalar / `ptr` / recursive-ADT), **decomposed** into
//! scalar fields (18.5.26a), **indirect** via a caller-owned buffer `ptr`
//! (1.7.26e), or a **word** — a blob of at most 8 bytes carried as one integer
//! (2.10.26h) — plus the resulting slot layout (`[sret]? indirect… env after-env…`).
//! Complements `musttail-eligibility` (which shows the *lowered signature* but not
//! the per-param decision) and `doctor check codegen tco-coverage`.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;

use albite_codegen::{MusttailDecision, ParamAbiKind};

use super::collect::collect_musttail_decisions;

/// Entry point for `albite info codegen indirect-abi <fn> <file>`.
pub(crate) fn cmd_indirect_abi(
    fn_name: &str,
    file: &PathBuf,
    verbose: bool,
    max_errors: usize,
) -> ExitCode {
    let run = match collect_musttail_decisions(file, verbose, max_errors) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    // The `$direct_mt` declaration records carry the per-param lowering. A plain
    // EMIT/SKIP function has none.
    let matches: Vec<&MusttailDecision> = run
        .decisions
        .iter()
        .filter(|d| d.base_name() == fn_name && !d.param_abi.is_empty())
        .collect();

    if matches.is_empty() {
        let exists = run.decisions.iter().any(|d| d.base_name() == fn_name);
        if exists {
            println!(
                "{fn_name}: no indirect/decomposed lowering — all params are by-value \
                 (musttails directly, or is a plain SKIP). See `musttail-eligibility`."
            );
            return ExitCode::SUCCESS;
        }
        eprintln!(
            "no musttail-lowered entry found for '{fn_name}'. It may be non-recursive, \
             not tail-recursive, or not present in this file."
        );
        return ExitCode::FAILURE;
    }

    print!("{}", render_blocks(fn_name, &matches));
    ExitCode::SUCCESS
}

/// Render one block per specialization: per-param ABI + the slot layout.
fn render_blocks(fn_name: &str, matches: &[&MusttailDecision]) -> String {
    let multi = matches.len() > 1;
    let mut out = String::new();
    for (i, d) in matches.iter().enumerate() {
        let label = if multi {
            format!("{fn_name}  (specialization {} of {})", i + 1, matches.len())
        } else {
            fn_name.to_string()
        };
        let _ = writeln!(out, "{label}  —  indirect-ABI lowering");
        let _ = writeln!(out, "  lowered $direct_mt signature: {}", d.lowered_sig);
        let _ = writeln!(out, "  sret return out-pointer: {}", yes_no(d.sret));
        if let Some(bits) = d.word_ret {
            let _ = writeln!(out, "  word return: i{bits}");
        }
        out.push_str("  parameters (source order):\n");
        for (p, kind) in d.param_abi.iter().enumerate() {
            let _ = writeln!(out, "    param {p}: {}", kind.human());
        }
        let _ = writeln!(out, "  slot layout: {}", slot_layout(d));
        // The ABI attributes the declaration, every call site, and the musttail
        // self-edge all carry — answering "does slot k carry noalias?" without
        // a `compile --emit-llvm` + grep round-trip (ADR 17.7.26e).
        if !d.slot_attrs.is_empty() {
            out.push_str("  slot ABI attributes:\n");
            for (s, attrs) in d.slot_attrs.iter().enumerate() {
                let _ = writeln!(out, "    slot {s}: {attrs}");
            }
        }
        out.push('\n');
    }
    out
}

fn yes_no(b: bool) -> &'static str {
    if b {
        "yes"
    } else {
        "no"
    }
}

/// A compact slot-order description matching `plan_mt_slots`:
/// `[sret | ret:i<bits>]? indirect… env after-env…`. A word return takes no
/// slot; it is shown where `sret` would be so the two never both appear, and
/// a word param renders as `p<k>:i<bits>` after `env` (ADR 2.10.26h §2.4).
fn slot_layout(d: &MusttailDecision) -> String {
    let mut slots: Vec<String> = Vec::new();
    if d.sret {
        slots.push("sret".to_string());
    }
    if let Some(bits) = d.word_ret {
        slots.push(format!("ret:i{bits}"));
    }
    for (p, kind) in d.param_abi.iter().enumerate() {
        if matches!(kind, ParamAbiKind::Indirect) {
            slots.push(format!("p{p}→buf"));
        }
    }
    slots.push("env".to_string());
    for (p, kind) in d.param_abi.iter().enumerate() {
        match kind {
            ParamAbiKind::Decomposed(n) => slots.push(format!("p{p}×{n}")),
            ParamAbiKind::ByValue => slots.push(format!("p{p}")),
            ParamAbiKind::Word(bits) => slots.push(format!("p{p}:i{bits}")),
            ParamAbiKind::Indirect => {}
        }
    }
    slots.join("  ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use albite_codegen::Decision;

    fn mt(name: &str, param_abi: Vec<ParamAbiKind>, sret: bool, sig: &str) -> MusttailDecision {
        mt_with_attrs(name, param_abi, sret, sig, Vec::new())
    }

    fn mt_with_attrs(
        name: &str,
        param_abi: Vec<ParamAbiKind>,
        sret: bool,
        sig: &str,
        slot_attrs: Vec<String>,
    ) -> MusttailDecision {
        MusttailDecision {
            function: format!("{name}$direct_mt"),
            decision: Decision::Decompose,
            reasons: Vec::new(),
            blockers: Vec::new(),
            lowered_sig: sig.to_string(),
            param_abi,
            sret,
            word_ret: None,
            slot_attrs,
        }
    }

    /// ADR 2.10.26h §2.4: a word param renders as `p<k>:i<bits>` after `env`
    /// and a word return as `ret:i<bits>` where `sret` would sit.
    #[test]
    fn word_param_and_word_return_render() {
        let mut d = mt(
            "pick",
            vec![ParamAbiKind::Word(32), ParamAbiKind::ByValue],
            false,
            "i32(ptr, i32, i64)",
        );
        d.word_ret = Some(32);
        let out = render_blocks("pick", &[&d]);
        assert!(out.contains("param 0: word (i32)"), "got: {out}");
        assert!(out.contains("word return: i32"), "got: {out}");
        assert!(
            out.contains("slot layout: ret:i32  env  p0:i32  p1"),
            "got: {out}"
        );
    }

    #[test]
    fn indirect_and_passthrough_render() {
        // spin(ctx: nested, n: Nat): ctx indirect, n by-value, scalar return.
        let d = mt(
            "spin",
            vec![ParamAbiKind::Indirect, ParamAbiKind::ByValue],
            false,
            "i64(ptr, ptr, i64)",
        );
        let out = render_blocks("spin", &[&d]);
        assert!(out.contains("param 0: indirect"));
        assert!(out.contains("param 1: by-value"));
        assert!(out.contains("sret return out-pointer: no"));
        // Slot layout: indirect buffer leads, then env, then the by-value scalar.
        assert!(out.contains("slot layout: p0→buf  env  p1"), "got: {out}");
    }

    #[test]
    fn sret_plus_indirect_layout() {
        let d = mt(
            "spin_ret",
            vec![ParamAbiKind::Indirect],
            true,
            "void(ptr, ptr, ptr)",
        );
        let out = render_blocks("spin_ret", &[&d]);
        assert!(out.contains("sret return out-pointer: yes"));
        assert!(out.contains("slot layout: sret  p0→buf  env"), "got: {out}");
    }

    #[test]
    fn mixed_decompose_indirect_layout() {
        // sret + indirect + flattenable(2) + by-value.
        let d = mt(
            "mixed",
            vec![
                ParamAbiKind::Indirect,
                ParamAbiKind::Decomposed(2),
                ParamAbiKind::ByValue,
            ],
            true,
            "void(ptr, ptr, ptr, i64, i64, i64)",
        );
        let out = render_blocks("mixed", &[&d]);
        assert!(out.contains("param 1: decomposed (2 scalars)"));
        assert!(
            out.contains("slot layout: sret  p0→buf  env  p1×2  p2"),
            "got: {out}"
        );
    }

    /// ADR 17.7.26e: the per-slot ABI attributes are reported, so "does the
    /// indirect-param slot carry `noalias`?" is answerable without emitting IR.
    #[test]
    fn slot_abi_attributes_are_reported() {
        let d = mt_with_attrs(
            "spin_ret",
            vec![ParamAbiKind::Indirect],
            true,
            "void(ptr, ptr, ptr)",
            vec![
                "sret: noalias nonnull align 8 dereferenceable(24)".to_string(),
                "indirect-param: noalias nonnull align 8 dereferenceable(24)".to_string(),
                "env: (none)".to_string(),
            ],
        );
        let out = render_blocks("spin_ret", &[&d]);
        assert!(out.contains("slot ABI attributes:"), "got: {out}");
        assert!(
            out.contains("slot 1: indirect-param: noalias nonnull align 8 dereferenceable(24)"),
            "got: {out}"
        );
        assert!(out.contains("slot 2: env: (none)"), "got: {out}");
    }

    /// A plain EMIT/SKIP entry has no lowered slot descriptor; the section is
    /// omitted rather than rendered empty.
    #[test]
    fn absent_slot_attributes_omit_the_section() {
        let d = mt("spin", vec![ParamAbiKind::Indirect], false, "i64(ptr, ptr)");
        let out = render_blocks("spin", &[&d]);
        assert!(!out.contains("slot ABI attributes"), "got: {out}");
    }
}
