//! Rendering for `doctor check module name-collisions` — human report and JSON.
//!
//! The report shape is ADR 12.7.26b's, deliberately: every candidate listed,
//! each with the Core name it elaborates under (ADR 4.10.26j D8). A reader who has seen `extern-map-ambiguity` can
//! read this without learning a second vocabulary (§1.4).

use crate::ast::Visibility;

use super::census::{Census, Collision, CollisionClass, DefinitionSite, Severity};

/// The reach line: what the run actually looked at.
///
/// Printed on every run, findings or not. `0 collisions` and `0 modules
/// examined` are different outcomes and must not render alike (§2.1). A run
/// that mounted packages says so (ADR 15.10.26n D4), so `examined 1
/// module(s)` on a manifest project cannot pass for a complete census.
pub fn reach_line(census: &Census) -> String {
    let packages = match census.packages_mounted {
        0 => String::new(),
        mounted => format!(" across {mounted} package(s)"),
    };
    format!(
        "  examined {} module(s){packages}, {} distinct value name(s) across {} definition site(s)\n",
        census.modules_examined, census.names_considered, census.definitions_considered
    )
}

/// The whole human report, as a value so every branch is assertable.
pub fn render_human(census: &Census, reported: &[&Collision], severity: Severity) -> String {
    let mut out = String::new();
    if reported.is_empty() {
        out.push_str(&format!(
            "✓ No name collisions ({}).\n",
            scope_label(severity)
        ));
    } else {
        out.push_str(&format!(
            "! {} name collision(s) ({}):\n",
            reported.len(),
            scope_label(severity)
        ));
        for collision in reported {
            out.push_str(&render_collision(collision));
        }
    }
    out.push_str(&reach_line(census));
    if !reported.is_empty() {
        out.push_str(ADVISORY);
    }
    out
}

/// Why the exit code is 0 anyway (D3). Said on findings, so nobody reads the
/// zero as "and therefore fine".
const ADVISORY: &str =
    "  advisory: exit is 0 even with findings — read `--json` to gate on them (ADR 13.8.26c D3)\n";

/// What `--severity` selected, in words.
fn scope_label(severity: Severity) -> &'static str {
    match severity {
        Severity::All => "all classes",
        Severity::Live => "live classes only: extern-symbol",
    }
}

/// One finding: the name, its class, and every defining module with the Core
/// name it elaborates under.
fn render_collision(collision: &Collision) -> String {
    let mut out = format!("\n  `{}` [{}]\n", collision.name, collision.class.label());
    for site in &collision.sites {
        out.push_str(&format!(
            "    {}{}   as `{}`\n",
            site.module,
            site_suffix(site),
            site.core_name
        ));
    }
    out.push_str(&format!("    → {}\n", remedy(collision)));
    out
}

/// The visibility and extern-ness of one site, in the compact form.
fn site_suffix(site: &DefinitionSite) -> String {
    let visibility = match site.visibility {
        Visibility::Private => "private",
        Visibility::Crate => "pub(crate)",
        Visibility::Public => "pub",
    };
    if site.is_extern_c {
        format!("  ({visibility}, extern \"C\")")
    } else {
        format!("  ({visibility})")
    }
}

/// What to do about a finding: across packages an extern pair is the build's
/// own refusal (ADR 4.10.26j D6, D8).
fn remedy(collision: &Collision) -> &'static str {
    match (collision.class, collision.crosses_packages) {
        (CollisionClass::ExternSymbol, false) => {
            "the Albite identifier IS the C symbol, so no visibility change helps — rename one"
        }
        (CollisionClass::ExternSymbol, true) => {
            "two packages declare one C symbol: the build refuses it as E0123 — declare it once"
        }
        (CollisionClass::Scoped, _) => {
            "information: each definition has its own Core name; a qualified path names one, \
             and a bare call reaching two is E0019"
        }
    }
}

/// The machine-readable form. Same fields, so a gate can be built on it without
/// parsing prose (D3).
pub fn render_json(census: &Census, reported: &[&Collision], severity: Severity) -> String {
    let findings: Vec<serde_json::Value> = reported
        .iter()
        .map(|collision| {
            serde_json::json!({
                "name": collision.name,
                "class": collision.class.label(),
                "crosses_packages": collision.crosses_packages,
                "sites": collision.sites.iter().map(site_json).collect::<Vec<_>>(),
            })
        })
        .collect();
    let value = serde_json::json!({
        "severity": match severity { Severity::All => "all", Severity::Live => "live" },
        "modules_examined": census.modules_examined,
        "names_considered": census.names_considered,
        "definitions_considered": census.definitions_considered,
        "collision_count": reported.len(),
        "collisions": findings,
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&value).unwrap_or_default()
    )
}

/// One site as JSON.
fn site_json(site: &DefinitionSite) -> serde_json::Value {
    serde_json::json!({
        "module": site.module.to_string(),
        "visibility": match site.visibility {
            Visibility::Private => "private",
            Visibility::Crate => "crate",
            Visibility::Public => "pub",
        },
        "extern_c": site.is_extern_c,
        "core_name": site.core_name,
    })
}
