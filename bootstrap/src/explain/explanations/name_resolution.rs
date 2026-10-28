//! Name resolution error explanations.
//!
//! Handles: `UndefinedVariable`, `UndefinedType`, `UndefinedConstructor`,
//! `DuplicateDefinition`, `ModuleNotFound`, `ItemNotFoundInModule`,
//! `DuplicateImport`, `GlobConflict`, `AmbiguousValue`, `UnresolvedImport`, `PrivateModule`,
//! `PrivateItem`, `PublicItemLeak`.

use crate::explain::error_catalogue::ErrorExplanation;

pub(super) fn name_resolution(name: &str) -> Option<ErrorExplanation> {
    binding_errors(name)
        .or_else(|| module_errors(name))
        .or_else(|| visibility_errors(name))
        .or_else(|| package_errors(name))
}

/// The *Packages* category (ADR 29.9.26g): a `albite.toml` and its local
/// path dependencies. Kept beside the name-resolution kinds because a package
/// is a boundary in name lookup, and `explanations/` is at its file cap.
fn package_errors(name: &str) -> Option<ErrorExplanation> {
    let exp = match name {
        "ManifestInvalid" => ErrorExplanation {
            name: "ManifestInvalid",
            code: "E0120",
            category: "Packages",
            summary: "`albite.toml` outside the supported schema",
            detail: "\
A `albite.toml` holds exactly two tables. `[package]` requires `name` — a \
module identifier (`[a-z_][a-z0-9_]*`, not a keyword), because it becomes a \
path segment — and accepts an optional `version` string. `[dependencies]` \
accepts only `name = { path = \"…\" }`, relative to the manifest.\n\
\n\
Version, git and registry dependencies arrive in Phase 2; they are refused \
rather than silently ignored.",
            example: "\
[package]\n\
name = \"my-project\"    # error: not a module identifier — use `my_project`",
            see_also: &["DependencyNotFound", "PackageNameConflict"],
        },
        "DependencyNotFound" => ErrorExplanation {
            name: "DependencyNotFound",
            code: "E0121",
            category: "Packages",
            summary: "a path dependency cannot be found",
            detail: "\
A path dependency names a directory that must exist and hold both a \
`albite.toml` and a `lib.tg`, the library's root module. The path is \
resolved relative to the manifest that names it.",
            example: "\
[dependencies]\n\
geometry = { path = \"../geometry\" }   # error if ../geometry/lib.tg is absent",
            see_also: &["ManifestInvalid"],
        },
        "PackageNameConflict" => ErrorExplanation {
            name: "PackageNameConflict",
            code: "E0122",
            category: "Packages",
            summary: "a dependency key clashes with a name",
            detail: "\
Every package is mounted once at the module root under its own name, so a \
dependency key must equal the library's `[package] name` (renames are not \
supported yet), and no two directories in the graph may share a name. A \
package's name must not equal a top-level module of the application — the \
root is shared — and a library's dependency key must not equal one of that \
library's own top-level modules, or `use units::f` would mean either.",
            example: "\
[dependencies]\n\
shapes = { path = \"../geometry\" }   # error: that package is named `geometry`",
            see_also: &["ManifestInvalid", "ModuleNotFound"],
        },
        "CrossPackageNameCollision" => ErrorExplanation {
            name: "CrossPackageNameCollision",
            code: "E0123",
            category: "Packages",
            summary: "two packages declare one extern C symbol",
            detail: "\
An extern's C symbol is its identity: two packages declaring `tg_x` would \
bind one native function, perhaps at two different signatures. The \
collision is refused up front, naming both declarations; declare the extern \
once and import it. Functions, theorems, lemmas and axioms left this code \
when values were keyed by module: two packages may now each define \
`helper`, a qualified path names exactly one, and importing both under one \
name is DuplicateImport (E0007) or GlobConflict (E0018).",
            example: "\
// lib_a/lib.tg\n\
pub extern \"C\" fn tg_x() -> Nat\n\
// lib_b/lib.tg\n\
pub extern \"C\" fn tg_x(n: Nat) -> Nat   // error: two packages declare `tg_x`",
            see_also: &["DuplicateImport", "GlobConflict", "AmbiguousValue"],
        },
        "DependencyCycle" => ErrorExplanation {
            name: "DependencyCycle",
            code: "E0124",
            category: "Packages",
            summary: "the dependency graph loops back on itself",
            detail: "\
A library may declare dependencies of its own, and the compiler walks every \
manifest from the application down. A package that is reached again while it \
is still being walked closes a cycle — including a package naming itself \
(`path = \".\"`) and a library naming the application's directory. The error \
prints the chain by package name and points at the key that closes it. \
Break the cycle by moving what both packages need into a third one that \
depends on neither.",
            example: "\
# lib_a/albite.toml\n\
[dependencies]\n\
lib_b = { path = \"../lib_b\" }\n\
# lib_b/albite.toml\n\
[dependencies]\n\
lib_a = { path = \"../lib_a\" }   # error: dependency cycle: app → lib_a → lib_b → lib_a",
            see_also: &["DependencyNotFound", "PackageNameConflict"],
        },
        _ => return None,
    };
    Some(exp)
}

