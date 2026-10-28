//! The LLVM symbol set one `.tg` function compiles to (ADR 5.8.26b retrospective).
//!
//! Pure string logic, deliberately outside the `codegen` feature gate: the
//! naming convention is a property of the emitter's *rules*, not of a linked
//! LLVM, so it stays testable in the LLVM-free build and is reachable by the
//! coverage/mutation diff gates.
//!
//! **Why this exists.** `.claude/CLAUDE.md` long documented "each `.tg` function
//! compiles to two LLVM symbols". It is up to four kinds, and the omission has
//! a measurable consequence: attributing a `perf` profile to a source function
//! means summing self time across *all* of them, and a reader who knows only
//! `<name>` and `<name>$direct` silently under-counts. Measured during ADR
//! 5.8.26b, where the hot function's samples landed almost entirely in
//! `$direct_mt` — the one kind the docs never mentioned.

/// One symbol a source function compiles to, with the role that explains why
/// a profile might attribute samples to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSymbol {
    /// The mangled symbol as it appears in `nm` output and `perf report`.
    pub symbol: String,
    /// What this symbol is, in one line.
    pub role: &'static str,
}

/// The symbols derivable from a function's name alone.
///
/// Lambdas are NOT here: `<fn>_lambda_N` exists only for functions that
/// actually close over something, so its presence is a fact about the emitted
/// module rather than about the name. `info codegen symbols --by-function`
/// joins these with the emitted lambda map to give the complete set.
pub fn conventional_symbols(fn_name: &str) -> Vec<FunctionSymbol> {
    vec![
        FunctionSymbol {
            symbol: fn_name.to_string(),
            role: "closure-returning wrapper (allocates env, returns {fn_ptr, env_ptr})",
        },
        FunctionSymbol {
            symbol: format!("{fn_name}$direct"),
            role: "direct-call variant (env ptr + args) — the GDB breakpoint target",
        },
        FunctionSymbol {
            symbol: format!("{fn_name}$direct_mt"),
            role:
                "musttail/indirect-buffer callee — where a self-recursive hot loop's samples land",
        },
    ]
}

/// Does `symbol` belong to `fn_name`'s family?
///
/// Used to pick a function's rows out of a flat symbol list. Matching is on
/// the full name plus a known suffix, never a bare prefix: `foo` must not
/// claim `foobar`, and `foo$direct` must not claim `foo$direct_mt`'s samples
/// twice.
pub fn belongs_to(symbol: &str, fn_name: &str) -> bool {
    if symbol == fn_name {
        return true;
    }
    let Some(rest) = symbol.strip_prefix(fn_name) else {
        return false;
    };
    rest == "$direct" || rest == "$direct_mt" || is_lambda_suffix(rest)
}

