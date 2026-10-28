//! The checked byte buffer (ADR 3.10.26n): nine `tg_bytes_*` intrinsics, the
//! seventh, eighth and ninth reserved to the native core (ADR 4.10.26n D5,
//! 6.10.26n D2, 9.10.26n D2).
//!
//! I3's restated scope — the runtime exports an ordinary program reaches,
//! written in Tungsten — needs a way to store a byte, and this is it: a buffer
//! behind an opaque handle, every index checked, conversion to `String`
//! checked for UTF-8. It is **not** a `albite_core` export. The bootstrap
//! code generator lowers them inline and never declares them to the linker
//! (`albite_codegen/src/codegen/data/ops/bytes/`); this module is the other
//! execution, so a `.tg` test over them runs rather than going `Stuck`.
//!
//! ## One table, two readers
//!
//! [`ByteIntrinsic`] and [`BYTES_MAX_LEN`] are public so the code generator
//! and `doctor check link extern-symbols` read the same list and the same
//! ceiling the evaluator enforces; the registry's `is_codegen_intrinsic` is
//! the link check's entry point.
//!
//! ## The safety boundary (D2)
//!
//! Here a handle is a key into a thread-local table, never an address, so a
//! forged or consumed handle is a trap rather than a host dereference inside
//! the compiler's own process. Compiled code protects only the handles
//! `tg_bytes_new` produced.
//!
//! ## The reserved intrinsic (ADR 4.10.26n D5)
//!
//! `tg_bytes_into_string_unchecked` is `into_string` without the UTF-8 walk
//! natively, because a compiled `String` may hold any bytes and the native
//! core's `tg_string_concat` must return them unchanged, as Rust's does. Here
//! it keeps the check: an evaluator `String` is a Rust `String`, and the only
//! caller, the native core, hands it evaluator strings, so the difference is
//! unobservable. [`ByteIntrinsic::is_reserved`] is what every reader consults:
//! the project driver refuses a declaration of it outside an embedded core
//! source, and the code generator outside the core's own run.
//!
//! ## Refusals
//!
//! Every refusal but one is an [`IntTrapKind`] recorded on the env, the same
//! operand-free line native code prints through `tg_int_trap` (D6). Checks run
//! in one order on both sides — liveness, then the index or bound, then the
//! value — so a call that breaks two rules prints the same line on each. The
//! one exception is evaluator-only: a read of a byte `tg_bytes_new_uninit` or
//! a grow left unwritten ([`unwritten`], ADR 9.10.26n D3).

mod unwritten;
mod utf8;

use unwritten::{ByteStop, UnwrittenBytes};

pub use utf8::{utf8_lead_entry, utf8_valid_by_lead_table, UTF8_LEAD_INVALID};

use std::cell::RefCell;
use std::collections::HashMap;

use crate::eval::env::{EvalEnv, IntTrapKind};
use crate::eval::{nat_to_term, term_to_nat, StepResult};
use crate::terms::Term;

/// The largest buffer `tg_bytes_new` allocates: 2³² bytes (ADR 3.10.26n P0.5).
///
/// The runtime allocator has no ceiling of its own beyond `malloc`'s, so the
/// ADR's proposed bound stands. It keeps the native header arithmetic far from
/// overflow and stops the evaluator asking for an impossible `Vec`.
pub const BYTES_MAX_LEN: u64 = 1 << 32;

/// The nine intrinsics: 3.10.26n D3's six, then 4.10.26n D5's, 6.10.26n D2's
/// and 9.10.26n D2's reserved ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteIntrinsic {
    /// `tg_bytes_new(len: Nat) -> Nat` — a zero-filled buffer.
    New,
    /// `tg_bytes_len(h: Nat) -> Nat`.
    Len,
    /// `tg_bytes_get(h: Nat, i: Nat) -> Nat` — a byte, zero-extended.
    Get,
    /// `tg_bytes_set(h: Nat, i: Nat, v: Nat) -> Nat` — returns `h`.
    Set,
    /// `tg_bytes_blit_string(h: Nat, at: Nat, s: String) -> Nat` — returns `h`.
    BlitString,
    /// `tg_bytes_into_string(h: Nat) -> String` — consumes the buffer.
    IntoString,
    /// `tg_bytes_into_string_unchecked(h: Nat) -> String` — consumes the
    /// buffer; natively skips the UTF-8 walk. Reserved to the native core.
    IntoStringUnchecked,
    /// `tg_bytes_from_string_grown(s: String, extra: Nat) -> Nat` — adopts
    /// `s`'s allocation as a live buffer `extra` unwritten bytes longer; `s`
    /// is dead afterwards. Natively a `realloc`, so reserved to the native
    /// core's owned export (ADR 6.10.26n D2, D3; 9.10.26n D2); here adoption
    /// is unobservable.
    FromStringGrown,
    /// `tg_bytes_new_uninit(len: Nat) -> Nat` — `tg_bytes_new` without the
    /// zero-fill natively. Reserved to the native core, which may call it only
    /// under the write-cover rule (ADR 9.10.26n D2, D3).
    NewUninit,
}

