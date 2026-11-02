//! Guard the pinned OSV scanner's npm source-resolution gap before using a cache
//! or accepting its otherwise successful output. Lockfiles remain data only.
use super::Inventory;
use rocket_git::{CancellationToken, Error, Result};
use serde_json::Value;

fn incomplete() -> Error {
    Error::InvalidInput("Dependency coverage is incomplete: an npm lockfile has missing or unsupported package source metadata. OSV 2.7 can omit their advisories. Regenerate the lockfile with complete registry resolution metadata, review it, then scan again.".into())
}

fn evaluated_source(source: &str) -> bool {
    // OSV 2.7 pins SCALIBR 5be78a00e3c2: DeterminePackageSource treats
    // unrecognized sources as Local and the scanner omits their advisories.
    // Recognized Git source categories remain supported. This is a coverage
    // check only: lockfile URLs are never fetched or used to execute programs.
    let lower = source.to_ascii_lowercase();
    if [
        "git+",
        "git://",
        "git@",
        "ssh://",
        "github:",
        "gitlab:",
        "bitbucket:",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || lower.ends_with(".git")
        || lower.contains(".git#")
    {
        return true;
    }
    let Some((scheme, rest)) = source.split_once("://") else {
        return false;
    };
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return false;
    }
    // Reuse the strict, bounded URL checker rather than WHATWG-style URL repair
    // (which could turn https:///host or bad escapes into a different URL).
    // Normalization here is validation only; OSV gets the unchanged lockfile.
    let rest = if let Some(ipv6) = rest.strip_prefix('[') {
        let Some((host, tail)) = ipv6.split_once(']') else {
            return false;
        };
        if host.parse::<std::net::Ipv6Addr>().is_err()
            || !tail.is_empty() && !tail.starts_with([':', '/', '?', '#'])
        {
            return false;
        }
        format!("registry.invalid{tail}")
    } else {
        rest.to_owned()
    };
    rocket_git::https_url(&format!("https://{rest}"))
}

fn entry(value: &Value) -> Result<()> {
    let package = value.as_object().ok_or_else(incomplete)?;
    if package.get("link").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let version = package
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(incomplete)?;
    let local = ["file:", "link:", "workspace:", "./", "../"]
        .iter()
        .any(|prefix| version.starts_with(prefix));
    if !local
        && package
            .get("resolved")
            .and_then(Value::as_str)
            .is_none_or(|source| !evaluated_source(source))
    {
        return Err(incomplete());
    }
    Ok(())
}

