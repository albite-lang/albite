//! The one line the old name prints, and when it stays quiet (ADR 20.10.26d D1).

/// The deprecation line, printed once per invocation of `tungsten`.
pub const DEPRECATION_NOTICE: &str =
    "note: `tungsten` is now `albite`; this name is removed in 2.1";

/// The notice for one invocation's arguments (without `argv[0]`), or `None`
/// when they ask for JSON errors — a machine consumer's stderr is unchanged.
pub fn notice(args: &[String]) -> Option<String> {
    if wants_json_errors(args) {
        None
    } else {
        Some(DEPRECATION_NOTICE.to_string())
    }
}

/// `--error-format json`, as two tokens or as `--error-format=json`.
fn wants_json_errors(args: &[String]) -> bool {
    args.iter().enumerate().any(|(index, arg)| {
        arg == "--error-format=json"
            || (arg == "--error-format" && args.get(index + 1).is_some_and(|value| value == "json"))
    })
}