impl ByteIntrinsic {
    /// Every intrinsic, once.
    pub const ALL: [ByteIntrinsic; 9] = [
        ByteIntrinsic::New,
        ByteIntrinsic::Len,
        ByteIntrinsic::Get,
        ByteIntrinsic::Set,
        ByteIntrinsic::BlitString,
        ByteIntrinsic::IntoString,
        ByteIntrinsic::IntoStringUnchecked,
        ByteIntrinsic::FromStringGrown,
        ByteIntrinsic::NewUninit,
    ];

    /// The symbol a `.tg` `extern "C" fn` declaration names.
    #[must_use]
    pub fn symbol(self) -> &'static str {
        match self {
            ByteIntrinsic::New => "tg_bytes_new",
            ByteIntrinsic::Len => "tg_bytes_len",
            ByteIntrinsic::Get => "tg_bytes_get",
            ByteIntrinsic::Set => "tg_bytes_set",
            ByteIntrinsic::BlitString => "tg_bytes_blit_string",
            ByteIntrinsic::IntoString => "tg_bytes_into_string",
            ByteIntrinsic::IntoStringUnchecked => "tg_bytes_into_string_unchecked",
            ByteIntrinsic::FromStringGrown => "tg_bytes_from_string_grown",
            ByteIntrinsic::NewUninit => "tg_bytes_new_uninit",
        }
    }

    /// Whether only the native core may use this intrinsic (ADR 4.10.26n D5):
    /// a user program that could convert bytes unchecked would diverge from
    /// the evaluator, undoing 3.10.26n's parity property. One that adopted a
    /// string literal would `realloc` read-only data (ADR 6.10.26n D2), and
    /// one that allocated unfilled has no write-cover rule (ADR 9.10.26n D2).
    #[must_use]
    pub fn is_reserved(self) -> bool {
        matches!(
            self,
            ByteIntrinsic::IntoStringUnchecked
                | ByteIntrinsic::FromStringGrown
                | ByteIntrinsic::NewUninit
        )
    }

    /// The intrinsic `name` spells, accepting the elaborator's `__c_` prefix.
    #[must_use]
    pub fn from_symbol(name: &str) -> Option<ByteIntrinsic> {
        let name = name.strip_prefix("__c_").unwrap_or(name);
        Self::ALL.into_iter().find(|op| op.symbol() == name)
    }
}

/// Whether `tg_bytes_new(len)` refuses: strictly above [`BYTES_MAX_LEN`], so
/// the ceiling itself is a legal length. Its own function so the boundary is
/// testable without allocating 4 GiB.
pub(crate) fn exceeds_ceiling(len: u64) -> bool {
    len > BYTES_MAX_LEN
}

/// Whether `tg_bytes_from_string_grown` refuses: `len + extra` above
/// [`BYTES_MAX_LEN`] (ADR 6.10.26n D2). Written so neither the sum nor the
/// difference can wrap: the first disjunct guards the subtraction. The native
/// lowering emits the same two comparisons.
#[must_use]
pub fn grown_exceeds_ceiling(len: u64, extra: u64) -> bool {
    len > BYTES_MAX_LEN || extra > BYTES_MAX_LEN - len
}

/// The evaluator's buffers: `handle → Some(bytes)` while live, `None` once
/// consumed. A handle absent from the map was never issued (forged).
///
/// A plain struct rather than bare thread-local state, so every rule is
/// assertable against a fresh table.
#[derive(Debug, Default)]
pub(crate) struct ByteBuffers {
    /// The last handle issued; handles start at 1, so 0 never names a buffer.
    last_handle: u64,
    buffers: HashMap<u64, Option<Vec<u8>>>,
    /// The bytes `new_uninit_buffer` and `from_string_grown` left unwritten.
    unwritten: UnwrittenBytes,
}

impl ByteBuffers {
    /// Allocate a zero-filled buffer, refusing above [`BYTES_MAX_LEN`]
    /// **before** allocating.
    pub(crate) fn new_buffer(&mut self, len: u64) -> Result<u64, IntTrapKind> {
        if exceeds_ceiling(len) {
            return Err(IntTrapKind::ByteBufferTooLarge);
        }
        let len = usize::try_from(len).map_err(|_| IntTrapKind::ByteBufferTooLarge)?;
        self.last_handle += 1;
        self.buffers.insert(self.last_handle, Some(vec![0; len]));
        Ok(self.last_handle)
    }

