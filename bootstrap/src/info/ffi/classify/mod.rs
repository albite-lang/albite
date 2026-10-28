//! The trust classes of a core export, decided from its body's facts (ADR
//! 5.10.26f D2).
//!
//! 1.10.26k's D2 fixed three classes and a first-match order — **kernel**,
//! then **runtime**, then **pure** — and recorded a *basis* per export: the
//! call or state that decided it. This module is that rule as a pure function
//! over [`Facts`], plus a fourth verdict, **unknown**, for a body that no
//! kernel or runtime fact decided and that could not be read in full. A body
//! that cannot be read is never classed pure.
//!
//! The tables below say which calls and which state are facts. They are the
//! reviewable half of the classifier: `extract` finds the calls, these decide
//! what each one means.

use std::fmt;

pub(crate) mod extract;
mod observe;
pub(crate) mod reach;
pub(crate) mod walk;

#[cfg(test)]
mod expectations;
#[cfg(test)]
mod oracle_tests;
#[cfg(test)]
mod tests;

/// One of the four verdicts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Class {
    /// Reads or writes the kernel's terms, types or contexts, or calls the
    /// checker, conversion or the evaluator.
    Kernel,
    /// Needs the operating system, or state that persists across calls.
    Runtime,
    /// A function of its argument values alone.
    Pure,
    /// Nothing decided it, and part of the body could not be read.
    Unknown,
}

impl Class {
    /// The record's spelling of the class.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Class::Kernel => "kernel",
            Class::Runtime => "runtime",
            Class::Pure => "pure",
            Class::Unknown => "unknown",
        }
    }

    /// Parse the record's spelling; `None` for anything else.
    #[cfg(test)]
    pub(crate) fn from_label(label: &str) -> Option<Class> {
        [Class::Kernel, Class::Runtime, Class::Pure, Class::Unknown]
            .into_iter()
            .find(|class| class.label() == label)
    }
}

impl fmt::Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// One thing the body (or a helper it reaches) was seen to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fact {
    /// The call or state, as written: `with_arena`, `fs::read`, `CLI_ARGS`.
    pub(crate) token: String,
    /// The helper the export called to reach it, when not its own body.
    pub(crate) via: Option<String>,
}

impl Fact {
    pub(crate) fn direct(token: impl Into<String>) -> Fact {
        Fact {
            token: token.into(),
            via: None,
        }
    }

    /// The same fact, reached through a call to `helper`.
    pub(crate) fn through(&self, helper: &str) -> Fact {
        Fact {
            token: self.token.clone(),
            via: Some(helper.to_string()),
        }
    }
}

impl fmt::Display for Fact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.via {
            Some(helper) => write!(f, "`{}` via `{helper}`", self.token),
            None => write!(f, "`{}`", self.token),
        }
    }
}

/// Everything the extraction found in one export's reach, by kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    pub(crate) kernel: Vec<Fact>,
    pub(crate) runtime: Vec<Fact>,
    /// Parts that could not be read: an unparsable macro, an unresolved call.
    pub(crate) unreadable: Vec<Fact>,
}

impl Facts {
    /// Fold `other` in, each fact marked as reached through `helper`.
    pub(crate) fn absorb_through(&mut self, other: &Facts, helper: &str) {
        let lift = |facts: &[Fact]| -> Vec<Fact> {
            facts.iter().map(|fact| fact.through(helper)).collect()
        };
        self.kernel.extend(lift(&other.kernel));
        self.runtime.extend(lift(&other.runtime));
        self.unreadable.extend(lift(&other.unreadable));
    }

    /// Fold in a `cfg` twin's facts unchanged: one export, its worst arm.
    pub(crate) fn absorb(&mut self, other: Facts) {
        self.kernel.extend(other.kernel);
        self.runtime.extend(other.runtime);
        self.unreadable.extend(other.unreadable);
    }
}

/// A class and the facts of that class that decided it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Verdict {
    pub(crate) class: Class,
    /// Every fact of the deciding kind, in the order they were found. Empty
    /// for pure, which no fact decides.
    pub(crate) deciding: Vec<Fact>,
}

impl Verdict {
    /// The basis column: the first deciding fact, and how many more stood.
    pub(crate) fn basis(&self) -> String {
        match self.deciding.split_first() {
            None => "no kernel or runtime fact".to_string(),
            Some((first, [])) => first.to_string(),
            Some((first, rest)) => format!("{first} (+{} more)", rest.len()),
        }
    }
}

