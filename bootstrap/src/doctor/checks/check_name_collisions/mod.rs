//! `albite doctor check module name-collisions` — two definitions, one binding
//! (ADR 13.8.26c).
//!
//! **The failure this exists for.** Both compilers key their value environments
//! on the **bare name**. Two same-named items in one program are one entry; the
//! winner is whichever the module-tree walk registers last, and if the winner is
//! private then every call site of the loser reports **E0016 — in the loser's
//! file, naming the winner's module**. The error is therefore reported nowhere
//! near the edit that caused it, and its count scales with call sites rather
//! than with the mistake: 77 for one collision in ADR 7.8.26b. That ADR
//! documented the mechanism in its own D1 and then hit it twice more while
//! implementing its fix, which is the argument that reading about it does not
//! prevent it.
//!
//! **What it reports now (ADR 4.10.26j D8).** The bootstrap keys values by
//! (module, name) and gives a colliding name a scoped Core name, so a value
//! collision is `scoped` — information, printed with each module's Core
//! name, never a finding. Only an `extern "C"` pair stays live: its shared
//! identifier is one C symbol, and across packages the build refuses it as
//! E0123. The self-host still keys values by the bare name, so for a
//! `albite1` build a `scoped` collision still means what this paragraph's
//! predecessor said.
//!
//! **Why it is parse-only.** The whole value of the check is answering "why 77
//! E0016s?" on the file that produced them, and a check that needs successful
//! elaboration is unreachable on every input that needs it (ADR 12.8.26a). So
//! it runs `parse_module_tree` + `build_module_info` and touches nothing under
//! `elaborate/`. Cost 2. Between the two it mounts the manifest's packages with
//! the driver's own `mount_packages` (ADR 15.10.26n), itself parse-level, so a
//! repeat inside a package is counted like any other.
//!
//! **Why it is advisory.** `check-health` has enough blocking arms, and a new
//! one whose false-positive rate on a healthy `.tg` tree was unmeasured should
//! earn its exit code. Exit is 0 even with findings, under `--severity live` as
//! much as under `all`; a caller wanting a gate reads `--json`.

use std::collections::HashSet;
use std::path::Path;
use std::process::ExitCode;

use crate::ast::Item;
use crate::driver::modules::{
    build_module_info, collect_parse_errors, get_module_name_from_parsed, parse_module_tree,
    parse_workspace_modules,
};
use crate::driver::{packages, ParsedModule};
use crate::elaborate::ModulePath;

pub mod census;
pub mod report;

use census::{census, Census, Collision, ReexportHandling, Severity};

#[cfg(test)]
mod report_tests;
#[cfg(test)]
mod source_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

/// What one run produced, as a value so every branch is assertable.
///
/// `ExitCode` implements neither `PartialEq` nor any accessor, so a test on the
/// command's return value cannot distinguish its two arms at all — and the
/// arms differ, because bad input is the only thing this command ever exits
/// non-zero for (`.claude/CLAUDE.md` § Killing survivors).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The tree parsed and the census rendered. Findings or not, this is a
    /// successful run: the check is advisory (D3).
    Reported(String),
    /// The entry file's tree could not be parsed — bad input, not a finding.
    Unparsable(String),
}

/// Parse, census and render. The whole command except printing and exiting.
pub fn run(file: &Path, severity: Severity, json: bool, include_reexports: bool) -> Outcome {
    let census = match census_of_project(file, reexport_handling(include_reexports)) {
        Ok(census) => census,
        Err(message) => return Outcome::Unparsable(message),
    };
    let reported = select(&census, severity);
    Outcome::Reported(if json {
        report::render_json(&census, &reported, severity)
    } else {
        report::render_human(&census, &reported, severity)
    })
}

