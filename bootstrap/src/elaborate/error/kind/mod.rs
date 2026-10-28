//! Categories of elaboration errors with default messages and error codes.
//! The codes, and how the bootstrap's numbering differs from the self-host's,
//! are in `codes.rs`.

use serde::{Deserialize, Serialize};

use crate::span::Span;
use albite_core::{Term, Type};

/// Categories of elaboration errors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ElabErrorKind {
    // ── Name resolution errors ──
    /// Reference to undefined variable
    UndefinedVariable(String),
    /// Reference to undefined type
    UndefinedType(String),
    /// Reference to undefined constructor
    UndefinedConstructor(String),
    /// Duplicate definition
    DuplicateDefinition(String),
    /// Module not found in qualified path
    ModuleNotFound {
        module: String,
        suggestion: Option<String>,
    },
    /// Item not found in specified module
    ItemNotFoundInModule { module: String, item: String },

    /// Duplicate import (same name imported twice)
    ///
    /// The diagnostic's primary span is `second_import_span` (the import that triggered
    /// duplicate detection). The `first_import_span` is rendered as a secondary label.
    DuplicateImport {
        /// The name that was imported twice
        name: String,
        /// Span where the first import was written
        first_import_span: Span,
        /// Span where the second (duplicate) import was written
        second_import_span: Span,
        /// Module from which the name was first imported
        first_source_module: String,
        /// Module from which the name was imported again
        second_source_module: String,
    },

    /// Glob import conflict (same name imported from multiple globs)
    GlobConflict {
        /// The name that conflicts
        name: String,
        /// First module that exports this name
        first_module: String,
        /// Second module that exports this name
        second_module: String,
    },

    /// Unresolved import path
    UnresolvedImport(String),

    /// Private module accessed from outside its visibility scope
    PrivateModule {
        /// The module path that was accessed
        module_path: String,
        /// The module path from which access was attempted
        accessed_from: String,
    },

    /// Private item (type, value, or constructor) accessed from outside its visibility scope
    PrivateItem {
        /// The name of the item that was accessed
        item_name: String,
        /// The kind of item (for better error messages)
        item_kind: String,
        /// The module where the item is defined
        defined_in: String,
        /// The module from which access was attempted
        accessed_from: String,
    },

    /// Public item leaks a less visible type in its signature
    ///
    /// Example: `pub fn foo() -> PrivateType` is an error because
    /// external code could call `foo` but cannot name its return type.
    PublicItemLeak {
        /// The public item that has the visibility leak
        item_name: String,
        /// The kind of item (function, type alias, etc.)
        item_kind: String,
        /// The required visibility level (e.g., "public")
        required_visibility: String,
        /// Chain showing how the private type is reached
        /// e.g., `["MyAlias", "InnerPrivate"]` for a type alias chain
        leak_path: Vec<String>,
        /// The actual visibility of the leaked type (e.g., "private")
        leaked_visibility: String,
    },

    // ── Type errors ──
    /// Type mismatch between expected and actual
    TypeMismatch { expected: Type, found: Type },

    /// Cannot infer type (need annotation)
    CannotInferType,

    /// Cannot infer type argument for a polymorphic function
    CannotInferTypeArg(String),

    /// Wrong number of arguments
    ArityMismatch { expected: usize, found: usize },

    /// Expected a function type
    ExpectedFunction(Type),

    /// Expected a specific type
    ExpectedType { expected: String, found: Type },

    // ── Phase 1 restrictions ──
    /// Feature not supported in Phase 1
    UnsupportedFeature(String),

    /// A refused assignment, or a closure mentioning a `mut` binding,
    /// E0104–E0106 (ADR 13.10.26r D4).
    Mutable(MutableErrorKind),

    // ── Pattern matching errors ──
    /// Non-exhaustive pattern match
    NonExhaustiveMatch,

    /// A match whose arms are constructor patterns of an ADT, applied to a
    /// scrutinee whose type is not that ADT (ADR 15.8.26b).
    ///
    /// `adt_name` is the ADT the arms' constructors belong to, when the
    /// failing path knows it; the infer-mode sum walkers do not.
    MatchScrutineeNotAdt {
        adt_name: Option<String>,
        found: Type,
    },

    /// Unreachable match arm (after catch-all)
    UnreachableArm,

    /// Dead code after `return` expression (ADR 13.5.26d)
    DeadCodeAfterReturn,

    /// Pattern nesting too deep
    PatternTooDeep { depth: usize, max: usize },

    /// Pattern not supported
    UnsupportedPattern(String),

    // ── Try operator errors (ADR 13.5.26e) ──
    /// `?` used on a type that is not `Result` or `Option`
    TryOnNonTryType(String),

    /// `?` return type mismatch
    TryReturnMismatch {
        operand_type: String,
        return_type: String,
    },

    /// `?` used outside a function/closure body
    TryOutsideReturnContext,

    /// explicit `return` inside a `try` block (ADR 15.5.26d)
    ReturnInsideTryBlock,

    /// `return` where no function return type is in scope — a theorem body,
    /// or any other non-function elaboration context (ADR 15.8.26b).
    ReturnOutsideFunction,

    /// `try` block requires a `Result` type annotation (ADR 15.5.26d)
    TryBlockRequiresResultType,

    /// `try` block expected Sum encoding but found something else (ADR 15.5.26d)
    TryBlockExpectedSumEncoding,

    /// `try` block Result type missing Ok or Err constructor (ADR 15.5.26d)
    TryBlockMissingConstructor(String),

    // ── Let-else errors (ADR 13.5.26f) ──
    /// `else` branch in `let`-`else` does not diverge. Reserved: unreachable,
    /// as match desugaring reports a non-diverging `else` as E0010.
    LetElseNonDiverging(String),

    /// Irrefutable pattern in `let`-`else` (warning)
    LetElseIrrefutable,

    /// Irrefutable pattern in `if let` (warning)
    IfLetIrrefutable,

    // ── Named record errors (ADR 13.5.26h) ──
    /// Type used in named record constructor is not a record type
    NotARecordType(String),

    /// Missing field in named record constructor
    MissingRecordField { field: String, type_name: String },

    /// Extra (unknown) field in named record constructor
    ExtraRecordField { field: String, type_name: String },

    /// Duplicate field in named record constructor
    DuplicateRecordField(String),

    // ── Entry point errors (for compile/run) ──
    /// No main function found
    NoMainFunction,

    /// File contains `sorry` (cannot compile)
    ContainsSorry,

    // ── Type alias errors (ADR 15.5.26g) ──
    /// Recursive type alias cycle
    RecursiveAlias(String),

    // ── Strict positivity (ADR 7.8.26e) ──
    /// A type definition's own SCC is reached from a forbidden position
    /// (left of an arrow, inside a `Ref`/`Ptr` cell, or in an `Eq` witness):
    /// a soundness gate, since it makes "structural subterm" well-founded.
    NonStrictlyPositive {
        /// The definition whose field carries the occurrence.
        type_name: String,
        /// The constructor; for a record, the record type's own name.
        ctor_name: String,
        /// Whether `type_name` is a record (changes how the field is rendered).
        is_record: bool,
        /// Which field of that constructor, already rendered
        /// (`field 0` / `` field `x` ``).
        field: String,
        /// The group member reached at a forbidden position.
        occurrence: String,
        /// Rendered `(intermediate type, parameter)` chain the violation was
        /// inherited through; empty for a direct arrow-domain occurrence.
        via: Vec<(String, String)>,
    },

    // ── Termination checking (ADR 29.6.26e) ──
    /// A recursive definition could not be admitted as a total constant. The
    /// message is `albite_core`'s, so the gate and the doctor census agree.
    CannotProveTermination {
        /// The definition that was rejected.
        function: String,
        /// The already-rendered headline.
        headline: String,
    },

    // ── Nested inductive families (ADR 11.8.26c) ──
    /// A `match` scrutinized a type whose recursive occurrence sits under a
    /// generic parameter (`type Rose = Node(Wrap<Rose>)`).
    ///
    /// Such a *nested* family encodes to a μ chain that never resolves to a
    /// structural head — for `Rose`, the vacuous `μα_Rose. α_Rose` — so its
    /// constructor arms have no sum type to be elaborated against. Before this
    /// was a diagnostic it was an unbounded loop in Body Elaboration.
    NestedRecursiveFamily {
        /// The type being matched on.
        type_name: String,
        /// The μ binder that would not flatten (`α_Rose`).
        binder: String,
        /// The generic the recursive occurrence is nested under, when it could
        /// be recovered from the definition (`Wrap`).
        nested_under: Option<String>,
    },

    /// A theorem, lemma or axiom reached a `#[partial]` constant.
    PartialInProof {
        /// The proof-relevant definition.
        proof: String,
        /// The `#[partial]` constant at the end of the chain.
        tainted: String,
    },

    // ── Equality proof errors (ADR 21.5.26d) ──
    /// `refl` checked against a non-equality type
    ReflExpectedEquality(Type),

    /// `refl` checked against an equality type whose sides are not definitionally equal
    InvalidRefl { left: Term, right: Term },

    /// `subst` proof argument does not have an equality type
    SubstExpectedEquality(Type),

    /// `trans` endpoints don't match: h1's right side ≠ h2's left side
    TransEndpointMismatch { left: Term, right: Term },

    /// `cong` first argument is not a function type
    CongExpectedFunction(Type),

    // ── Motive errors (ADR 21.5.26g) ──
    /// `subst` motive is not a predicate lambda (e.g., a literal or non-lambda expression)
    MotiveNotPredicate(Type),

    /// `subst` motive binder type does not match equality base type
    MotiveDomainMismatch { expected: Type, found: Type },

    /// `subst` motive body is not a valid type expression
    MotiveBodyNotType,

    /// `natind` motive domain is not `Nat` (ADR 22.5.26a)
    NatIndMotiveNotNat(Type),

    // ── Comparator errors (ADR 29.6.26f / 15.8.26b) ──
    /// `__compare` (which `assert_eq_*` desugars to) at a type the build
    /// cannot synthesize a comparator for. Carries the rendered type.
    ComparatorUnavailable(String),

    // ── Signed integer errors (ADR 14.9.26c) ──
    /// An integer literal checked against `Int` that does not fit the signed
    /// 64-bit range. Carries the literal as written.
    IntLiteralOutOfRange(String),

    /// A user `type` definition under a builtin type's name (`Int`, `Nat`, …).
    /// Builtin lookup precedes user types, so the definition would be
    /// unreachable rather than a shadow.
    BuiltinTypeRedefined(String),
    /// An `exists` binder that sorts `Prop` (ADR 4.10.26h D3, 4.10.26i D1).
    ExistsBinderNotData(Type),

    // ── Internal errors (ADR 15.8.26b) ──
    /// The compiler's own broken invariant (E9998), not user-actionable: a bug
    /// report, never a program edit, so `albite explain error` resolves the
    /// code but the no-argument listing leaves it off.
    InternalError(String),

    // ── Other errors ──
    /// A generic message, deliberately code-less (E9999): defensive branches
    /// and test-harness verdicts (ADR 15.8.26b). A broken invariant is
    /// [`Self::InternalError`]; a user-reachable condition gets its own kind.
    Other(String),

    // Appended last, so no serialized kind's bincode index moves (ADR 2.10.26c
    // D8). `check_proof` refused a proof; `tactic` is `None` in term mode.
    KernelRefusedProof {
        def: String,
        tactic: Option<(String, Span)>,
        cause: String,
    },
    /// A tactic block's own errors, E0082–E0083 (D8).
    Tactic(TacticErrorKind),
    /// A kernel exemption row the build contradicts, E0084 (ADR 2.10.26d D4).
    KernelExemptionViolation { def: String, why: String },
    /// A proof reached foreign code, E0063 (ADR 3.10.26c D3); `via` runs from
    /// the proof to the definition that makes the call.
    ForeignInProof {
        proof: String,
        via: Vec<String>,
        extern_symbol: String,
    },
    /// Manifest and mounting errors, E0120–E0124 (ADR 3.10.26m D8).
    Package(PackageErrorKind),
    /// Sort errors, E0066–E0067 (ADR 4.10.26i D3, D5).
    Sort(SortErrorKind),
    /// A bare value two visible definitions match, E0019 (ADR 4.10.26j D3).
    AmbiguousValue {
        name: String,
        candidates: Vec<String>,
    },
    /// A resource handle used after it was consumed, or captured by a
    /// closure, E0092–E0093 (ADR 7.10.26t D7).
    Handle(HandleErrorKind),
    /// A region whose value is not first-order data, whose body reaches a
    /// handle or an outer type variable, or that sits in a proof,
    /// E0094–E0096 (ADR 9.10.26o D4).
    Region(RegionErrorKind),
}

mod codes;
mod handle;
mod mutable;
mod package;
mod region;
mod sort;
mod tactic;
pub use handle::*;
pub use mutable::*;
pub use package::*;
pub use region::*;
pub use sort::*;
pub use tactic::*;