/// 1.10.26k's D2, first match wins, with unknown before pure.
pub(crate) fn classify_export(facts: &Facts) -> Verdict {
    let (class, deciding) = if !facts.kernel.is_empty() {
        (Class::Kernel, &facts.kernel)
    } else if !facts.runtime.is_empty() {
        (Class::Runtime, &facts.runtime)
    } else if !facts.unreadable.is_empty() {
        (Class::Unknown, &facts.unreadable)
    } else {
        (Class::Pure, &facts.kernel)
    };
    Verdict {
        class,
        deciding: deciding.clone(),
    }
}

// ============================================================================
// What counts as a fact
// ============================================================================

/// Macros that open the kernel arena (`albite_core::ffi`).
pub(crate) const KERNEL_MACROS: [&str; 2] = ["with_arena", "with_arena_ref"];

/// Persistent state that is the kernel's own: the arena, the evaluator's
/// environment table and the two checkers' registries. Any other `static` or
/// `thread_local!` the crates define is runtime state.
pub(crate) const KERNEL_STATICS: [&str; 3] = ["ARENA", "EVAL_ENVS", "REGISTRY"];

/// `albite_core` types whose methods are the kernel's data: a call to one
/// is a kernel fact (`alloc_ctx`, `get_ctx`) and is not followed further.
pub(crate) const KERNEL_TYPES: [&str; 1] = ["Arena"];

/// `albite_core` modules holding the checker, conversion and the evaluator.
/// A call into one is a kernel fact and is not followed further.
pub(crate) const KERNEL_MODULES: [&str; 3] = ["typecheck", "conversion", "eval"];

/// The shared allocator. 1.10.26k's D2: reaching the arena mode through it
/// does not make a body runtime, so a call into it is no fact and is not
/// followed.
pub(crate) const ALLOCATOR_MODULE: &str = "c_allocator";
pub(crate) const ALLOCATOR_ENTRIES: [&str; 3] =
    ["__albite_alloc", "__albite_realloc", "__albite_free"];

/// Console macros: each writes stdout or stderr.
pub(crate) const CONSOLE_MACROS: [&str; 5] = ["print", "println", "eprint", "eprintln", "dbg"];

/// `std` modules and types whose every call reaches the operating system.
const OS_STD_SEGMENTS: [&str; 7] = ["fs", "env", "process", "Command", "File", "net", "thread"];

/// `std` calls named by their last two segments that reach the OS or a clock.
const OS_STD_CALLS: [&str; 6] = [
    "io::stdout",
    "io::stderr",
    "io::stdin",
    "Instant::now",
    "SystemTime::now",
    "Path::exists",
];

/// Methods that reach the filesystem whatever their receiver is named.
const OS_METHODS: [&str; 7] = [
    "exists",
    "is_dir",
    "is_file",
    "metadata",
    "read_dir",
    "canonicalize",
    "elapsed",
];

/// Crates outside the workspace whose calls are OS calls.
const OS_CRATES: [&str; 2] = ["libc", "atty"];

/// A runtime fact for a call into `std`/`core`/`alloc`, if it reaches the OS.
/// `segments` is the call's path with its crate root already stripped.
pub(crate) fn os_fact_for_std_call(segments: &[String]) -> Option<Fact> {
    let reaches_os = segments
        .iter()
        .any(|segment| OS_STD_SEGMENTS.contains(&segment.as_str()))
        || OS_STD_CALLS.contains(&last_two(segments).as_str());
    reaches_os.then(|| Fact::direct(last_two(segments)))
}

/// A runtime fact for a method call no crate defines, if it is a filesystem
/// or clock query.
pub(crate) fn os_fact_for_method(name: &str) -> Option<Fact> {
    OS_METHODS
        .contains(&name)
        .then(|| Fact::direct(format!(".{name}()")))
}

/// Whether a call into a crate outside the workspace is an OS call.
pub(crate) fn is_os_crate(root: &str) -> bool {
    OS_CRATES.contains(&root)
}

/// `a::b::c` → `b::c`; a single segment is itself.
pub(crate) fn last_two(segments: &[String]) -> String {
    let start = segments.len().saturating_sub(2);
    segments[start..].join("::")
}
