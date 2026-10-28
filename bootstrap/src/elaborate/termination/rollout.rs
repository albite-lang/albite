//! How much of the termination report gates a build (ADR 29.6.26e §4, flipped
//! to its target state by ADR 11.8.26b).
//!
//! Every rejection is an error. 29.6.26e shipped `ProofsOnly` as the default
//! because a hard gate would have rejected 557 of the self-hosted compiler's
//! definitions on the day it landed; 11.8.26b's annotation pass drove that
//! residual to zero, so the gate now gates. The knob survives as an escape
//! hatch for bisecting — `--termination proofs` restores the old behaviour, and
//! `report` silences the gate entirely — not as the way it is normally run.

/// Which failures are errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Enforcement {
    /// Every failure is an error.
    #[default]
    All,
    /// Only proof-relevant failures are errors — a proof may not reach an
    /// unchecked or partial constant. Executable code is reported, not gated.
    ProofsOnly,
    /// Nothing is an error; the report is informational.
    Report,
}

/// The setting selecting the enforcement level, as a suffix read through
/// `albite_runtime::settings` (ADR 20.10.26e D1): `ALBITE_TERMINATION`.
pub const ENFORCEMENT_VAR: &str = "TERMINATION";

impl Enforcement {
    /// Parse a knob value; an unrecognised one keeps the default rather than
    /// failing a build on a typo in an environment variable.
    ///
    /// Every level is spelled out and the fallback is `default()`, so flipping
    /// which variant is `#[default]` moves the typo case with it. Leaving
    /// `proofs` to be caught by a catch-all would have silently *weakened* the
    /// gate for every misspelling once ADR 11.8.26b made `all` the default.
    #[must_use]
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("all") => Enforcement::All,
            Some("proofs") => Enforcement::ProofsOnly,
            Some("report") => Enforcement::Report,
            _ => Enforcement::default(),
        }
    }

    /// Whether a failure at this proof-relevance is an error.
    #[must_use]
    pub fn gates(self, is_proof_failure: bool) -> bool {
        match self {
            Enforcement::All => true,
            Enforcement::ProofsOnly => is_proof_failure,
            Enforcement::Report => false,
        }
    }
}

/// Process-wide override, set from `--termination` before elaboration starts —
/// by the CLI, and nothing else (ADR 2.10.26b D2): a report tool's window is
/// [`WINDOW`], and a test stores only the default here.
///
/// A global rather than a threaded option because enforcement is a property of
/// the *run*, and the gate sits four call layers below the CLI; `set_max_errors`
/// is the same shape for the same reason. It stays process-wide because nothing
/// proves every command elaborates on the thread that parsed the flag, and a
/// silently ignored `--termination` would be worse than any race.
static OVERRIDE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(NO_OVERRIDE);

/// Sentinel for "no `--termination` was passed".
const NO_OVERRIDE: u8 = u8::MAX;

thread_local! {
    /// The level a [`ReportingOnly`] window forces, for its own thread only
    /// (ADR 2.10.26b D1).
    ///
    /// A window means "this tool's own elaboration", and every tool elaborates
    /// on the thread that opened it, so scoping it to that thread changes
    /// nothing a user sees. What it removes is the cross-thread reach a
    /// process-wide window had in the single multi-threaded test binary, in
    /// both directions ADR 12.8.26a §5.1 met: one test's window weakening
    /// another's gate, and another test's store landing mid-way through a
    /// window. Neither can happen now, so no lock serializes the readers — a
    /// lock only covered the readers someone remembered.
    static WINDOW: std::cell::Cell<Option<Enforcement>> = const { std::cell::Cell::new(None) };
}

/// Record a `--termination` value for the rest of the run.
pub fn set_enforcement(level: Enforcement) {
    OVERRIDE.store(level.as_code(), std::sync::atomic::Ordering::Relaxed);
}

/// Drop the override, restoring "no `--termination` was passed".
///
/// Only tests need this, and they need it as of ADR 11.8.26b: a test that
/// "restored" the old state by storing `ProofsOnly` used to be a no-op, because
/// that *was* the default. Now it would leave a weakened gate pinned for every
/// later test in the same process.
pub fn reset_enforcement() {
    OVERRIDE.store(NO_OVERRIDE, std::sync::atomic::Ordering::Relaxed);
}

