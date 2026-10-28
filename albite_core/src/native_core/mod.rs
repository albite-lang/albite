//! The native core's export table (ADR 4.10.26n D2, D9).
//!
//! A `.tg` function compiles to `<name>`, `<name>$direct`, `<name>$direct_mt`
//! and lambdas, none of which a Rust or C caller can name. This table closes
//! that gap: each row names a C symbol and the core definition it exports,
//! and the bootstrap code generator (`albite_codegen`'s `native_core`)
//! defines the symbol as a shim over the definition's `$direct`.
//!
//! The table is plain data here, not in the code generator, so the readers
//! that need no LLVM — `tungsten info ffi exports`, which lists which exports
//! are now written in Tungsten — read the same rows the code generator emits.

/// The prefix every non-exported symbol of the core's code-generation run
/// carries, forced, so the core's names can never meet a program's at link.
pub const NATIVE_CORE_PREFIX: &str = "__albite_native_core__";

/// One fixed C symbol defined by the native core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeCoreExport {
    /// The C symbol the shim defines.
    pub symbol: &'static str,
    /// The core definition it calls, as the core's elaboration names it.
    pub definition: &'static str,
    /// The Rust export it can stand in for. The shim takes that export's
    /// declared signature, so every call site's ABI is unchanged.
    pub shadows: &'static str,
    /// Whether `ALBITE_NATIVE_CORE=on` enables this row (ADR 6.10.26n D5);
    /// `all` enables every row, so a row is proved before it is on by default.
    pub on_by_default: bool,
}

/// Every export written in Tungsten: the concat (4.10.26n D1), its owned
/// twin (6.10.26n D4), the integer trap (7.10.26q D3) and the program-argument
/// init (7.10.26r D3).
pub const NATIVE_CORE_EXPORTS: &[NativeCoreExport] = &[
    // On by default (ADR 9.10.26n D5): with the 16-byte trailer and the
    // unfilled allocation both micro shapes and the `main.tg` budget held.
    NativeCoreExport {
        symbol: "tg_native_string_concat",
        definition: "string_concat",
        shadows: "tg_string_concat",
        on_by_default: true,
    },
    // On by default (ADR 16.10.26j D7): with the grow amortised
    // (`__albite_realloc_amortised`, F = 5/2) the owned shape measured
    // 0.95–1.02× Rust's against 6.10.26n D6's 1.10×, and `main.tg` 0.999×
    // with RSS +0.15%.
    NativeCoreExport {
        symbol: "tg_native_string_concat_owned",
        definition: "string_concat_owned",
        shadows: "tg_string_concat_owned",
        on_by_default: true,
    },
    // The integer trap (ADR 7.10.26q D3): cold, so its rule is parity and
    // the `main.tg` budget, not speed (D5), and both held.
    NativeCoreExport {
        symbol: "tg_native_int_trap",
        definition: "int_trap",
        shadows: "tg_int_trap",
        on_by_default: true,
    },
    // The program-argument init (ADR 7.10.26r D3): it publishes `argc` and
    // `argv` into the store the Rust readers read, with no copy (D1). On by
    // default: parity, the startup budget and the `main.tg` budget held (D5).
    NativeCoreExport {
        symbol: "tg_native_init_args_c",
        definition: "init_args_c",
        shadows: "tg_init_args_c",
        on_by_default: true,
    },
];

/// One core-reserved runtime intrinsic (ADR 7.10.26q D1): a call the code
/// generator lowers inline — to one libc call, because no Tungsten `extern`
/// can state libc's ABI (a `String` is `{ptr, len}`, a descriptor a C `int`),
/// or to atomic accesses of the argument store, which Tungsten cannot name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeIntrinsic {
    /// `tg_core_write_stderr(s: String) -> Nat`: `write(2, s.ptr, s.len)`.
    WriteStderr,
    /// `tg_core_abort() -> Unit`: `abort()`, then `unreachable`.
    Abort,
    /// `tg_core_publish_args(argc: Nat, argv: Nat) -> Unit`: publish the pair
    /// into `tg_core_args_count`/`tg_core_args_vector` once (ADR 7.10.26r D2).
    PublishArgs,
}

/// The fixed list the core's rules admit beside the byte intrinsics. A row is
/// a rule change, so each later one needs its own ADR (roadmap.7.10.26d D4).
pub const CORE_RUNTIME_INTRINSICS: &[RuntimeIntrinsic] = &[
    RuntimeIntrinsic::WriteStderr,
    RuntimeIntrinsic::Abort,
    RuntimeIntrinsic::PublishArgs,
];