/// Binding-related name resolution errors.
fn binding_errors(name: &str) -> Option<ErrorExplanation> {
    let exp = match name {
        "UndefinedVariable" => ErrorExplanation {
            name: "UndefinedVariable",
            code: "E0001",
            category: "Name Resolution",
            summary: "cannot find value in scope",
            detail: "\
The compiler cannot find a variable, function, or value with this name \
in the current scope.\n\
\n\
Common causes:\n\
• Typo in the variable name\n\
• The variable is defined in a different scope or module\n\
• Missing `use` import for a module-level definition\n\
• The variable is defined after the point of use",
            example: "\
fn main() -> Nat {\n\
    x    // error: cannot find value `x` in this scope\n\
}",
            see_also: &[
                "UndefinedType",
                "UndefinedConstructor",
                "ItemNotFoundInModule",
            ],
        },

        "UndefinedType" => ErrorExplanation {
            name: "UndefinedType",
            code: "E0002",
            category: "Name Resolution",
            summary: "cannot find type in scope",
            detail: "\
The compiler cannot find a type with this name in the current scope.\n\
\n\
Common causes:\n\
• Typo in the type name\n\
• Missing `use` import for the type\n\
• The type is defined in a different module\n\
• Using a value name where a type name is expected",
            example: "\
fn main() -> Foo {   // error: cannot find type `Foo` in this scope\n\
    42\n\
}",
            see_also: &["UndefinedVariable", "UndefinedConstructor"],
        },

        "UndefinedConstructor" => ErrorExplanation {
            name: "UndefinedConstructor",
            code: "E0003",
            category: "Name Resolution",
            summary: "cannot find constructor in scope",
            detail: "\
The compiler cannot find a constructor with this name.\n\
\n\
Common causes:\n\
• Typo in the constructor name\n\
• Missing `use` import for the type that declares this constructor\n\
• Using a type name instead of a constructor name\n\
• The ADT definition has different constructor names than expected",
            example: "\
type Color = Red | Green | Blue\n\
\n\
fn main() -> Color {\n\
    Yellow    // error: cannot find constructor `Yellow`\n\
}",
            see_also: &["UndefinedVariable", "UndefinedType"],
        },

        "DuplicateDefinition" => ErrorExplanation {
            name: "DuplicateDefinition",
            code: "E0004",
            category: "Name Resolution",
            summary: "name defined multiple times",
            detail: "\
A name (function, type, or value) is defined more than once in the same scope.\n\
\n\
Common causes:\n\
• Copy-paste leaving a duplicate definition\n\
• Two modules defining the same name without namespacing\n\
• Accidentally redefining a standard library name",
            example: "\
fn greet() -> String { \"hello\" }\n\
fn greet() -> String { \"hi\" }    // error: `greet` is defined multiple times",
            see_also: &["DuplicateImport", "GlobConflict"],
        },

        _ => return None,
    };
    Some(exp)
}

