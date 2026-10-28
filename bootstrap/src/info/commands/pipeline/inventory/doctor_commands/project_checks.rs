//! The `doctor check` leaves outside the `type` and `ir` sub-namespaces, for
//! the `info pipeline` inventory (split from its parent at the size cap).

use super::{CostTier, PipelineEntry};

/// The `doctor check` leaves outside the `type` and `ir` sub-namespaces.
pub const DOCTOR_PROJECT_CHECKS: &[PipelineEntry] = &[
    PipelineEntry::subcommand(
        "doctor check module reexport-completeness",
        "albite doctor check module reexport-completeness <file>",
        "Check pub use re-export completeness",
    )
    .with_cost(CostTier::Elaborate)
    // The dual, pointed back at (ADR 29.8.26a AC 5): `name-collisions` already
    // named this one, and a one-way cross-reference is how a reader who arrives
    // at the wrong half of a pair stays there.
    .with_see_also(&["doctor check module name-collisions"]),
    PipelineEntry::subcommand(
        "doctor check module name-collisions",
        "albite doctor check module name-collisions <file> [--severity all|live] [--json]",
        "Report value names defined in >1 reachable module. Both compilers key\n\
         values on the BARE NAME, so the walk's last registration wins and the\n\
         loser's call sites report E0016 in THEIR files naming the winner's\n\
         module. Names both modules and marks the winner. Parse-only, so it\n\
         runs on a file the elaborator REJECTS. Advisory: exit 0 even with\n\
         findings (ADR 13.8.26c)",
    )
    .with_cost(CostTier::Parse)
    .with_see_also(&["doctor check module reexport-completeness"]),
    PipelineEntry::subcommand(
        "doctor check codegen tco-coverage",
        "albite doctor check codegen tco-coverage <file> [--gate]",
        "Rank self-recursive fns by O(N)-stack risk from the actual musttail\n\
         gate; --gate is the deterministic CI form (make check-tco-gate),\n\
         exit≠0 on an un-allowlisted HIGH-risk SKIP. Self-recursive ONLY:\n\
         non-self tail edges are counted as SKIP_NON_SELF, never ranked or\n\
         gated (ADRs 1.7.26b, 1.7.26e, 5.8.26a)",
    )
    .with_cost(CostTier::Compile)
    .requiring_codegen()
    .with_see_also(&["info codegen musttail-eligibility"]),
    PipelineEntry::subcommand(
        "doctor check codegen unit-cost",
        "albite doctor check codegen unit-cost <file> [--threshold 0.5s|8GB] [--json]",
        "Ranked per-unit codegen cost census: wall time + alloc volume;\n\
         threshold gates the exit code. --emit-serial-list prints the\n\
         ALBITE_CODEGEN_SERIAL_UNITS value for units at/above it\n\
         (ADR 8.7.26a)",
    )
    .with_cost(CostTier::Compile)
    .requiring_codegen(),
    PipelineEntry::subcommand(
        "doctor check codegen mono-coverage",
        "albite doctor check codegen mono-coverage <file>",
        "Verify all TyApp sites have mono owners",
    )
    .with_cost(CostTier::Compile)
    .requiring_codegen(),
    PipelineEntry::subcommand(
        "doctor check codegen extern-map-ambiguity",
        "albite doctor check codegen extern-map-ambiguity <file> [--json]",
        "Detect calls silently bound to the wrong same-named def via the\n\
         clobber-last extern map; exit≠0 on findings (ADR 12.7.26b)",
    )
    .with_cost(CostTier::Compile)
    .requiring_codegen(),
    // Absent from the hand-written inventory for as long as it has existed —
    // the live drift instance ADR 28.7.26f §1.1 was written around, and the
    // first thing the completeness test catches.
    PipelineEntry::subcommand(
        "doctor check link collisions",
        "albite doctor check link collisions <dir>",
        "Run `nm -g` over a directory of .o files and report duplicate defined\n\
         text symbols — the duplicate-symbol link failure, before the linker\n\
         (ADR 6.5.26d §2.5). suggest-tools names this one for 'referenced but\n\
         not declared'",
    )
    .with_cost(CostTier::Instant)
    .requiring_codegen(),
    PipelineEntry::subcommand(
        "doctor check module overlap",
        "albite doctor check module overlap",
        "Detect foo.rs + foo/mod.rs coexistence (E0761)",
    )
    .with_cost(CostTier::Instant),
    PipelineEntry::subcommand(
        "doctor check module signature-collection",
        "albite doctor check module signature-collection <file>",
        "Check Signature Collection global collection health",
    )
    .with_cost(CostTier::Elaborate)
    // A signature-collection fault is usually a bad import in ANOTHER module,
    // which is where the re-export check looks.
    .with_see_also(&["doctor check module reexport-completeness"]),
    PipelineEntry::subcommand(
        "doctor check link health",
        "albite doctor check link health <binary>",
        "Verify a compiled binary's body stack (Linux: runtime entry) + executability",
    )
    .with_cost(CostTier::Instant),
    PipelineEntry::subcommand(
        "doctor check link extern-symbols",
        "albite doctor check link extern-symbols <file>",
        "Report extern-C decls no albite_core symbol PROVIDES — each is an\n\
         undefined reference at link time, minutes into a self-compile, after a\n\
         clean check/test/check-health. Two source walks, no build needed;\n\
         --core-root overrides the scanned root. Exit 2 on findings, and on an\n\
         empty corpus, since `0 examined` must not read like `0 findings`.\n\
         A `tg_bytes_*` intrinsic is never linked, so it is listed, not a finding",
    )
    .with_cost(CostTier::Parse)
    .with_see_also(&["doctor check extern-coverage"]),
    PipelineEntry::subcommand(
        "doctor check link reserved-names",
        "albite doctor check link reserved-names <file>...",
        "Report C names an archive, object or binary IMPORTS that the reserved\n\
         link-name table lacks — a program definition so named would capture\n\
         the runtime's call at link time. Runs nm; exit 2 on findings, exit 1\n\
         when the inputs import nothing. make check-reserved-link-names runs it",
    )
    .with_cost(CostTier::Instant)
    .with_see_also(&["doctor check link extern-symbols"]),
    PipelineEntry::subcommand(
        "doctor check selfhost closed-terms",
        "albite doctor check selfhost closed-terms <file>",
        "Self-hosted Core terms with FREE value variables — a name the\n\
         elaborator resolved through its env and never bound. Type-checks and\n\
         compiles, so nothing else sees it. Spawns albite1; exit 2 unasked",
    )
    .with_cost(CostTier::Elaborate)
    .with_see_also(&[
        "diff selfhost-core",
        "doctor check type termination",
        "doctor check selfhost well-typed-terms",
    ]),
    PipelineEntry::subcommand(
        "doctor check selfhost well-typed-terms",
        "albite doctor check selfhost well-typed-terms <file>",
        "Self-hosted eliminators over the WRONG former — `fst` of a scalar,\n\
         `app` of a lambda's result. Closed, type-checks, compiles, so\n\
         closed-terms passes it. Shrink-only baseline over main.tg (300 of\n\
         2302); spawns albite1; exit 2 unasked (ADR 3.9.26h)",
    )
    .with_cost(CostTier::Elaborate)
    .with_see_also(&["doctor check selfhost closed-terms", "diff selfhost-core"]),
    PipelineEntry::subcommand(
        "doctor check proof kernel-census",
        "albite doctor check proof kernel-census <file> [--include-total] [--all-rows] | --corpus <root>… [--proof-bearing]",
        "Every theorem's elaborated Core through the kernel's proof judgment,\n\
         each refusal sorted by cause (hole, needs-conversion, dependent-subst,\n\
         marker, named type, ...). A census, not a gate: exit 0 whatever it\n\
         counts, exit 2 on an empty population (ADR 30.9.26f). `--corpus`\n\
         census-es every entry file under the roots, cold, and diffs each\n\
         term-mode theorem's gate verdict against the exemption register\n\
         (ADR 2.10.26o)",
    )
    .with_cost(CostTier::Elaborate)
    .with_see_also(&[
        "doctor check type termination",
        "doctor check sorry-sites",
        "info def",
    ]),
    PipelineEntry::subcommand(
        "doctor check proof term-size",
        "albite doctor check proof term-size <file> [--include-total | --all-definitions] [--top <n>] [--baseline <json> [--max-growth <r>] [--min-delta <n>]] [--json] [--json-out <path>]",
        "Every theorem — or with --all-definitions every definition — beside\n\
         the size of its Core term, from one cold elaboration: Subst, Refl and\n\
         Global-occurrence counts, a node total and case-arms (match width)\n\
         per definition; --top ranks the largest. --baseline joins a saved\n\
         --json run by (module, name) and ranks what grew. A census, not a\n\
         gate: exit 0 when it completes, exit 1 only under --baseline when a\n\
         row is flagged, exit 2 on an empty population, a refused baseline or\n\
         a file that does not elaborate (ADR 13.10.26c, 15.10.26g)",
    )
    .with_cost(CostTier::Elaborate)
    .with_see_also(&["doctor check proof kernel-census", "info def"]),
    PipelineEntry::subcommand(
        "doctor check self-compile-readiness",
        "albite doctor check self-compile-readiness",
        "Pre-flight checks for self-compile (filesystem, linker, LLVM)",
    )
    .with_cost(CostTier::Instant),
    PipelineEntry::subcommand(
        "doctor check nested-patterns",
        "albite doctor check nested-patterns <file>",
        "Detect nested constructor+tuple match patterns (ADR 20.5.26a)",
    )
    .with_cost(CostTier::Parse),
    PipelineEntry::subcommand(
        "doctor check sorry-sites",
        "albite doctor check sorry-sites <file|dir>",
        "Which definitions carry a proof hole, and who put it there? One row\n\
         per definition whose Core holds a Sorry: authored (file:line:col),\n\
         synthesised (absurd branch / unreachable pattern arm — a nested\n\
         pattern's lowering) and unclassified. The census behind `check`'s\n\
         `contains sorry`, with no codegen; exit 0 (ADR 18.9.26g). Then each\n\
         theorem resting on an axiom or sorry, with its roots (ADR 13.10.26g).\n\
         A directory censuses every .tg under it, one row per entry and\n\
         totals summed over entries; exit 1 if an entry did not elaborate,\n\
         2 if there is nothing to census (ADR 15.10.26a)",
    )
    .with_cost(CostTier::Elaborate)
    .with_see_also(&["doctor check nested-patterns", "info def"]),
    PipelineEntry::subcommand(
        "doctor check extern-coverage",
        "albite doctor check extern-coverage <file>",
        "Report extern-C decls the evaluator CANNOT execute — their calls go\n\
         silently Stuck on run/test/playground (native codegen unaffected);\n\
         exit 2 on findings (ADR 28.7.26a)",
    )
    .with_cost(CostTier::Parse)
    .with_see_also(&["info eval externs", "doctor check link extern-symbols"]),
    PipelineEntry::subcommand(
        "doctor check comparable",
        "albite doctor check comparable <type> <file>",
        "Can the structural comparator handle this type, and where does it break?
         Reports the gate's own verdict (opaque leaf / incomplete closure /
         unsettled synthesis), so it cannot disagree with what a run does.
         Since 1.8.26c a generic instantiation (List<T>) resolves rather than
         being an opaque leaf. --all checks every type in ONE elaboration; clean
         the cache first (record/adt types are uncached, so a cache hit reports
         everything noncomparable). ADRs 1.8.26b/1.8.26c; exit 1 findings, 2 bad input",
    )
    .with_cost(CostTier::Elaborate)
    .with_see_also(&["info type size", "info type mu-members"]),
    PipelineEntry::subcommand(
        "doctor map-span",
        "albite doctor map-span <file> <offset>",
        "Map byte offset to file:line:col",
    )
    .with_cost(CostTier::Instant),
    PipelineEntry::subcommand(
        "doctor suggest-tools",
        "albite doctor suggest-tools <desc>",
        "Suggest diagnostic tools for an error — start here when stuck",
    )
    .with_cost(CostTier::Instant),
];