/// Enforcement forced to `Report` on this thread for as long as the guard
/// lives, restoring whatever window was open before (including none) on drop.
///
/// **Why a report tool needs this (ADR 12.8.26a).** A `doctor check` whose whole
/// job is to print a census has to *reach* its census, and it reaches it by
/// elaborating the file first. Inheriting the build's enforcement level means a
/// file with a rejection aborts elaboration and the tool exits without ever
/// printing the thing the user ran it for — so making the gate stricter silently
/// deletes the diagnostic aimed at exactly the files the gate now rejects. That
/// happened when ADR 11.8.26b flipped the default to `All`.
///
/// The verdict is unaffected: `TerminationTally::exit` counts rejections from
/// the report, not from whether elaboration aborted, so the tool still exits
/// non-zero on a corpus it cannot certify.
///
/// The guard is `!Send` (ADR 2.10.26b D1): a window moved to another thread
/// would be dropped there, restoring that thread's slot and leaving this one's
/// forced to `Report` for good.
#[must_use = "enforcement is restored when the guard drops, so it must be bound"]
pub struct ReportingOnly {
    previous: Option<Enforcement>,
    _this_thread_only: std::marker::PhantomData<*const ()>,
}

impl ReportingOnly {
    /// Force `Report` on this thread until the returned guard drops.
    pub fn begin() -> Self {
        let previous = WINDOW.with(|window| window.replace(Some(Enforcement::Report)));
        ReportingOnly {
            previous,
            _this_thread_only: std::marker::PhantomData,
        }
    }
}

impl Drop for ReportingOnly {
    fn drop(&mut self) {
        WINDOW.with(|window| window.set(self.previous));
    }
}

impl Enforcement {
    /// Wire encoding for the process-wide override.
    const fn as_code(self) -> u8 {
        match self {
            Enforcement::All => 0,
            Enforcement::ProofsOnly => 1,
            Enforcement::Report => 2,
        }
    }

    /// Inverse of [`Enforcement::as_code`]; `None` for the sentinel.
    const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Enforcement::All),
            1 => Some(Enforcement::ProofsOnly),
            2 => Some(Enforcement::Report),
            _ => None,
        }
    }
}

/// The enforcement level for this thread's elaboration: a [`ReportingOnly`]
/// window if one is open here, else `--termination` if given, else the
/// environment variable, else the default.
///
/// **The caller-thread invariant (ADR 2.10.26b D1).** The level is read only on
/// the thread that called `elaborate_project` — the one a window was opened
/// on. A reader inside the pooled module walk must be *handed* the level, not
/// read it, or it would silently miss its caller's window; the debug assertion
/// makes that mistake loud in every debug test run.
#[must_use]
pub fn enforcement() -> Enforcement {
    debug_assert!(
        rayon::current_thread_index().is_none(),
        "the enforcement level is read on elaborate_project's caller thread, never a pool worker"
    );
    resolve(
        WINDOW.with(std::cell::Cell::get),
        Enforcement::from_code(OVERRIDE.load(std::sync::atomic::Ordering::Relaxed)),
        albite_runtime::settings::read(ENFORCEMENT_VAR).as_deref(),
    )
}

/// Whether a [`ReportingOnly`] window is open on this thread — the window
/// alone, never `--termination` or the environment (ADR 2.10.26d D2). The
/// kernel gate reads this: a census must reach a file whose theorem the
/// kernel refuses, and `--termination report` must not silence the kernel.
/// Read on the caller thread, like [`enforcement`].
#[must_use]
pub fn reporting_window_open() -> bool {
    debug_assert!(
        rayon::current_thread_index().is_none(),
        "the reporting window is read on elaborate_project's caller thread, never a pool worker"
    );
    WINDOW.with(std::cell::Cell::get).is_some()
}

/// The precedence [`enforcement`] applies, as a pure function: window, then
/// CLI, then environment, then default (ADR 2.10.26b D2).
pub(super) fn resolve(
    window: Option<Enforcement>,
    cli: Option<Enforcement>,
    env: Option<&str>,
) -> Enforcement {
    window.or(cli).unwrap_or_else(|| Enforcement::parse(env))
}