/// Module-related name resolution errors.
fn module_errors(name: &str) -> Option<ErrorExplanation> {
    let exp = match name {
        "ModuleNotFound" => ErrorExplanation {
            name: "ModuleNotFound",
            code: "E0005",
            category: "Name Resolution",
            summary: "cannot find referenced module",
            detail: "\
The compiler cannot find the module referenced in a `mod` or `use` declaration.\n\
\n\
Common causes:\n\
• The module file does not exist (expected `name.tg` or `name/mod.tg`)\n\
• Typo in the module name\n\
• Wrong directory structure",
            example: "\
mod utils;    // error: cannot find module `utils`\n\
              // expected: utils.tg or utils/mod.tg",
            see_also: &["ItemNotFoundInModule", "UnresolvedImport"],
        },

        "ItemNotFoundInModule" => ErrorExplanation {
            name: "ItemNotFoundInModule",
            code: "E0006",
            category: "Name Resolution",
            summary: "item not found in module",
            detail: "\
The specified item does not exist in the referenced module.\n\
\n\
Common causes:\n\
• Typo in the item name\n\
• The item exists but is not public (not exported)\n\
• The item was renamed or removed",
            example: "\
use math::multiply;    // error: cannot find `multiply` in module `math`\n\
                       // perhaps you meant `mul`?",
            see_also: &["ModuleNotFound", "PrivateItem"],
        },

        "DuplicateImport" => ErrorExplanation {
            name: "DuplicateImport",
            code: "E0007",
            category: "Name Resolution",
            summary: "same name imported twice",
            detail: "\
The same name is imported more than once, either from the same module \
or from different modules.\n\
\n\
Common causes:\n\
• Importing the same name explicitly and via glob (`use mod::*`)\n\
• Two separate `use` statements importing the same name\n\
• Importing from two modules that export the same name",
            example: "\
use math::add;\n\
use utils::add;    // error: `add` is imported from both `math` and `utils`",
            see_also: &["GlobConflict", "DuplicateDefinition"],
        },

        "GlobConflict" => ErrorExplanation {
            name: "GlobConflict",
            code: "E0018",
            category: "Name Resolution",
            summary: "glob imports conflict on a name",
            detail: "\
Two glob imports (`use module::*`) bring the same name into scope.\n\
\n\
Fix: use explicit imports to disambiguate.\n\
\n\
Common causes:\n\
• Two modules export items with the same name\n\
• Overly broad glob imports",
            example: "\
use math::*;\n\
use utils::*;    // error: `add` is imported from both `math::*` and `utils::*`\n\
\n\
// Fix: use explicit imports\n\
use math::add;\n\
use utils::concat;",
            see_also: &["DuplicateImport"],
        },
        "AmbiguousValue" => ErrorExplanation {
            name: "AmbiguousValue",
            code: "E0019",
            category: "Name Resolution",
            summary: "a bare name matches two visible definitions",
            detail: "\
Two modules may define one value name; a qualified path names exactly one. \
A bare name the current module neither defines nor imports is looked up \
program-wide, and when two definitions there are reachable and visible, \
neither is chosen. Qualify the call, or import one with `use`.",
            example: "\
// x.tg: pub fn helper(n: Nat) -> Nat { n + 10 }\n\
// y.tg: pub fn helper(n: Nat) -> Nat { n + 100 }\n\
fn bare() -> Nat { helper(1) }    // error: `helper` is ambiguous\n\
fn fixed() -> Nat { x::helper(1) }",
            see_also: &["DuplicateImport", "PrivateItem"],
        },

        "UnresolvedImport" => ErrorExplanation {
            name: "UnresolvedImport",
            code: "E0008",
            category: "Name Resolution",
            summary: "cannot resolve import path",
            detail: "\
The import path cannot be resolved to a valid module or item.\n\
\n\
Common causes:\n\
• Module does not exist at the expected path\n\
• Typo in the import path\n\
• Circular import dependency",
            example: "\
use deeply::nested::missing;    // error: cannot resolve import",
            see_also: &["ModuleNotFound", "ItemNotFoundInModule"],
        },

        _ => return None,
    };
    Some(exp)
}

/// Visibility-related name resolution errors.
fn visibility_errors(name: &str) -> Option<ErrorExplanation> {
    let exp = match name {
        "PrivateModule" => ErrorExplanation {
            name: "PrivateModule",
            code: "E0009",
            category: "Name Resolution",
            summary: "module is private",
            detail: "\
The referenced module is private and cannot be accessed from the current module.\n\
\n\
Common causes:\n\
• Trying to access an internal module from outside its parent\n\
• Missing `pub` on the module declaration",
            example: "\
// In lib.tg:\n\
mod internal;           // private module\n\
\n\
// In main.tg:\n\
use lib::internal::helper;    // error: module `internal` is private",
            see_also: &["PrivateItem", "PublicItemLeak"],
        },

        "PrivateItem" => ErrorExplanation {
            name: "PrivateItem",
            code: "E0016",
            category: "Name Resolution",
            summary: "item is private",
            detail: "\
The referenced item (function, type, or constructor) is private and cannot \
be accessed from the current module.\n\
\n\
Common causes:\n\
• The item is not marked `pub`\n\
• Trying to access an internal helper from outside its module",
            example: "\
// In math.tg:\n\
fn internal_add(x: Nat, y: Nat) -> Nat { x + y }   // private\n\
\n\
// In main.tg:\n\
use math::internal_add;    // error: `internal_add` is private",
            see_also: &["PrivateModule", "PublicItemLeak"],
        },

        "PublicItemLeak" => ErrorExplanation {
            name: "PublicItemLeak",
            code: "E0017",
            category: "Name Resolution",
            summary: "public item exposes private type",
            detail: "\
A public item's signature references a private type. External code could \
call this item but wouldn't be able to name the type in its signature.\n\
\n\
Common causes:\n\
• Public function returning a private type\n\
• Public type alias referencing a private type\n\
• Public function taking a private type as parameter",
            example: "\
type Secret = { value: Nat }          // private type\n\
\n\
pub fn get_secret() -> Secret {       // error: public function exposes\n\
    { value: 42 }                     // private type `Secret`\n\
}",
            see_also: &["PrivateItem"],
        },

        _ => return None,
    };
    Some(exp)
}
