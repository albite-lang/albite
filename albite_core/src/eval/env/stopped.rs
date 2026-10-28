//! Evaluation that stops without producing a value (ADR 21.7.26f, 22.7.26a,
//! 1.8.26b).
//!
//! Five shapes: the wall-clock deadline tripped, the step limit was
//! exhausted, a global re-entered its own forcing — a **black hole**
//! (detected in `lookup.rs`) — a `compare<T>` whose comparator could not be
//! synthesized, or a residual comparison that reached a test assertion. None
//! is folded into `StepResult::Stuck` at the reporting boundary: a stuck term
//! is a *normal* outcome and would let the test pass silently (ADR 22.7.26a
//! §1.3; ADR 1.8.26b D3, which found the same trap under `compare`).
//! Interior stepping does treat them as stuck (so the loops terminate), but
//! the reason rides the env and every entry point returns it here.

use std::fmt;

use super::comparator_stop::ComparatorFailure;
use super::EvalEnv;
use crate::terms::{IntBinOp, Term};

/// Which operation trapped (ADR 14.9.26c; byte buffers, ADR 3.10.26n D6; and
/// the substring cut, ADR 16.10.26h D3).
///
/// The numeric code is what crosses the FFI to `tg_int_trap(kind)`, so the
/// native and evaluator paths print the same line from the same table. The
/// byte-buffer kinds were added in place rather than renaming the table to a
/// general `TrapKind`: `tg_int_trap` takes a bare code, so it already carries
/// any kind, and a rename would touch every `Int` trap site for no reader gain.
///
/// The messages live in a third place too: the native core's
/// `src/native_core/trap.tg` writes them as literals, since a compiled trap
/// under `ALBITE_NATIVE_CORE=on` runs no Rust (ADR 7.10.26q D2). A bootstrap
/// test evaluates its `int_trap_line` against [`IntTrapKind::message`] for
/// every code (`bootstrap/src/driver/native_core/trap_tests.rs`), so a new
/// kind is an edit here and there, and that test names the one forgotten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntTrapKind {
    /// `+ - *` or negation overflowed the signed range.
    Overflow(IntBinOp),
    /// Negation of the signed minimum.
    NegationOverflow,
    /// `/` or `%` with a zero divisor.
    DivisionByZero,
    /// `to_int` of a `Nat` above the signed maximum.
    NatAboveSignedMax,
    /// A byte-buffer index at or past its length, or a blit that would run
    /// past it (ADR 3.10.26n).
    ByteIndexOutOfRange,
    /// A byte store of a value above 255.
    ByteValueOutOfRange,
    /// A byte buffer used after `tg_bytes_into_string` consumed it — or, on
    /// the evaluator only, a handle that never named a buffer at all.
    ByteBufferConsumed,
    /// `tg_bytes_new` asked for more than `BYTES_MAX_LEN` bytes.
    ByteBufferTooLarge,
    /// `tg_bytes_into_string` over bytes that are not valid UTF-8.
    ByteBufferNotUtf8,
    /// `substring` whose start or end lands inside a multi-byte character:
    /// its result would not be UTF-8 (ADR 16.10.26h D1).
    SubstringInsideCharacter,
}

impl IntTrapKind {
    /// The numeric code `tg_int_trap` takes; [`IntTrapKind::from_code`] inverts it.
    #[must_use]
    pub fn code(self) -> u32 {
        match self {
            IntTrapKind::Overflow(IntBinOp::Add) => 0,
            IntTrapKind::Overflow(IntBinOp::Sub) => 1,
            IntTrapKind::Overflow(IntBinOp::Mul) => 2,
            IntTrapKind::Overflow(IntBinOp::Div) => 3,
            IntTrapKind::Overflow(IntBinOp::Mod) => 4,
            IntTrapKind::NegationOverflow => 5,
            IntTrapKind::DivisionByZero => 6,
            IntTrapKind::NatAboveSignedMax => 7,
            IntTrapKind::ByteIndexOutOfRange => 8,
            IntTrapKind::ByteValueOutOfRange => 9,
            IntTrapKind::ByteBufferConsumed => 10,
            IntTrapKind::ByteBufferTooLarge => 11,
            IntTrapKind::ByteBufferNotUtf8 => 12,
            IntTrapKind::SubstringInsideCharacter => 13,
            // Comparisons cannot overflow; the arm is a table completeness
            // stub, never reached.
            IntTrapKind::Overflow(_) => 99,
        }
    }