/// The `--termination` override as stored, for tests that must see the
/// process-wide half without reading through a window or the environment.
#[cfg(test)]
pub(super) fn cli_override() -> Option<Enforcement> {
    Enforcement::from_code(OVERRIDE.load(std::sync::atomic::Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_gates_every_rejection() {
        // ADR 11.8.26b: the target state, reached once the corpus annotation
        // pass drove the self-hosted residual to zero.
        let level = Enforcement::default();
        assert_eq!(level, Enforcement::All);
        assert!(level.gates(true));
        assert!(level.gates(false));
    }

    #[test]
    fn proofs_only_still_demotes_executable_rejections() {
        assert!(Enforcement::ProofsOnly.gates(true));
        assert!(!Enforcement::ProofsOnly.gates(false));
    }

    #[test]
    fn all_gates_both_kinds_and_report_gates_neither() {
        assert!(Enforcement::All.gates(true));
        assert!(Enforcement::All.gates(false));
        assert!(!Enforcement::Report.gates(true));
        assert!(!Enforcement::Report.gates(false));
    }

    #[test]
    fn the_knob_parses_its_three_values() {
        assert_eq!(Enforcement::parse(Some("all")), Enforcement::All);
        assert_eq!(Enforcement::parse(Some("report")), Enforcement::Report);
        assert_eq!(Enforcement::parse(Some("proofs")), Enforcement::ProofsOnly);
    }

    #[test]
    fn an_unset_or_misspelled_knob_keeps_the_default() {
        // Not spelled `Enforcement::All` on purpose: the property is "tracks the
        // default", and writing the variant would let a future flip pass while
        // the typo case silently stayed behind.
        for knob in [None, Some("alll"), Some(""), Some("proof")] {
            assert_eq!(Enforcement::parse(knob), Enforcement::default(), "{knob:?}");
        }
    }

    #[test]
    fn surrounding_whitespace_is_tolerated() {
        assert_eq!(Enforcement::parse(Some("  all  ")), Enforcement::All);
    }

    #[test]
    fn the_override_encoding_round_trips_and_rejects_the_sentinel() {
        for level in [
            Enforcement::All,
            Enforcement::ProofsOnly,
            Enforcement::Report,
        ] {
            assert_eq!(Enforcement::from_code(level.as_code()), Some(level));
        }
        assert_eq!(Enforcement::from_code(NO_OVERRIDE), None);
    }

    /// ADR 2.10.26b AC1: another thread's window does not reach this one.
    ///
    /// Two barriers force the interleaving: the holder opens its window before
    /// the first, this thread reads between them, and the holder closes its
    /// window only after the second — so the read below always lands inside
    /// the window, and a red run never leaves one open.
    #[test]
    fn a_reporting_window_on_one_thread_is_invisible_to_another() {
        use std::sync::{Arc, Barrier};
        let window_open = Arc::new(Barrier::new(2));
        let read_done = Arc::new(Barrier::new(2));
        let holder = {
            let (window_open, read_done) = (Arc::clone(&window_open), Arc::clone(&read_done));
            std::thread::spawn(move || {
                let window = ReportingOnly::begin();
                let holder_read = enforcement();
                window_open.wait();
                read_done.wait();
                drop(window);
                holder_read
            })
        };
        window_open.wait();
        let other_read = enforcement();
        read_done.wait();
        let holder_read = holder.join().expect("the window holder does not panic");

        assert_ne!(
            other_read,
            Enforcement::Report,
            "the window leaked across threads"
        );
        assert_eq!(
            other_read,
            Enforcement::parse(albite_runtime::settings::read(ENFORCEMENT_VAR).as_deref())
        );
        assert_eq!(
            holder_read,
            Enforcement::Report,
            "the window applies to its own thread"
        );
    }

    /// ADR 2.10.26b AC3: a window still governs its own thread, nests, and
    /// unwinds to exactly what was there before. Run on a fresh thread so the
    /// base is this thread's empty slot, whatever the test thread holds.
    #[test]
    fn a_window_applies_to_its_own_thread_and_restores_on_drop() {
        std::thread::spawn(|| {
            let base = enforcement();
            assert_eq!(WINDOW.with(std::cell::Cell::get), None, "no window yet");

            let outer = ReportingOnly::begin();
            assert_eq!(enforcement(), Enforcement::Report);
            let inner = ReportingOnly::begin();
            assert_eq!(enforcement(), Enforcement::Report);
            drop(inner);
            assert_eq!(
                enforcement(),
                Enforcement::Report,
                "the outer window survives"
            );
            assert_eq!(WINDOW.with(std::cell::Cell::get), Some(Enforcement::Report));
            drop(outer);

            assert_eq!(
                WINDOW.with(std::cell::Cell::get),
                None,
                "the slot is emptied"
            );
            assert_eq!(enforcement(), base, "back to the base");
        })
        .join()
        .expect("the window round-trips on its own thread");
    }

    /// ADR 2.10.26b AC6: a pooled reader fails loudly rather than silently
    /// missing its caller's window. Debug builds only, as the assertion is.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "never a pool worker")]
    fn reading_the_level_on_a_pooled_thread_panics() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .expect("a one-thread pool");
        let _ = pool.install(enforcement);
    }

    /// ADR 2.10.26d D2: the kernel gate's accessor sees the window and only
    /// the window — a `--termination report` override leaves it closed.
    #[test]
    fn the_window_accessor_ignores_the_cli_override() {
        std::thread::spawn(|| {
            assert!(!reporting_window_open(), "no window yet");
            let window = ReportingOnly::begin();
            assert!(reporting_window_open());
            drop(window);
            assert!(!reporting_window_open(), "closed on drop");
            // The override is process-wide, so it is compared, not stored.
            assert_eq!(
                resolve(None, Some(Enforcement::Report), None),
                Enforcement::Report
            );
            assert!(!reporting_window_open(), "an override is not a window");
        })
        .join()
        .expect("the accessor reads its own thread's window");
    }
}
