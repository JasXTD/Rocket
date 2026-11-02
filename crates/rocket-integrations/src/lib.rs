#![forbid(unsafe_code)]
//! Versioned integration packages, bounded native UI descriptions and a separate
//! Lua worker. Package discovery never executes code. Credentials stay in the host.
mod catalog;
mod compare;
mod execution;
mod developer;
pub mod playground;
mod repository_context;
mod model;
#[cfg(feature = "runtime")]
pub mod runtime;
pub mod storage;
pub mod profiles;
pub use catalog::{
    ApprovedCopy, Discovery, Package, PackageIssue, bundled, discover, example_names, install_example,
    install_package, read_package_file, remove_package,
};
pub use compare::{CommandChange, CommandLine, Line, LineKind, SourceChange, compare_commands, compare_source};
pub use execution::{Outcome, execute, submit};
pub use developer::{Validation, validate_package};
pub use repository_context::repository_context;
pub use model::{Cell, Command, Node, Request, Setting, Target, array, id, text};
use rocket_git::{Error, Result};
pub(crate) fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.into())
}
pub(crate) fn object(v: &serde_json::Value) -> Result<&serde_json::Map<String, serde_json::Value>> {
    v.as_object()
        .ok_or_else(|| invalid("integration_schema: Expected an object."))
}

#[cfg(test)]
mod tests;
