use super::*;
use albite_bootstrap::driver::diagnostics::compile_failure::{BuildErrorKind, CompileFailure};

#[test]
#[cfg(not(target_os = "macos"))]
fn test_lld_available_is_consistent() {
    // OnceLock caching: repeated calls must return the same value.
    let first = lld_available();
    let second = lld_available();
    assert_eq!(
        first, second,
        "lld_available should be deterministic (OnceLock)"
    );
}

#[test]
fn test_linker_command_exe_path_default() {
    let file = Path::new("/tmp/test.tg");
    let cmd = LinkerCommand::new(file, None, false, false);
    // Default: strip .tg extension
    assert_eq!(cmd.exe_path, PathBuf::from("/tmp/test"));
}

#[test]
fn test_linker_command_exe_path_custom_output() {
    let file = Path::new("/tmp/test.tg");
    let output = Path::new("/out/binary");
    let cmd = LinkerCommand::new(file, Some(output), false, false);
    assert_eq!(cmd.exe_path, PathBuf::from("/out/binary"));
}

#[test]
fn test_build_cc_preserves_input_order() {
    let file = Path::new("/tmp/test.tg");
    let linker = LinkerCommand::new(file, None, false, false);
    let inputs = vec![
        PathBuf::from("/tmp/b.o"),
        PathBuf::from("/tmp/a.o"),
        PathBuf::from("/tmp/c.o"),
    ];
    let cmd = linker.build_cc(&inputs);
    let args: Vec<_> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    // Inputs appear in the order provided (not sorted)
    let b_pos = args.iter().position(|a| a == "/tmp/b.o").unwrap();
    let a_pos = args.iter().position(|a| a == "/tmp/a.o").unwrap();
    let c_pos = args.iter().position(|a| a == "/tmp/c.o").unwrap();
    assert!(b_pos < a_pos, "b.o should appear before a.o");
    assert!(a_pos < c_pos, "a.o should appear before c.o");
}

#[test]
fn test_build_cc_includes_sanitize_flag() {
    let file = Path::new("/tmp/test.tg");
    let linker = LinkerCommand::new(file, None, true, false);
    let inputs = vec![PathBuf::from("/tmp/test.o")];
    let cmd = linker.build_cc(&inputs);
    let args: Vec<_> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    assert!(
        args.contains(&"-fsanitize=address".to_string()),
        "sanitized LinkerCommand should include -fsanitize=address"
    );
}

#[test]
fn test_build_cc_no_sanitize_by_default() {
    let file = Path::new("/tmp/test.tg");
    let linker = LinkerCommand::new(file, None, false, false);
    let inputs = vec![PathBuf::from("/tmp/test.o")];
    let cmd = linker.build_cc(&inputs);
    let args: Vec<_> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    assert!(
        !args.contains(&"-fsanitize=address".to_string()),
        "non-sanitized LinkerCommand should not include -fsanitize=address"
    );
}

#[test]
fn test_build_cc_includes_static_library() {
    let file = Path::new("/tmp/test.tg");
    let linker = LinkerCommand::new(file, None, false, false);
    let inputs = vec![PathBuf::from("/tmp/test.o")];
    let cmd = linker.build_cc(&inputs);
    let args: Vec<_> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    // Static linking (ADR 18.5.26e): full path to .a archive
    assert!(
        args.iter().any(|a| a.ends_with("libalbite_core.a")),
        "LinkerCommand should include libalbite_core.a static archive, got: {:?}",
        args,
    );
}

#[test]
fn test_build_cc_includes_platform_libs() {
    let file = Path::new("/tmp/test.tg");
    let linker = LinkerCommand::new(file, None, false, false);
    let inputs = vec![PathBuf::from("/tmp/test.o")];
    let cmd = linker.build_cc(&inputs);
    let args: Vec<_> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    // Platform-specific transitive C deps should be present.
    // macOS: cc implicitly links libSystem (provides libc + libm), so we
    // do NOT pass -lSystem explicitly — that causes a duplicate-library warning.
    #[cfg(target_os = "macos")]
    assert!(
        !args.contains(&"-lSystem".to_string()),
        "should NOT include -lSystem on macOS (implicit via cc), got: {:?}",
        args,
    );
    #[cfg(target_os = "linux")]
    assert!(
        args.contains(&"-lpthread".to_string()),
        "should include platform libs on Linux, got: {:?}",
        args,
    );
}

#[test]
fn test_build_cc_includes_stack_size() {
    let file = Path::new("/tmp/test.tg");
    let linker = LinkerCommand::new(file, None, false, false);
    let inputs = vec![PathBuf::from("/tmp/test.o")];
    let cmd = linker.build_cc(&inputs);
    let args: Vec<_> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    // macOS: ld64's flag sizes the main thread, which the runtime's fallback
    // runs on (ADR 20.5.26e, 19.10.26c D5). Linux: glibc ignores the
    // `-z stack-size` field, so ADR 19.10.26c D5 dropped the flag — the
    // runtime sizes the body's stack instead.
    #[cfg(target_os = "macos")]
    assert!(
        args.contains(&"-Wl,-stack_size,0x8000000".to_string()),
        "should include stack_size linker flag on macOS, got: {:?}",
        args,
    );
    #[cfg(target_os = "linux")]
    assert!(
        !args.iter().any(|a| a.contains("stack-size")),
        "the no-op -z stack-size flag must stay gone on Linux, got: {:?}",
        args,
    );
}