/// Entry point for `albite doctor check module name-collisions <file>`.
pub fn cmd_check_name_collisions(
    file: &Path,
    severity: Severity,
    json: bool,
    include_reexports: bool,
) -> ExitCode {
    match run(file, severity, json, include_reexports) {
        Outcome::Reported(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        // Bad input, not a finding — the audits' 0/1/2 contract. A findings exit
        // is 0 here (D3), so 2 is the only code this command ever returns
        // non-zero.
        Outcome::Unparsable(message) => {
            eprint!("{message}");
            ExitCode::from(2)
        }
    }
}

/// What `--include-reexports` selects. Named rather than inlined so the
/// default — subtract, the shipped behaviour — is assertable without a process.
pub fn reexport_handling(include_reexports: bool) -> ReexportHandling {
    if include_reexports {
        ReexportHandling::Keep
    } else {
        ReexportHandling::Subtract
    }
}

/// The findings `--severity` selects, in report order.
pub fn select(census: &Census, severity: Severity) -> Vec<&Collision> {
    census
        .collisions
        .iter()
        .filter(|collision| severity.admits(collision.class))
        .collect()
}

/// Collect every `(module, name)` that an `extern "C" fn` defines.
///
/// A separate walk because `ModuleContents` records visibility but not
/// extern-ness, and extern-ness is what separates D3's class (c) — a duplicate
/// *link* symbol, unfixable by any visibility change — from the rest.
fn extern_c_sites(module: &ParsedModule, path: &ModulePath) -> HashSet<(ModulePath, String)> {
    let mut sites = HashSet::new();
    collect_extern_c_sites(module, path, &mut sites);
    sites
}

/// Recursive helper for [`extern_c_sites`], mirroring the driver's own tree
/// walk so the module paths it produces are the same ones `ModuleInfo` keys on.
fn collect_extern_c_sites(
    module: &ParsedModule,
    path: &ModulePath,
    sites: &mut HashSet<(ModulePath, String)>,
) {
    for item in &module.source_file.items {
        if let Item::ExternFn(e) = item {
            sites.insert((path.clone(), e.name.name.clone()));
        }
    }
    for submodule in &module.submodules {
        let child = path.child(get_module_name_from_parsed(submodule));
        collect_extern_c_sites(submodule, &child, sites);
    }
}

/// Census an already-parsed tree — the one place the module info, the extern
/// walk and the multimap are wired together, so the command and the
/// `tool-reachability` probe cannot drift into censusing different things.
#[must_use]
pub fn census_of_tree(tree: &ParsedModule, reexports: ReexportHandling) -> Census {
    let info = build_module_info(tree);
    let externs = extern_c_sites(tree, &ModulePath::root());
    census(&info, &externs, reexports)
}

/// Parse `file`'s reachable tree and census it. `None` when the entry file
/// cannot be parsed or its packages cannot be mounted.
#[must_use]
pub fn census_of_file(file: &Path, reexports: ReexportHandling) -> Option<Census> {
    census_of_project(file, reexports).ok()
}

/// Parse `file`'s tree, mount the packages its manifest declares, and census
/// the result — the set both compilers' environments see (ADR 15.10.26n).
///
/// Mounting is `prepare`'s own `mount_packages`, not a second walker (D1), so
/// the census reaches exactly the packages the planner mounts (D6). Workspace
/// siblings are parsed only because the planner reads their names to place
/// packages; they are not counted (D5). Without a manifest nothing is mounted
/// and the census is today's (D2). `Err` is bad input with its message (D3).
fn census_of_project(file: &Path, reexports: ReexportHandling) -> Result<Census, String> {
    let mut visited = HashSet::new();
    let mut chain = Vec::new();
    let mut tree = parse_module_tree(file, &mut visited, &mut chain, None)
        .map_err(|e| format!("error: {e}\n"))?;

    let manifest = packages::load_manifest(file);
    let siblings = parse_workspace_modules(&packages::sibling_root(file, &manifest), None);
    let refused = |cause: String| {
        format!(
            "error: the manifest's packages could not be mounted ({cause}); \
             run `albite check {}` for the full diagnostic\n",
            file.display()
        )
    };
    // `mount_packages` renders its own refusals to stderr, so only the
    // pointer to the full diagnostic is added here.
    let mounted = packages::mount_packages(file, manifest, &mut tree, &siblings, None)
        .map_err(|e| refused(e.to_string()))?;
    // A package's parse errors ride inside its tree, as the entry's do; the
    // entry's are tolerated as today (the check exists for broken files),
    // but a package `check` would refuse is not the program it censuses.
    if let Some(broken) = unparsable_package_file(&tree) {
        return Err(refused(format!("{} does not parse", broken.display())));
    }

    let mut census = census_of_tree(&tree, reexports);
    // The first manifest is the application's own; every one after it is a
    // mounted package (`MountedPackages::announcement` counts the same way).
    census.packages_mounted = mounted.manifests.len().saturating_sub(1);
    Ok(census)
}

/// The first source file under a mounted package that has parse errors.
fn unparsable_package_file(tree: &ParsedModule) -> Option<std::path::PathBuf> {
    tree.submodules
        .iter()
        .filter(|module| module.mount_name.is_some())
        .flat_map(|library| collect_parse_errors(library, &mut Vec::new()))
        .map(|(path, _)| path)
        .next()
}
