//! The pairing table itself, plus the probes its rows name (ADR 13.8.26c §2.4).
//!
//! Data, separated from the engine that runs it: `mod.rs` holds `evaluate`,
//! `render` and `verdict`, which are the same code whatever the table says.
//! The rows change every time a gate does, and this is the file to edit when
//! one has.

use std::path::Path;

use crate::driver;
use crate::elaborate::termination::ReportingOnly;

use super::{CompanionPairing, FixtureFile, Reachability};

/// Probe for a companion that drives elaboration under forced `Report`.
fn probe_under_reporting_only(file: &Path) -> Reachability {
    let _reporting = ReportingOnly::begin();
    probe_plain(file)
}

/// Probe for a companion that elaborates at whatever the run's level is.
fn probe_plain(file: &Path) -> Reachability {
    match driver::elaborate_project(file, false, 0, None) {
        Ok(_) => Reachability::Reachable,
        Err(_) => Reachability::BlockedByGate,
    }
}

/// Probe for a companion that never elaborates: parse the module tree, build
/// the module info, and census it.
///
/// Every other probe here bottoms out in `elaborate_project`, which returns
/// `BlockedByGate` for a rejected file — so reusing one would assert the exact
/// opposite of this companion's intent (ADR 13.8.26c §2.4).
///
/// `Reachable` demands the verdict, not merely the code path: a census that ran
/// and found nothing on a fixture built to collide has not reported on the
/// rejected input, and that is the failure mode the check's own §5 warns about
/// — a multimap built over the wrong field reports 0 and looks correct.
pub(super) fn probe_parse_only_census(file: &Path) -> Reachability {
    use crate::doctor::checks::check_name_collisions::{census::ReexportHandling, census_of_file};

    match census_of_file(file, ReexportHandling::Subtract) {
        Some(census) if !census.collisions.is_empty() => Reachability::Reachable,
        _ => Reachability::BlockedByGate,
    }
}

