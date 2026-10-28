//! A native-core row's trust class, read from what its definition reaches
//! (ADR 9.10.26c D1–D3).
//!
//! The core may call byte intrinsics, which 5.10.26f made pure, and the
//! core-reserved runtime intrinsics (7.10.26q D1), which write to stderr,
//! abort or publish the program's arguments. A row is `runtime` when its
//! definition reaches one of the latter through its global references, and
//! `pure` only when every reachable global resolved and every reachable
//! `ExternCall` is a byte intrinsic. Anything else is an error, so a core
//! change that calls something new cannot slip back into `pure`.
//!
//! An `extern` elaborates to a stub definition of its own name whose body is
//! the `ExternCall` (6.10.26n §6), so following a `Term::Global` into the stub
//! reaches the call. The walk is a pure function over `&[CoreDef]`.

use std::collections::{BTreeSet, HashMap};
use std::fmt;

use albite_bootstrap::elaborate::CoreDef;
use albite_core::eval::byte_intrinsics::ByteIntrinsic;
use albite_core::native_core::RuntimeIntrinsic;
use albite_core::terms::Term;

use super::classify::Class;
use super::NATIVE_CORE_BASIS;

/// A row's class and the sentence that decided it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RowVerdict {
    pub(crate) class: Class,
    pub(crate) basis: String,
}

/// Why a row could not be classed — never a class (D1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReachError {
    /// The row's definition is not in the core.
    MissingRow { definition: String },
    /// A reachable global has no definition.
    MissingGlobal { referrer: String, global: String },
    /// A reachable `ExternCall` is neither a byte nor a runtime intrinsic.
    ForeignExtern { caller: String, symbol: String },
}

impl fmt::Display for ReachError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReachError::MissingRow { definition } => {
                write!(f, "`{definition}` is not defined in the core")
            }
            ReachError::MissingGlobal { referrer, global } => {
                write!(
                    f,
                    "`{referrer}` references `{global}`, which has no definition"
                )
            }
            ReachError::ForeignExtern { caller, symbol } => write!(
                f,
                "`{caller}` calls `{symbol}`, which is neither a byte nor a runtime intrinsic"
            ),
        }
    }
}

/// What one definition's body names: the globals it references and the
/// extern symbols it calls, `__c_` prefix stripped.
#[derive(Default)]
struct BodyNames {
    globals: Vec<String>,
    externs: Vec<String>,
}

fn collect_body_names(term: &Term, names: &mut BodyNames) {
    match term {
        Term::Global(name) => names.globals.push(name.clone()),
        Term::ExternCall(symbol, _) => {
            let bare = symbol.strip_prefix("__c_").unwrap_or(symbol);
            names.externs.push(bare.to_string());
        }
        _ => {}
    }
    term.for_each_subterm(|child| collect_body_names(child, names));
}

/// The class of the row whose definition is `definition`, walked through
/// `defs` (D1). A cycle terminates: each definition is visited once.
///
/// # Errors
/// The definition is missing, a reachable global has no definition, or a
/// reachable `ExternCall` is on neither intrinsic list. An error outranks a
/// runtime intrinsic found elsewhere, so a row is classed only when the whole
/// reach was read.
pub(crate) fn row_verdict(definition: &str, defs: &[CoreDef]) -> Result<RowVerdict, ReachError> {
    let by_name: HashMap<&str, &CoreDef> =
        defs.iter().map(|def| (def.name.as_str(), def)).collect();
    if !by_name.contains_key(definition) {
        return Err(ReachError::MissingRow {
            definition: definition.to_string(),
        });
    }
    let mut visited: BTreeSet<&str> = BTreeSet::new();
    let mut pending: Vec<&str> = vec![definition];
    let mut runtime: BTreeSet<&'static str> = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if !visited.insert(name) {
            continue;
        }
        let mut names = BodyNames::default();
        collect_body_names(&by_name[name].term.term, &mut names);
        for symbol in names.externs {
            if let Some(intrinsic) = RuntimeIntrinsic::from_symbol(&symbol) {
                runtime.insert(intrinsic.symbol());
            } else if ByteIntrinsic::from_symbol(&symbol).is_none() {
                return Err(ReachError::ForeignExtern {
                    caller: name.to_string(),
                    symbol,
                });
            }
        }
        for global in names.globals {
            match by_name.get_key_value(global.as_str()) {
                Some((&resolved, _)) => pending.push(resolved),
                None => {
                    return Err(ReachError::MissingGlobal {
                        referrer: name.to_string(),
                        global,
                    })
                }
            }
        }
    }
    Ok(verdict_from_runtime(&runtime))
}

/// D3: a pure row keeps the old basis; a runtime row names its intrinsics,
/// sorted.
fn verdict_from_runtime(runtime: &BTreeSet<&'static str>) -> RowVerdict {
    if runtime.is_empty() {
        return RowVerdict {
            class: Class::Pure,
            basis: NATIVE_CORE_BASIS.to_string(),
        };
    }
    let symbols: Vec<&str> = runtime.iter().copied().collect();
    RowVerdict {
        class: Class::Runtime,
        basis: format!("native core: runtime intrinsics {}", symbols.join(", ")),
    }
}