    /// [`ByteBuffers::new_buffer`] with every byte marked unwritten (ADR
    /// 9.10.26n D2): natively the block is not filled.
    pub(crate) fn new_uninit_buffer(&mut self, len: u64) -> Result<u64, IntTrapKind> {
        let handle = self.new_buffer(len)?;
        // Under the ceiling, so `len` fits a `usize` on a 64-bit host.
        self.unwritten.open(handle, len as usize, 0);
        Ok(handle)
    }

    /// A live buffer holding `text`'s bytes then `extra` unwritten ones (ADR
    /// 6.10.26n D2, 9.10.26n D2), refusing above the ceiling **before**
    /// allocating. Natively this adopts the string's allocation; here the
    /// string is a value, so it is copied, and the difference is
    /// unobservable. The new bytes hold zeros here and are marked unwritten.
    #[allow(clippy::wrong_self_convention)] // Reason: the extern's name
    pub(crate) fn from_string_grown(&mut self, text: &str, extra: u64) -> Result<u64, IntTrapKind> {
        if grown_exceeds_ceiling(text.len() as u64, extra) {
            return Err(IntTrapKind::ByteBufferTooLarge);
        }
        let mut bytes = text.as_bytes().to_vec();
        // Under the ceiling, so `extra` fits a `usize` on a 64-bit host.
        bytes.resize(text.len() + extra as usize, 0);
        let len = bytes.len();
        self.last_handle += 1;
        self.buffers.insert(self.last_handle, Some(bytes));
        self.unwritten.open(self.last_handle, len, text.len());
        Ok(self.last_handle)
    }

    /// The live buffer behind `handle`; consumed and forged both refuse.
    fn live(&self, handle: u64) -> Result<&Vec<u8>, IntTrapKind> {
        match self.buffers.get(&handle) {
            Some(Some(bytes)) => Ok(bytes),
            _ => Err(IntTrapKind::ByteBufferConsumed),
        }
    }

    fn live_mut(&mut self, handle: u64) -> Result<&mut Vec<u8>, IntTrapKind> {
        match self.buffers.get_mut(&handle) {
            Some(Some(bytes)) => Ok(bytes),
            _ => Err(IntTrapKind::ByteBufferConsumed),
        }
    }

    pub(crate) fn len(&self, handle: u64) -> Result<u64, IntTrapKind> {
        Ok(self.live(handle)?.len() as u64)
    }

    /// The byte at `index`, zero-extended. The index is compared as the
    /// unsigned `u64` it is, so `2^63` reads as large, never negative.
    pub(crate) fn get(&self, handle: u64, index: u64) -> Result<u64, IntTrapKind> {
        let bytes = self.live(handle)?;
        let slot = usize::try_from(index).map_err(|_| IntTrapKind::ByteIndexOutOfRange)?;
        bytes
            .get(slot)
            .map(|byte| u64::from(*byte))
            .ok_or(IntTrapKind::ByteIndexOutOfRange)
    }

    /// Store `value` at `index`; returns the handle.
    pub(crate) fn set(&mut self, handle: u64, index: u64, value: u64) -> Result<u64, IntTrapKind> {
        let bytes = self.live_mut(handle)?;
        let slot = usize::try_from(index).map_err(|_| IntTrapKind::ByteIndexOutOfRange)?;
        let cell = bytes
            .get_mut(slot)
            .ok_or(IntTrapKind::ByteIndexOutOfRange)?;
        *cell = u8::try_from(value).map_err(|_| IntTrapKind::ByteValueOutOfRange)?;
        self.unwritten.write(handle, slot, slot + 1);
        Ok(handle)
    }

    /// Copy all of `text` to `at`; returns the handle.
    ///
    /// The bound is `at ≤ len` and then `len(text) ≤ len − at` — never
    /// `at + len(text) > len`, which wraps at `at = u64::MAX`.
    pub(crate) fn blit_string(
        &mut self,
        handle: u64,
        at: u64,
        text: &str,
    ) -> Result<u64, IntTrapKind> {
        let bytes = self.live_mut(handle)?;
        let len = bytes.len() as u64;
        if at > len || text.len() as u64 > len - at {
            return Err(IntTrapKind::ByteIndexOutOfRange);
        }
        // Both bounds hold, so `at` fits the buffer and so a `usize`.
        let start = at as usize;
        bytes[start..start + text.len()].copy_from_slice(text.as_bytes());
        self.unwritten.write(handle, start, start + text.len());
        Ok(handle)
    }