pub(super) fn check(inventory: &Inventory, cancel: &CancellationToken) -> Result<()> {
    for file in &inventory.files {
        cancel.check()?;
        if !matches!(
            file.path.file_name().and_then(|s| s.to_str()),
            Some("package-lock.json" | "npm-shrinkwrap.json")
        ) {
            continue;
        }
        // Discovery already bounds each file to 8 MiB. serde_json also limits
        // nesting; check cancellation again after parsing and between entries.
        let value: Value = serde_json::from_slice(&file.bytes).map_err(|_| incomplete())?;
        cancel.check()?;
        let lock = value.as_object().ok_or_else(incomplete)?;
        let version = match lock.get("lockfileVersion") {
            Some(value) => value.as_u64().ok_or_else(incomplete)?,
            None if !lock.contains_key("packages") => 1,
            None => return Err(incomplete()),
        };
        match version {
            2 | 3 => {
                let packages = lock
                    .get("packages")
                    .and_then(Value::as_object)
                    .ok_or_else(incomplete)?;
                for (path, package) in packages {
                    cancel.check()?;
                    // The root and workspace source folders are local. Installed
                    // dependency entries, including nested/scoped ones, are not.
                    if path.split('/').any(|part| part == "node_modules") {
                        entry(package)?;
                    }
                }
            }
            1 => {
                if lock.contains_key("packages") {
                    return Err(incomplete());
                }
                let Some(dependencies) = lock.get("dependencies") else {
                    continue;
                };
                let mut pending = vec![dependencies];
                while let Some(dependencies) = pending.pop() {
                    for package in dependencies.as_object().ok_or_else(incomplete)?.values() {
                        cancel.check()?;
                        entry(package)?;
                        if let Some(nested) = package.get("dependencies") {
                            pending.push(nested);
                        }
                    }
                }
            }
            _ => return Err(incomplete()),
        }
    }
    cancel.check()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dependency_security::scan::Input;
    use serde_json::json;
    fn inventory(value: Value) -> Inventory {
        Inventory {
            files: vec![Input {
                path: "nested/package-lock.json".into(),
                ecosystem: "npm".into(),
                bytes: serde_json::to_vec(&value).unwrap(),
            }],
            notes: vec![],
        }
    }
    #[test]
    fn resolved_registry_and_explicit_workspace_entries_remain_supported() {
        for version in [2, 3] {
            let inv = inventory(json!({"lockfileVersion":version,"packages":{
                "":{"name":"fixture"},
                "packages/local":{"version":"1.0.0"},
                "node_modules/local":{"link":true,"resolved":"packages/local"},
                "node_modules/file-local":{"version":"file:../local"},
                "node_modules/@scope/name":{"version":"1.2.3","resolved":"https://registry.example.invalid/package.tgz"},
                "node_modules/a/node_modules/b":{"version":"2.3.4","resolved":"https://registry.example.invalid/nested.tgz"}
            }}));
            check(&inv, &CancellationToken::new()).unwrap();
        }
        check(&inventory(json!({"lockfileVersion":1,"dependencies":{
            "example":{"version":"1.2.3","resolved":"https://registry.example.invalid/package.tgz","dependencies":{
                "local":{"version":"file:../local"}
            }}
        }})), &CancellationToken::new()).unwrap();
    }
    #[test]
    fn unrecognized_or_malformed_resolved_sources_fail_closed() {
        for source in [
            "garbage",
            "../package.tgz",
            "file:../package.tgz",
            "//registry.npmjs.org/package.tgz",
            "https:registry.npmjs.org/package.tgz",
            "https:///package.tgz",
            "https://",
            "https://bad host/p.tgz",
            "https://registry.npmjs.org/%zz",
            " https://registry.npmjs.org/p.tgz",
            "https://registry.npmjs.org/p.tgz\n",
        ] {
            let inv = inventory(
                json!({"lockfileVersion":3,"packages":{"node_modules/p":{"version":"1.2.3","resolved":source}}}),
            );
            assert!(
                check(&inv, &CancellationToken::new()).is_err(),
                "{source:?}"
            );
        }
        for source in [
            "https://registry.npmjs.org/@scope/p/-/p-1.2.3.tgz",
            "HTTP://localhost:4873/p.tgz",
            "https://[::1]:4873/p.tgz",
            "https://registry.example.invalid/p%20x.tgz?q=1&x=2",
            "git+https://example.invalid/p.git#1234567",
            "git@example.invalid:p.git",
            "github:owner/repo#v1",
            "gitlab:owner/repo",
            "bitbucket:owner/repo",
            "ssh://example.invalid/p.git",
            "git://example.invalid/p.git",
        ] {
            assert!(evaluated_source(source), "{source}");
        }
    }

    #[test]
    fn nested_and_scoped_packages_cannot_hide_missing_resolution() {
        for path in ["node_modules/@scope/name", "node_modules/a/node_modules/b"] {
            let inv = inventory(json!({"lockfileVersion":3,"packages":{path:{"version":"1.2.3"}}}));
            assert!(check(&inv, &CancellationToken::new()).is_err());
        }
        let inv = inventory(json!({"lockfileVersion":1,"dependencies":{"parent":{
            "version":"1.2.3","resolved":"https://registry.example.invalid/package.tgz",
            "dependencies":{"child":{"version":"2.3.4"}}
        }}}));
        assert!(check(&inv, &CancellationToken::new()).is_err());
    }
    #[test]
    fn invalid_and_unknown_npm_formats_fail_closed_and_cancel() {
        for value in [
            json!(null),
            json!({"lockfileVersion":99}),
            json!({"lockfileVersion":"3","packages":{}}),
            json!({"packages":{"node_modules/example":{"version":"1.2.3"}}}),
            json!({"lockfileVersion":3,"packages":[]}),
            json!({"lockfileVersion":1,"dependencies":{"bad":null}}),
        ] {
            assert!(check(&inventory(value), &CancellationToken::new()).is_err());
        }
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            check(&inventory(json!({})), &cancel),
            Err(Error::Cancelled)
        ));
    }
}