    /// Inverse of [`IntTrapKind::code`]; `None` for a code no kind owns.
    #[must_use]
    pub fn from_code(code: u32) -> Option<IntTrapKind> {
        Some(match code {
            0 => IntTrapKind::Overflow(IntBinOp::Add),
            1 => IntTrapKind::Overflow(IntBinOp::Sub),
            2 => IntTrapKind::Overflow(IntBinOp::Mul),
            3 => IntTrapKind::Overflow(IntBinOp::Div),
            4 => IntTrapKind::Overflow(IntBinOp::Mod),
            5 => IntTrapKind::NegationOverflow,
            6 => IntTrapKind::DivisionByZero,
            7 => IntTrapKind::NatAboveSignedMax,
            8 => IntTrapKind::ByteIndexOutOfRange,
            9 => IntTrapKind::ByteValueOutOfRange,
            10 => IntTrapKind::ByteBufferConsumed,
            11 => IntTrapKind::ByteBufferTooLarge,
            12 => IntTrapKind::ByteBufferNotUtf8,
            13 => IntTrapKind::SubstringInsideCharacter,
            _ => return None,
        })
    }

    /// The one stderr line both paths print, byte for byte (AC 3).
    #[must_use]
    pub fn message(self) -> String {
        match self {
            IntTrapKind::Overflow(op) => format!("integer overflow in {}", op.symbol()),
            IntTrapKind::NegationOverflow => "integer overflow in negation".to_string(),
            IntTrapKind::DivisionByZero => "integer division by zero".to_string(),
            IntTrapKind::NatAboveSignedMax => "to_int: value above the signed maximum".to_string(),
            // Operand-free by construction (ADR 3.10.26n D6): native code
            // passes only the code, so neither side may print an index.
            IntTrapKind::ByteIndexOutOfRange => "byte index out of range".to_string(),
            IntTrapKind::ByteValueOutOfRange => "byte value out of range".to_string(),
            IntTrapKind::ByteBufferConsumed => "byte buffer used after conversion".to_string(),
            IntTrapKind::ByteBufferTooLarge => "byte buffer too large".to_string(),
            IntTrapKind::ByteBufferNotUtf8 => "byte buffer is not valid UTF-8".to_string(),
            IntTrapKind::SubstringInsideCharacter => {
                "substring cuts inside a character".to_string()
            }
        }
    }
}

impl EvalEnv {
    /// Record why evaluation stopped. First writer wins: the earliest failure
    /// is the cause, and every later one is a consequence of evaluation
    /// continuing past it (ADR 1.8.26b / D3; generalised by 14.9.26c).
    pub fn record_stop(&self, stop: EvalStopped) {
        let mut slot = self.recorded_stop.borrow_mut();
        if slot.is_none() {
            *slot = Some(stop);
        }
    }

    /// The recorded stop, if any.
    #[must_use]
    pub fn recorded_stop(&self) -> Option<EvalStopped> {
        self.recorded_stop.borrow().clone()
    }
}