/// Every failure mode whose companion diagnostic this check holds to its
/// promise.
pub const PAIRINGS: &[CompanionPairing] = &[
    CompanionPairing {
        failure_mode: "E0062 termination (default enforcement `all`)",
        tool: "albite doctor check type termination",
        // `spin` recurses on its whole argument, so it can never be certified.
        fixture: &[FixtureFile {
            path: "reachability.tg",
            source: "type Lst = Nil | Cons(Nat, Lst)\n\
                  fn spin(l: Lst) -> Nat { spin(l) }\n\
                  fn main() -> Nat { 0 }\n",
        }],
        expected: Reachability::Reachable,
        probe: probe_under_reporting_only,
    },
    CompanionPairing {
        failure_mode: "E0062 termination (default enforcement `all`)",
        tool: "albite info def <name> <file> --why-not-certified",
        // The same fixture: this flag exists to explain an E0062 rejection, so
        // a file with one is the only input it is for.
        fixture: &[FixtureFile {
            path: "reachability.tg",
            source: "type Lst = Nil | Cons(Nat, Lst)\n\
                  fn spin(l: Lst) -> Nat { spin(l) }\n\
                  fn main() -> Nat { 0 }\n",
        }],
        // Added because ADR 12.8.26a's own review found this command blocked by
        // the gate it was written to explain. `info` forces `Report` for the
        // whole namespace now; this row is what keeps that true.
        expected: Reachability::Reachable,
        probe: probe_under_reporting_only,
    },
    CompanionPairing {
        failure_mode: "E0061 strict positivity (unconditional)",
        tool: "albite doctor check type positivity",
        // `Bad` occurs left of an arrow, which E0061 rejects outright.
        fixture: &[FixtureFile {
            path: "reachability.tg",
            source: "type Bad = Mk(Bad -> Nat)\n\
                  fn main() -> Nat { 0 }\n",
        }],
        // Honest, not aspirational: E0061 has no enforcement knob, so the tool
        // genuinely cannot report on a file that fails it. The surfaces say so
        // too (`.claude/CLAUDE.md` § Elaboration Pipeline). If someone gives
        // E0061 a report level, this row must flip — which is the point.
        expected: Reachability::BlockedByGate,
        probe: probe_plain,
    },
    CompanionPairing {
        failure_mode: "E0064 nested inductive family (unconditional)",
        tool: "albite info type type-encoding <T>",
        // The §1.1 reproducer of ADR 11.8.26c. Note the *types* alone check
        // clean — it is the `match` that is rejected — so the fixture must
        // carry the match to be an input this gate refuses at all.
        fixture: &[FixtureFile {
            path: "reachability.tg",
            source: "type Wrap<T> = W(T)\n\
                  type Rose = Node(Wrap<Rose>)\n\
                  fn depth(r: Rose) -> Nat {\n\
                      match r { Node(w) => match w { W(inner) => 1 + depth(inner) } }\n\
                  }\n\
                  fn main() -> Nat { 0 }\n",
        }],
        // Honest, not aspirational, on the E0061 pattern. E0064 has no
        // enforcement knob, so the encoding inspector genuinely cannot run on
        // a file that fails it — which is why the E0064 *message* names the
        // binder itself rather than deferring to the tool. This row is what
        // stops the explain text drifting back to "inspect it with …".
        expected: Reachability::BlockedByGate,
        probe: probe_plain,
    },
    CompanionPairing {
        failure_mode: "E0064 nested inductive family (unconditional)",
        tool: "albite doctor check type vacuous-mu",
        // Same reproducer as the row above: the `match` is what E0064 refuses.
        fixture: &[FixtureFile {
            path: "reachability.tg",
            source: "type Wrap<T> = W(T)\n\
                  type Rose = Node(Wrap<Rose>)\n\
                  fn depth(r: Rose) -> Nat {\n\
                      match r { Node(w) => match w { W(inner) => 1 + depth(inner) } }\n\
                  }\n\
                  fn main() -> Nat { 0 }\n",
        }],
        // This row exists to stop a plausible-sounding lie. `vacuous-mu` finds
        // the shape E0064 rejects, so it reads like the thing to run *after*
        // hitting one — and it is not: it elaborates, so the gate blocks it on
        // exactly those files. Its value is entirely PRE-`match`, on a corpus
        // that still compiles, and every surface says so. Without a row here,
        // nothing would stop the next surface saying otherwise.
        expected: Reachability::BlockedByGate,
        probe: probe_plain,
    },
    CompanionPairing {
        failure_mode: "E0019 ambiguous bare value (ordinary elaboration error)",
        tool: "albite doctor check module name-collisions",
        // Two modules define `pub fn shared_helper` and `main` calls it bare,
        // so the elaborator refuses the call as ambiguous (ADR 4.10.26j D3) —
        // a file it rejects, the state in which anyone asks this tool which
        // modules define the name. (Until 4.10.26j the shape was two private
        // helpers and an E0016; those now elaborate, each calling its own.)
        fixture: &[
            FixtureFile {
                path: "reachability.tg",
                source: "mod alpha;\n\
                         mod beta;\n\
                         fn main() -> Nat { shared_helper() }\n",
            },
            FixtureFile {
                path: "alpha.tg",
                source: "pub fn shared_helper() -> Nat { 1 }\n",
            },
            FixtureFile {
                path: "beta.tg",
                source: "pub fn shared_helper() -> Nat { 2 }\n",
            },
        ],
        // Reachable by construction, and this row is what keeps it so: the
        // check is parse-only precisely so that making it elaborate — for a
        // visibility fact, say, or a resolved call target — would show up here
        // rather than as a tool that silently stopped working on every input
        // it was written for.
        expected: Reachability::Reachable,
        probe: probe_parse_only_census,
    },
    CompanionPairing {
        failure_mode: "E0081 kernel refusal of a tactic proof (unconditional)",
        tool: "albite doctor check proof kernel-census",
        // A shape the elaborator accepts and the kernel refuses. It was the δ
        // golden's (ADR 2.10.26c D10) until the kernel unfolded `two` (ADR
        // 2.10.26l). Since 3.10.26p a record reaches the kernel by its
        // encoding, so this one is self-recursive — a cycle left named (D3).
        fixture: &[FixtureFile {
            path: "reachability.tg",
            source: "type Point = { x: Nat, next: Link }\n\
                  type Link = End | More(Point)\n\
                  fn px(p: Point) -> Nat { p.x }\n\
                  theorem px_refl(p: Point): px(p) == px(p) by { refl }\n\
                  fn main() -> Nat { 0 }\n",
        }],
        // Honest, on the E0061 pattern. The census elaborates through the same
        // walk, under `ReportingOnly` as here, and a kernel refusal is gated at
        // every level (2.10.26c D5), so the census cannot report on a file the
        // enforcement refuses. E0081's message carries the kernel's own error
        // instead, and its `explain` entry says so. If a kernel refusal ever
        // gains a report level, this row must flip.
        expected: Reachability::BlockedByGate,
        probe: probe_under_reporting_only,
    },
    CompanionPairing {
        failure_mode: "E0081 kernel refusal of a term-mode theorem (ADR 2.10.26d)",
        tool: "albite doctor check proof kernel-census",
        // The record theorem's term-mode shape, with no register row.
        fixture: &[FixtureFile {
            path: "reachability.tg",
            source: "type Point = { x: Nat, next: Link }\n\
                  type Link = End | More(Point)\n\
                  fn px(p: Point) -> Nat { p.x }\n\
                  theorem px_refl(p: Point): px(p) == px(p) { refl }\n\
                  fn main() -> Nat { 0 }\n",
        }],
        // Unlike the tactic row above: the term-mode gate reads the census's
        // reporting window and reports nothing under it (2.10.26d D2), so the
        // census reaches the very file the build refuses and prints its row.
        expected: Reachability::Reachable,
        probe: probe_under_reporting_only,
    },
];