#[test]
#[cfg(target_os = "macos")]
fn test_build_cc_no_duplicate_system_libs_macos() {
    // Regression: cc (Apple Clang) implicitly links -lSystem. Passing it
    // explicitly causes "ld: warning: ignoring duplicate libraries: '-lSystem'".
    let file = Path::new("/tmp/test.tg");
    let linker = LinkerCommand::new(file, None, false, false);
    let inputs = vec![PathBuf::from("/tmp/test.o")];
    let cmd = linker.build_cc(&inputs);
    let args: Vec<_> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect();
    assert!(
        !args.contains(&"-lSystem".to_string()),
        "-lSystem must not be passed explicitly on macOS (implicit via cc)"
    );
    assert!(
        !args.contains(&"-lc".to_string()),
        "-lc must not be passed explicitly on macOS (provided by libSystem)"
    );
    assert!(
        !args.contains(&"-lm".to_string()),
        "-lm must not be passed explicitly on macOS (provided by libSystem)"
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Tool failures are CompileFailure values (ADR 30.9.26j AC2)
// ═══════════════════════════════════════════════════════════════════════

fn tool_failed(status: std::process::ExitStatus) -> CompileFailure {
    CompileFailure::build(BuildErrorKind::ToolFailed, format!("failed with {status}"))
}

fn tool_unavailable(error: std::io::Error) -> CompileFailure {
    CompileFailure::build(
        BuildErrorKind::ToolUnavailable,
        format!("could not run: {error}"),
    )
}

// 30.9.26j AC2: a tool that exits non-zero is ToolFailed (E0144).
#[test]
fn a_tool_that_exits_non_zero_is_tool_failed() {
    let mut cmd = Command::new("false");
    let failure = link::run_tool(&mut cmd, tool_failed, tool_unavailable).unwrap_err();
    assert_eq!(failure.code(), Some("E0144"));
}

// 30.9.26j AC2: a tool that cannot be spawned is ToolUnavailable (E0143).
#[test]
fn a_tool_that_cannot_be_spawned_is_tool_unavailable() {
    let mut cmd = Command::new("/nonexistent/tungsten-30-9-26j-tool");
    let failure = link::run_tool(&mut cmd, tool_failed, tool_unavailable).unwrap_err();
    assert_eq!(failure.code(), Some("E0143"));
}

#[test]
fn a_tool_that_succeeds_is_ok() {
    let mut cmd = Command::new("true");
    assert!(link::run_tool(&mut cmd, tool_failed, tool_unavailable).is_ok());
}

// 30.9.26j D3: the missing-archive diagnosis is carried verbatim.
#[test]
fn the_missing_archive_diagnosis_round_trips_verbatim() {
    let diagnosis =
        staticlib_check::missing_staticlib_diagnosis(Path::new("/nonexistent/libalbite_core.a"))
            .expect("a missing archive is diagnosed");
    let failure = CompileFailure::preformatted(BuildErrorKind::RuntimeLibraryUnusable, &diagnosis);
    assert_eq!(failure.human_lines().join("\n"), diagnosis);
    assert_eq!(failure.code(), Some("E0145"));
}

// 30.9.26j D3: a link against a missing archive fails before `cc` runs.
#[test]
fn run_link_without_the_archive_is_runtime_library_unusable() {
    let mut linker = LinkerCommand::new(Path::new("/tmp/test.tg"), None, false, false);
    linker.lib_dir = PathBuf::from("/nonexistent/tungsten-30-9-26j");
    let failure = linker.run_link(Command::new("true")).unwrap_err();
    assert_eq!(failure.code(), Some("E0145"));
}

// ═══════════════════════════════════════════════════════════════════════
// A program carries only the core code it calls (ADR 6.10.26i AC 2)
// ═══════════════════════════════════════════════════════════════════════

fn command_args(cmd: &Command) -> Vec<String> {
    cmd.get_args()
        .map(|a| a.to_string_lossy().to_string())
        .collect()
}

/// The current platform's strip flags, refusing an empty answer on the two
/// platforms that must strip — so a dropped flag fails, not passes vacuously.
fn current_strip_flags() -> &'static [&'static str] {
    let flags = strip_flags(std::env::consts::OS);
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        assert!(!flags.is_empty(), "macOS and Linux links must strip");
    }
    flags
}

// 6.10.26i AC 2: each OS spelling answers its own flag; any other answers none.
#[test]
fn strip_flags_answers_the_platforms_flag() {
    assert_eq!(strip_flags("macos"), ["-Wl,-dead_strip"]);
    assert_eq!(strip_flags("linux"), ["-Wl,--gc-sections"]);
    assert!(strip_flags("windows").is_empty());
}

// 6.10.26i AC 2: every user link through build_cc strips.
#[test]
fn build_cc_strips_unreferenced_sections() {
    let linker = LinkerCommand::new(Path::new("/tmp/test.tg"), None, false, false);
    let args = command_args(&linker.build_cc(&[PathBuf::from("/tmp/test.o")]));
    for flag in current_strip_flags() {
        assert!(args.iter().any(|a| a == flag), "{flag} missing: {args:?}");
    }
}

// 6.10.26i AC 2: the sanitizer's own clang link strips too.
#[test]
fn sanitize_command_strips_unreferenced_sections() {
    let linker = LinkerCommand::new(Path::new("/tmp/test.tg"), None, true, false);
    let cmd = sanitize::sanitize_command(
        &linker,
        Path::new("/tmp/module.ll"),
        &[PathBuf::from("/tmp/core.o")],
    );
    assert_eq!(cmd.get_program(), "clang");
    let args = command_args(&cmd);
    for flag in current_strip_flags() {
        assert!(args.iter().any(|a| a == flag), "{flag} missing: {args:?}");
    }
    // The command is otherwise the one emit_sanitized always ran.
    assert_eq!(args[0], "-fsanitize=address");
    assert_eq!(args[1], "/tmp/module.ll");
    assert_eq!(args[2], "/tmp/core.o");
    assert!(args.iter().any(|a| a.ends_with("libalbite_core.a")));
}