/// Why an evaluation entry point stopped without a value.
///
/// Generalizes 21.7.26f's `EvalTimedOut`: every non-value stop is one type,
/// so no caller can discard one variant while handling another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalStopped {
    /// The wall-clock deadline passed (ADR 21.7.26f / D1). `steps` is
    /// forensics for the reporting layer (`~4.1M steps` in a TIMEOUT line).
    TimedOut { steps: u64 },
    /// The step limit was exhausted before the term reached a value.
    StepLimit { limit: usize },
    /// A global re-entered its own forcing: it has no value (ADR 22.7.26a).
    /// The cycle is insertion-ordered and closed — `["a", "b", "a"]` for a
    /// mutual cycle, `["f", "f"]` for direct self-reference.
    BlackHole { cycle: Vec<String> },
    /// `compare<T>` at a concrete `T` whose comparator could not be
    /// synthesized (ADR 1.8.26b D3). The enumerated reason is what decides
    /// whether a residual wall is a *cost* wall or a *correctness* wall.
    Uncomparable(ComparatorFailure),
    /// A residual `compare` application reached a test assertion: the
    /// comparison never ran, so the assertion asserted nothing (ADR 1.8.26b).
    ///
    /// Deliberately distinct from [`EvalStopped::Uncomparable`], which
    /// enumerates the failure modes known *today*. This one is the backstop
    /// for a mode nobody has thought of yet — it is asserted at the assertion
    /// boundary rather than by enumerating the ways synthesis can go wrong,
    /// which is the whole failure class the ADR exists for.
    ComparisonNeverRan { symbol: String },
    /// An eliminator was applied to a value of the wrong shape: `Fst`/`Snd` of
    /// a non-pair, or `Unfold` of a non-`Fold`. Unconditionally a bug — such a
    /// term can never step, whatever else is in scope — and it is what a
    /// representation mismatch looks like at runtime: a constructor-payload
    /// nesting disagreement for the projections (ADR 1.8.26b D1;
    /// `docs/repo-memory/adt-abi-safety.md`), a fold/unfold count disagreement
    /// for `Unfold` (ADR 1.8.26b D2).
    MalformedElimination {
        eliminator: &'static str,
        head: String,
    },
    /// An `Int` operation trapped: overflow, a zero divisor, or a bridge out
    /// of range (ADR 14.9.26c). Native code aborts through `tg_int_trap`
    /// with the same line, and `tungsten run` exits non-zero on both.
    IntTrap { kind: IntTrapKind },
    /// A byte buffer's unwritten byte was read: a `get` of it, or a
    /// conversion over it (ADR 9.10.26n D3). **Evaluator-only**, so not an
    /// [`IntTrapKind`]: natively an unfilled buffer has no trap to match, and
    /// the static write-cover rule is what keeps compiled code from reaching
    /// one. Here it is the second line under the core's own `.tg` tests.
    UnwrittenByteRead { offset: u64 },
}

impl fmt::Display for EvalStopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalStopped::TimedOut { steps } => {
                write!(f, "evaluation timed out (~{steps} steps)")
            }
            EvalStopped::StepLimit { limit } => {
                write!(f, "evaluation exceeded {limit} steps")
            }
            EvalStopped::BlackHole { cycle } => {
                write!(f, "black hole: {}", cycle.join(" → "))
            }
            EvalStopped::Uncomparable(failure) => write!(f, "{failure}"),
            EvalStopped::ComparisonNeverRan { symbol } => write!(
                f,
                "a comparison never ran: `{symbol}` was still unresolved when an \
                 assertion consumed its result, so the assertion asserted nothing"
            ),
            EvalStopped::MalformedElimination { eliminator, head } => write!(
                f,
                "`{eliminator}` applied to {head} — the value's representation does \
                 not match the eliminator applied to it"
            ),
            EvalStopped::IntTrap { kind } => write!(f, "{}", kind.message()),
            EvalStopped::UnwrittenByteRead { offset } => {
                write!(f, "byte buffer reads an unwritten byte at offset {offset}")
            }
        }
    }
}

/// What `EvalEnv::lookup` found for a global name.
///
/// Three-state by design (ADR 22.7.26a / D2): `Unbound` and `BlackHole` both
/// step to `Stuck` interiorly, but they are different facts — "no definition
/// under this name" vs "this definition has no value" — and only the former
/// may surface as an ordinary stuck term at the reporting boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GlobalLookup {
    /// The global's (memoized) value.
    Value(Term),
    /// No definition under this name; the reference is stuck, as today.
    Unbound,
    /// The global re-entered its own forcing (directly or through a mutual
    /// cycle). The ordered cycle is recorded on the env for the entry point.
    BlackHole,
}