impl RuntimeIntrinsic {
    /// The `extern "C"` name the core declares.
    #[must_use]
    pub fn symbol(self) -> &'static str {
        match self {
            RuntimeIntrinsic::WriteStderr => "tg_core_write_stderr",
            RuntimeIntrinsic::Abort => "tg_core_abort",
            RuntimeIntrinsic::PublishArgs => "tg_core_publish_args",
        }
    }

    /// The row `symbol` names, accepting the elaborator's `__c_` prefix.
    #[must_use]
    pub fn from_symbol(symbol: &str) -> Option<RuntimeIntrinsic> {
        let bare = symbol.strip_prefix("__c_").unwrap_or(symbol);
        CORE_RUNTIME_INTRINSICS
            .iter()
            .copied()
            .find(|intrinsic| intrinsic.symbol() == bare)
    }
}

/// Which [`NATIVE_CORE_EXPORTS`] rows a `ALBITE_NATIVE_CORE` setting
/// enables (ADR 6.10.26n D5). The compile driver's and the code generator's
/// switches map onto it, so the rule is stated once, here, with no LLVM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnabledRows {
    /// `off`: no row.
    None,
    /// `on`: the rows with `on_by_default`.
    Defaults,
    /// `all`: every row.
    Every,
}

impl EnabledRows {
    /// Every setting, once.
    pub const ALL: [EnabledRows; 3] =
        [EnabledRows::None, EnabledRows::Defaults, EnabledRows::Every];

    /// Whether this setting enables `row`.
    #[must_use]
    pub fn enables(self, row: &NativeCoreExport) -> bool {
        match self {
            EnabledRows::None => false,
            EnabledRows::Defaults => row.on_by_default,
            EnabledRows::Every => true,
        }
    }
}

/// The C symbol a call to the Rust export `shadowed` takes under `enabled`
/// over `rows`: the enabled row's shim, or `shadowed` itself.
#[must_use]
pub fn shadowing_symbol(
    rows: &[NativeCoreExport],
    enabled: EnabledRows,
    shadowed: &'static str,
) -> &'static str {
    rows.iter()
        .find(|row| row.shadows == shadowed && enabled.enables(row))
        .map_or(shadowed, |row| row.symbol)
}

/// The invariant the owned export's adoption rests on (ADR 6.10.26n D2): an
/// enabled row shadowing `tg_string_concat_owned` implies an enabled row
/// shadowing `tg_string_concat`, so an owned `++`'s left operand came from the
/// core's run and its block is `core_buffer_layout`'s.
#[must_use]
pub fn owned_implies_concat(rows: &[NativeCoreExport], enabled: EnabledRows) -> bool {
    let shadows_enabled = |shadowed: &str| {
        rows.iter()
            .any(|row| row.shadows == shadowed && enabled.enables(row))
    };
    !shadows_enabled("tg_string_concat_owned") || shadows_enabled("tg_string_concat")
}

/// Whether `symbol` is one of [`NATIVE_CORE_EXPORTS`]' C symbols.
#[must_use]
pub fn is_native_core_export(symbol: &str) -> bool {
    NATIVE_CORE_EXPORTS.iter().any(|row| row.symbol == symbol)
}

/// The name the core's run gives `name`: unchanged for an export row, else
/// prefixed. Idempotent, so a second pass renames nothing.
#[must_use]
pub fn native_core_symbol_name(name: &str) -> String {
    if is_native_core_export(name) || name.starts_with(NATIVE_CORE_PREFIX) {
        name.to_string()
    } else {
        format!("{NATIVE_CORE_PREFIX}{name}")
    }
}

/// The core's trailer size: `{ len: i64, live: i64 }` (ADR 9.10.26n D1).
/// `data` is not stored: it is the block's start, `trailer − header_offset`.
/// A user program's separate header keeps all three fields, at the code
/// generator's own `HEADER_BYTES`.
pub const CORE_TRAILER_BYTES: u64 = 16;

/// The trailer's alignment, and the granule the data region rounds up to.
pub const BUFFER_HEADER_ALIGN: u64 = 8;

/// One block for a `len`-byte buffer in the core's own run (ADR 6.10.26m D2,
/// 9.10.26n D1): `(block_bytes, header_offset)`. Data starts at offset 0, so
/// a converted string's pointer is still an allocation start; the trailer is
/// at `round_up_8(len + 1)` — one spare byte, so the trailer always sits past
/// the data — and is [`CORE_TRAILER_BYTES`] long, so a 7-byte buffer asks for
/// 24 bytes. `None` above `BYTES_MAX_LEN`, the ceiling `tg_bytes_new` traps on
/// before any size is computed, so the sum cannot wrap.
#[must_use]
pub fn core_buffer_layout(len: u64) -> Option<(u64, u64)> {
    if len > crate::eval::byte_intrinsics::BYTES_MAX_LEN {
        return None;
    }
    let header_offset = (len + BUFFER_HEADER_ALIGN) & !(BUFFER_HEADER_ALIGN - 1);
    Some((header_offset + CORE_TRAILER_BYTES, header_offset))
}

#[cfg(test)]
mod tests;