/// `_lambda_1`, `_lambda_27`, … — and nothing else.
fn is_lambda_suffix(rest: &str) -> bool {
    rest.strip_prefix("_lambda_")
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// Does one emitted symbol-map row belong to `fn_name`?
///
/// A row qualifies two independent ways, and both are needed: the emitter
/// *recorded* it against this source function (`source_name`), or its IR name
/// is a member of the family by the naming convention ([`belongs_to`]). An
/// anonymous closure has no `source_name` but a conventional `ir_name`; a
/// renamed one is the reverse. Narrowing this to a conjunction silently empties
/// the lambda list, which reads as "this function has no closures" rather than
/// as a bug — which is why it is a named function with its own tests rather
/// than an inline predicate in a printer.
pub fn row_belongs_to(source_name: Option<&str>, ir_name: &str, fn_name: &str) -> bool {
    source_name == Some(fn_name) || belongs_to(ir_name, fn_name)
}

/// The Core names `--by-function <query>` lists (ADR 13.10.26a D2).
///
/// `defs` pairs each definition's Core name with its source name, as
/// `CoreNames` reports them — the two differ exactly when the Core name is
/// scoped. A query that IS a scoped Core name selects itself alone. Any other
/// query is a source name and selects every definition written under it,
/// sorted: two when two modules share it, one renamed `write`, or one bare
/// `main`. The table decides, never the spelling, so a user's own `_tg_3foo`
/// (Core name equal to its source name) is not a scoped `foo`. Empty means
/// the program defines nothing under that name.
pub fn resolve_by_function(query: &str, defs: &[(&str, &str)]) -> Vec<String> {
    let is_scoped_core_name = defs
        .iter()
        .any(|(core, source)| *core == query && core != source);
    if is_scoped_core_name {
        return vec![query.to_string()];
    }
    let mut selected: Vec<String> = defs
        .iter()
        .filter(|(_, source)| *source == query)
        .map(|(core, _)| (*core).to_string())
        .collect();
    selected.sort();
    selected.dedup();
    selected
}

/// Indices of the symbol-map rows — `(recorded source name, IR name)` — that
/// belong to `core_name` (ADR 13.10.26a D5). Run once per resolved Core name,
/// so two colliding definitions each get their own lambdas and share none.
pub fn owned_row_indices(rows: &[(Option<&str>, &str)], core_name: &str) -> Vec<usize> {
    rows.iter()
        .enumerate()
        .filter(|(_, (source, ir))| row_belongs_to(*source, ir, core_name))
        .map(|(index, _)| index)
        .collect()
}

/// The heading over one resolved symbol set (ADR 13.10.26a D3): when the
/// compiler spelt the queried name differently, say so, so the reader learns
/// the spelling the profile will show.
pub fn symbol_set_heading(query: &str, core_name: &str) -> String {
    if query == core_name {
        format!("Symbol set for `{core_name}`:")
    } else {
        format!("Symbol set for `{query}` — `{query}` is spelled `{core_name}`:")
    }
}

/// The answer for a query that selects nothing (ADR 13.10.26a D4): printing
/// the conventional rows for it would name symbols the program never defines.
pub fn no_definition_line(query: &str, file: &str) -> String {
    format!("no definition named `{query}` in {file}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_conventional_set_is_the_three_name_derived_symbols() {
        let syms = conventional_symbols("import_list_lookup");
        let names: Vec<_> = syms.iter().map(|s| s.symbol.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "import_list_lookup",
                "import_list_lookup$direct",
                "import_list_lookup$direct_mt",
            ]
        );
        // Every entry explains itself — an unlabelled symbol list is what the
        // docs already were.
        assert!(syms.iter().all(|s| !s.role.is_empty()));
    }

    /// `$direct_mt` must be listed. It is the kind CLAUDE.md omitted, and the
    /// kind that carried 56.94% of ADR 5.8.26b's profile.
    #[test]
    fn direct_mt_is_present_because_the_docs_used_to_omit_it() {
        let syms = conventional_symbols("f");
        assert!(syms.iter().any(|s| s.symbol == "f$direct_mt"));
    }

    #[test]
    fn membership_accepts_every_kind() {
        assert!(belongs_to("f", "f"));
        assert!(belongs_to("f$direct", "f"));
        assert!(belongs_to("f$direct_mt", "f"));
        assert!(belongs_to("f_lambda_1", "f"));
        assert!(belongs_to("f_lambda_27", "f"));
    }

    /// A bare prefix match would fold a different function's samples into this
    /// one's total — the exact way a profile silently lies.
    #[test]
    fn membership_rejects_a_longer_unrelated_name() {
        assert!(!belongs_to("foobar", "foo"));
        assert!(!belongs_to("foo_helper", "foo"));
        assert!(!belongs_to("foo$directly", "foo"));
        assert!(!belongs_to("foo_lambda_", "foo"));
        assert!(!belongs_to("foo_lambda_x", "foo"));
        assert!(!belongs_to("other", "foo"));
    }

    /// The reverse direction: a shorter name must not claim a longer one's
    /// family members either.
    #[test]
    fn membership_is_not_symmetric_across_different_functions() {
        assert!(!belongs_to("foo$direct", "foobar"));
        assert!(belongs_to("foobar$direct", "foobar"));
    }

    /// Each disjunct alone must qualify a row. Turning this into a conjunction
    /// empties the lambda list, which reads as "no closures here" rather than
    /// as a defect.
    #[test]
    fn a_row_qualifies_by_recorded_name_or_by_convention_independently() {
        // Recorded against the function, IR name unrecognisable on its own.
        assert!(row_belongs_to(Some("f"), "__anon_7", "f"));
        // Conventional IR name, nothing recorded.
        assert!(row_belongs_to(None, "f_lambda_2", "f"));
        // Both.
        assert!(row_belongs_to(Some("f"), "f_lambda_2", "f"));
    }

    /// A row belonging to a DIFFERENT function must not be pulled in — that
    /// would attribute another function's closures to this one.
    #[test]
    fn a_row_for_another_function_does_not_qualify() {
        assert!(!row_belongs_to(Some("g"), "g_lambda_1", "f"));
        assert!(!row_belongs_to(None, "__anon_7", "f"));
        // The equality is on the WHOLE recorded name, not a prefix of it.
        assert!(!row_belongs_to(Some("foobar"), "__anon_1", "foo"));
    }

    // 13.10.26a AC1 — a source-name query resolves through the table.
    #[test]
    fn a_source_name_query_resolves_to_its_scoped_core_name() {
        let defs = [("_tg_5write", "write"), ("main", "main")];
        assert_eq!(resolve_by_function("write", &defs), vec!["_tg_5write"]);
        assert_eq!(resolve_by_function("main", &defs), vec!["main"]);
    }

    // 13.10.26a AC1 — a collision resolves to every definer, sorted.
    #[test]
    fn a_colliding_source_name_resolves_to_both_definers_sorted() {
        let defs = [("_tg_1y_6helper", "helper"), ("_tg_1x_6helper", "helper")];
        assert_eq!(
            resolve_by_function("helper", &defs),
            vec!["_tg_1x_6helper", "_tg_1y_6helper"]
        );
    }

    // 13.10.26a AC1 — the table decides, not the spelling.
    #[test]
    fn a_user_spelt_scoped_looking_name_is_not_selected_by_its_decoding() {
        let defs = [("_tg_3foo", "_tg_3foo")];
        assert!(resolve_by_function("foo", &defs).is_empty());
        assert_eq!(resolve_by_function("_tg_3foo", &defs), vec!["_tg_3foo"]);
    }

    // 13.10.26a AC2 — a scoped Core name resolves to itself alone.
    #[test]
    fn a_scoped_core_name_query_resolves_to_itself_alone() {
        let defs = [("_tg_1x_6helper", "helper"), ("_tg_1y_6helper", "helper")];
        assert_eq!(
            resolve_by_function("_tg_1y_6helper", &defs),
            vec!["_tg_1y_6helper"]
        );
    }

    // 13.10.26a AC2 — an unknown name resolves to nothing.
    #[test]
    fn an_unknown_name_resolves_to_nothing() {
        let defs = [("_tg_5write", "write"), ("main", "main")];
        assert!(resolve_by_function("nosuch", &defs).is_empty());
        assert!(resolve_by_function("anything", &[]).is_empty());
    }

    /// One definition reached through two routes is still one set.
    #[test]
    fn a_duplicated_definition_resolves_once() {
        let defs = [("f", "f"), ("f", "f")];
        assert_eq!(resolve_by_function("f", &defs), vec!["f"]);
    }

    // 13.10.26a AC2 — lambda rows go to the Core name that owns them, none shared.
    #[test]
    fn colliding_definitions_each_own_their_lambdas_and_share_none() {
        let rows = [
            (Some("_tg_1x_6helper"), "__lambda_1"),
            (Some("_tg_1y_6helper"), "__lambda_2"),
            (None, "_tg_1y_6helper_lambda_3"),
            (Some("main"), "__lambda_4"),
        ];
        assert_eq!(owned_row_indices(&rows, "_tg_1x_6helper"), vec![0]);
        assert_eq!(owned_row_indices(&rows, "_tg_1y_6helper"), vec![1, 2]);
        assert!(owned_row_indices(&rows, "helper").is_empty());
    }

    #[test]
    fn the_heading_names_the_spelling_only_when_it_differs() {
        assert_eq!(symbol_set_heading("f", "f"), "Symbol set for `f`:");
        assert_eq!(
            symbol_set_heading("write", "_tg_5write"),
            "Symbol set for `write` — `write` is spelled `_tg_5write`:"
        );
    }

    #[test]
    fn the_no_definition_line_names_the_query_and_the_file() {
        assert_eq!(
            no_definition_line("nosuch", "prog.tg"),
            "no definition named `nosuch` in prog.tg"
        );
    }
}
