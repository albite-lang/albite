use super::*;

mod constructors;
mod fields;
mod min_visibility;
mod modules;
mod packages;
mod reexports;
mod scoped_values;

/// `env` with the module tree under `mount` declared a separate dependency
/// package, so any module outside it is cross-package (ADR 29.9.26g D6).
fn cross_package(env: &Env, mount: &ModulePath) -> Env {
    let mut env = env.clone();
    env.packages.add_dependency("dep", mount.clone());
    env
}
