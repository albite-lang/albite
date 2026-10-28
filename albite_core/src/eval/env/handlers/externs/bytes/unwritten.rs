//! Which bytes of a buffer are still unwritten (ADR 9.10.26n D3).
//!
//! `tg_bytes_new_uninit` and `tg_bytes_from_string_grown` leave their new
//! bytes unwritten natively, and only the core's static write-cover rule
//! proves every one is copied in before anything reads the buffer. Here the
//! evaluator keeps the second line: it marks each byte those two open as
//! unwritten, clears the mark on every store, and stops evaluation with
//! `EvalStopped::UnwrittenByteRead` when a `get` or a conversion reaches a
//! byte still marked. The bytes themselves are zeros, as before, so nothing
//! else about a buffer changes.

use std::collections::HashMap;

use crate::eval::env::{EvalStopped, IntTrapKind};

/// Why a byte intrinsic refused: a trap both executions print, or the
/// evaluator-only unwritten read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ByteStop {
    /// A refusal native code makes too, through `tg_int_trap`.
    Trap(IntTrapKind),
    /// A read of a byte no store has written (ADR 9.10.26n D3).
    UnwrittenRead { offset: u64 },
}

impl From<IntTrapKind> for ByteStop {
    fn from(kind: IntTrapKind) -> Self {
        ByteStop::Trap(kind)
    }
}

impl ByteStop {
    /// The stop recorded on the env.
    pub(crate) fn into_stop(self) -> EvalStopped {
        match self {
            ByteStop::Trap(kind) => EvalStopped::IntTrap { kind },
            ByteStop::UnwrittenRead { offset } => EvalStopped::UnwrittenByteRead { offset },
        }
    }
}

/// Per handle, one flag per byte, `true` while that byte is unwritten. A
/// handle with no entry has every byte written.
#[derive(Debug, Default)]
pub(crate) struct UnwrittenBytes {
    marks: HashMap<u64, Vec<bool>>,
}

impl UnwrittenBytes {
    /// Mark `[start, len)` of `handle`'s `len`-byte buffer unwritten.
    pub(crate) fn open(&mut self, handle: u64, len: usize, start: usize) {
        if start >= len {
            return;
        }
        let mut marks = vec![false; len];
        marks[start..].iter_mut().for_each(|mark| *mark = true);
        self.marks.insert(handle, marks);
    }

    /// Record a store to `[start, end)`.
    pub(crate) fn write(&mut self, handle: u64, start: usize, end: usize) {
        if let Some(marks) = self.marks.get_mut(&handle) {
            marks[start..end].iter_mut().for_each(|mark| *mark = false);
            if !marks.contains(&true) {
                self.marks.remove(&handle);
            }
        }
    }

    /// The first unwritten offset in `[start, end)`, if any.
    pub(crate) fn first_in(&self, handle: u64, start: usize, end: usize) -> Option<u64> {
        let marks = self.marks.get(&handle)?;
        marks[start..end]
            .iter()
            .position(|unwritten| *unwritten)
            .map(|position| (start + position) as u64)
    }

    /// Forget `handle`, which a conversion consumed.
    pub(crate) fn close(&mut self, handle: u64) {
        self.marks.remove(&handle);
    }
}