    /// The buffer as a `String`, consuming the handle. Invalid UTF-8 refuses
    /// and leaves the buffer live — the trap ends the program either way.
    #[allow(clippy::wrong_self_convention)] // Reason: the extern's name
    pub(crate) fn into_string(&mut self, handle: u64) -> Result<String, IntTrapKind> {
        let text = std::str::from_utf8(self.live(handle)?)
            .map_err(|_| IntTrapKind::ByteBufferNotUtf8)?
            .to_owned();
        self.buffers.insert(handle, None);
        self.unwritten.close(handle);
        Ok(text)
    }

    /// `get`, stopping on an unwritten byte (ADR 9.10.26n D3) after the
    /// liveness and index checks, in their usual order.
    pub(crate) fn read_written(&self, handle: u64, index: u64) -> Result<u64, ByteStop> {
        let byte = self.get(handle, index)?;
        // `get` bounded `index` by the buffer's length.
        let slot = index as usize;
        match self.unwritten.first_in(handle, slot, slot + 1) {
            Some(offset) => Err(ByteStop::UnwrittenRead { offset }),
            None => Ok(byte),
        }
    }

    /// `into_string`, stopping on any unwritten byte (ADR 9.10.26n D3):
    /// liveness first, then the unwritten bytes, then UTF-8.
    pub(crate) fn convert_written(&mut self, handle: u64) -> Result<String, ByteStop> {
        let len = self.live(handle)?.len();
        if let Some(offset) = self.unwritten.first_in(handle, 0, len) {
            return Err(ByteStop::UnwrittenRead { offset });
        }
        Ok(self.into_string(handle)?)
    }
}

thread_local! {
    /// One table per evaluating thread, as the type arena and the builder
    /// heap are: a handle means nothing outside the thread that issued it.
    static BYTE_BUFFERS: RefCell<ByteBuffers> = RefCell::new(ByteBuffers::default());
}

/// Execute a `tg_bytes_*` intrinsic, or `None` if `name` is not one (or the
/// arity is wrong, so the caller's other dispatchers get their turn).
///
/// A refusal records its stop on `env` and steps to `Stuck`, the way an `Int`
/// overflow does, so the entry point reports the trap line.
pub(super) fn step_bytes_extern(name: &str, values: &[Term], env: &EvalEnv) -> Option<StepResult> {
    let op = ByteIntrinsic::from_symbol(name)?;
    let outcome = BYTE_BUFFERS.with(|table| apply(&mut table.borrow_mut(), op, values))?;
    Some(match outcome {
        Ok(value) => StepResult::Stepped(value),
        Err(stop) => {
            env.record_stop(stop.into_stop());
            StepResult::Stuck
        }
    })
}

/// Marshal `values` for `op` and run it against `table`.
///
/// `None` for a shape the `.tg` declaration cannot produce — wrong arity, or a
/// non-`Nat` where a `Nat` belongs — so the call is not claimed.
pub(crate) fn apply(
    table: &mut ByteBuffers,
    op: ByteIntrinsic,
    values: &[Term],
) -> Option<Result<Term, ByteStop>> {
    let nat = |term: &Term| term_to_nat(term).map(|n| n as u64);
    let handle_term = |result: Result<u64, IntTrapKind>| {
        result
            .map(|n| nat_to_term(n as usize))
            .map_err(ByteStop::from)
    };
    Some(match (op, values) {
        (ByteIntrinsic::New, [len]) => handle_term(table.new_buffer(nat(len)?)),
        (ByteIntrinsic::NewUninit, [len]) => handle_term(table.new_uninit_buffer(nat(len)?)),
        (ByteIntrinsic::Len, [h]) => handle_term(table.len(nat(h)?)),
        (ByteIntrinsic::Get, [h, i]) => table
            .read_written(nat(h)?, nat(i)?)
            .map(|n| nat_to_term(n as usize)),
        (ByteIntrinsic::Set, [h, i, v]) => handle_term(table.set(nat(h)?, nat(i)?, nat(v)?)),
        (ByteIntrinsic::BlitString, [h, at, Term::StringLit(text)]) => {
            handle_term(table.blit_string(nat(h)?, nat(at)?, text))
        }
        // The unchecked conversion keeps the check here: see the module docs.
        (ByteIntrinsic::IntoString | ByteIntrinsic::IntoStringUnchecked, [h]) => {
            table.convert_written(nat(h)?).map(Term::StringLit)
        }
        (ByteIntrinsic::FromStringGrown, [Term::StringLit(text), extra]) => {
            handle_term(table.from_string_grown(text, nat(extra)?))
        }
        _ => return None,
    })
}

#[cfg(test)]
pub(super) mod tests;

#[cfg(test)]
mod grow_tests;

#[cfg(test)]
mod unwritten_tests;