/// One verdict or reason per row definition, from the core's definitions —
/// or, when the core did not elaborate, that reason for every row (D4).
pub(crate) fn row_verdicts<'a>(
    definitions: impl IntoIterator<Item = &'a str>,
    core: Result<&[CoreDef], &str>,
) -> HashMap<String, Result<RowVerdict, String>> {
    definitions
        .into_iter()
        .map(|definition| {
            let verdict = match core {
                Ok(defs) => row_verdict(definition, defs).map_err(|error| error.to_string()),
                Err(reason) => Err(reason.to_string()),
            };
            (definition.to_string(), verdict)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use albite_bootstrap::span::Span;
    use albite_core::terms::SpannedTerm;
    use albite_core::Type;

    use super::*;

    fn def(name: &str, term: Term) -> CoreDef {
        CoreDef {
            name: name.to_string(),
            ty: Type::Unit,
            term: SpannedTerm::generated(term),
            span: Span::new(0, 0),
        }
    }

    fn call(callee: &str, arg: Term) -> Term {
        Term::App(Box::new(Term::Global(callee.to_string())), Box::new(arg))
    }

    /// An `extern` stub (6.10.26n §6): a definition of its own name whose
    /// body is the elaborator's `__c_`-prefixed `ExternCall`.
    fn stub(symbol: &str) -> CoreDef {
        def(
            symbol,
            Term::ExternCall(format!("__c_{symbol}"), vec![Term::Unit]),
        )
    }

    // ADR 9.10.26c AC1: two global hops, then extern stubs, reach runtime
    // intrinsics; the basis names them sorted, whatever order they were met.
    #[test]
    fn a_runtime_intrinsic_two_hops_away_is_runtime_with_sorted_symbols() {
        let defs = [
            def("row", call("middle", Term::Unit)),
            def("middle", call("leaf", Term::Unit)),
            def(
                "leaf",
                call("tg_core_write_stderr", call("tg_core_abort", Term::Unit)),
            ),
            stub("tg_core_write_stderr"),
            stub("tg_core_abort"),
        ];
        assert_eq!(
            row_verdict("row", &defs),
            Ok(RowVerdict {
                class: Class::Runtime,
                basis: "native core: runtime intrinsics tg_core_abort, tg_core_write_stderr"
                    .to_string(),
            })
        );
    }

    // ADR 9.10.26c AC1: byte intrinsics alone, reached directly or through a
    // stub, are pure under the old basis.
    #[test]
    fn byte_intrinsics_alone_are_pure() {
        let defs = [
            def(
                "row",
                call(
                    "tg_bytes_len",
                    Term::ExternCall("tg_bytes_new".to_string(), vec![Term::Unit]),
                ),
            ),
            stub("tg_bytes_len"),
        ];
        assert_eq!(
            row_verdict("row", &defs),
            Ok(RowVerdict {
                class: Class::Pure,
                basis: NATIVE_CORE_BASIS.to_string(),
            })
        );
    }

    // ADR 9.10.26c AC1: a cycle terminates, and a cycle with a way out to a
    // runtime intrinsic is runtime.
    #[test]
    fn a_cycle_terminates_and_its_outgoing_path_is_followed() {
        let closed = [
            def("row", call("other", Term::Unit)),
            def("other", call("row", Term::Unit)),
        ];
        assert_eq!(
            row_verdict("row", &closed).map(|v| v.class),
            Ok(Class::Pure)
        );
        let open = [
            def("row", call("other", Term::Unit)),
            def("other", call("row", call("tg_core_abort", Term::Unit))),
            stub("tg_core_abort"),
        ];
        assert_eq!(
            row_verdict("row", &open).map(|v| v.basis),
            Ok("native core: runtime intrinsics tg_core_abort".to_string())
        );
    }

    // ADR 9.10.26c AC1: each unread fact is an error naming it, never pure —
    // even beside a runtime intrinsic the walk did find.
    #[test]
    fn a_missing_row_global_or_foreign_extern_is_an_error_naming_it() {
        let defs = [def("row", Term::Unit)];
        let missing_row = row_verdict("absent", &defs).expect_err("no row");
        assert_eq!(
            missing_row.to_string(),
            "`absent` is not defined in the core"
        );
        let dangling = [def("row", call("ghost", Term::Unit))];
        assert_eq!(
            row_verdict("row", &dangling)
                .expect_err("dangling")
                .to_string(),
            "`row` references `ghost`, which has no definition"
        );
        let foreign = [
            def("row", call("tg_core_abort", call("puts", Term::Unit))),
            stub("tg_core_abort"),
            stub("puts"),
        ];
        assert_eq!(
            row_verdict("row", &foreign),
            Err(ReachError::ForeignExtern {
                caller: "puts".to_string(),
                symbol: "puts".to_string(),
            })
        );
        assert_eq!(
            ReachError::ForeignExtern {
                caller: "f".to_string(),
                symbol: "puts".to_string(),
            }
            .to_string(),
            "`f` calls `puts`, which is neither a byte nor a runtime intrinsic"
        );
    }

    // ADR 9.10.26c D4: a core that did not elaborate is every row's reason;
    // one that did gives each row its own verdict or reason.
    #[test]
    fn row_verdicts_carries_a_core_failure_to_every_row() {
        let failed = row_verdicts(["a", "b"], Err("no core"));
        assert_eq!(failed.len(), 2);
        assert!(failed.values().all(|v| v == &Err("no core".to_string())));
        let defs = [def("a", Term::Unit)];
        let read = row_verdicts(["a", "b"], Ok(&defs));
        assert_eq!(read["a"].as_ref().map(|v| v.class), Ok(Class::Pure));
        assert_eq!(read["b"], Err("`b` is not defined in the core".to_string()));
    }
}
