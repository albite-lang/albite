//! `ALBITE_NATIVE_CORE`, read once (ADR 4.10.26n D6, 6.10.26n D5).
//!
//! An environment variable rather than a flag, because it must cross two
//! process boundaries unchanged: `albite1 compile` spawns the bootstrap,
//! and `diff exec` spawns `albite compile`. `ALBITE_CODEGEN_JOBS` is the
//! precedent. Parsing is a pure function, so an unset variable, the three
//! accepted values and every other value are each tested without touching
//! the process environment.

/// The setting's suffix, read through `albite_runtime::settings` (ADR
/// 20.10.26e D1): `ALBITE_NATIVE_CORE`.
pub const NATIVE_CORE_ENV: &str = "NATIVE_CORE";

/// Which export rows a `++` may call, and whether the core is linked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeCoreChoice {
    /// The Rust exports, exactly today's IR; no core run.
    Off,
    /// The rows with `on_by_default`, written in Tungsten; the core is linked.
    On,
    /// Every row, so parity and measurement can run before a row is on by
    /// default (6.10.26n D5); the core is linked.
    All,
}

/// The default: `on` (ADR 7.10.26q D5), so an unset variable enables exactly
/// the rows that met their rules — today the integer trap alone.
pub const NATIVE_CORE_DEFAULT: NativeCoreChoice = NativeCoreChoice::On;

/// Parse the variable's value: unset is [`NATIVE_CORE_DEFAULT`], `on`, `off`
/// and `all` are themselves, and anything else is an error naming all three —
/// never a silent default.
///
/// # Errors
/// A value other than `on`, `off` or `all`, the empty string included.
pub fn native_core_choice(value: Option<&str>) -> Result<NativeCoreChoice, String> {
    match value {
        None => Ok(NATIVE_CORE_DEFAULT),
        Some("on") => Ok(NativeCoreChoice::On),
        Some("off") => Ok(NativeCoreChoice::Off),
        Some("all") => Ok(NativeCoreChoice::All),
        Some(other) => Err(format!(
            "{}{NATIVE_CORE_ENV}={other:?} is not a setting: use `on`, `off` or `all` \
             (unset means `on`)",
            albite_runtime::settings::PREFIX
        )),
    }
}

/// Read the variable from this process's environment, once per call site.
///
/// # Errors
/// As [`native_core_choice`].
pub fn native_core_choice_from_env() -> Result<NativeCoreChoice, String> {
    native_core_choice(albite_runtime::settings::read(NATIVE_CORE_ENV).as_deref())
}
