//! Inventory and original license texts of every crate the product is built from.
//! Development-only crates (including this tool's dependencies) are not shipped.
use crate::{Result, field, i18n, root};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::Path,
};

const PRODUCT: [&str; 2] = ["unbloat", "unbloat-shell"];
const ACCEPTED: [&str; 9] = [
    "MIT",
    "Apache-2.0",
    "Unlicense",
    "Unicode-3.0",
    "BSL-1.0",
    "Zlib",
    "ISC",
    "BSD-3-Clause",
    "0BSD",
];
const NOTICES: [&str; 6] = [
    "LICENSE",
    "LICENCE",
    "COPYING",
    "NOTICE",
    "COPYRIGHT",
    "UNLICENSE",
];

pub fn collect() -> Result {
    let metadata = crate::cargo_metadata(true)?;
    let packages: BTreeMap<&str, &Value> = metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|package| Ok((field(package, "id")?, package)))
        .collect::<Result<_>>()?;
    let mut inventory = vec![
        "# Locked dependency inventory".to_owned(),
        String::new(),
        "Generated from Cargo.lock with cargo metadata. Original license texts follow in source-notices/.".to_owned(),
        String::new(),
        "| Package | Version | License expression | Upstream |".to_owned(),
        "| --- | --- | --- | --- |".to_owned(),
    ];
    let destination = root().join("licenses/source-notices");
    // Regenerate from scratch so notices of removed or upgraded crates never ship.
    if destination.exists() {
        fs::remove_dir_all(&destination)?;
    }
    fs::create_dir_all(&destination)?;
    let mut shipped: Vec<&Value> = product_dependencies(&metadata)?
        .into_iter()
        .filter_map(|id| packages.get(id).copied())
        .filter(|package| !package["source"].is_null())
        .collect();
    shipped.sort_by_key(|package| (field(package, "name").ok(), field(package, "version").ok()));
    for package in shipped {
        let name = field(package, "name")?;
        let version = field(package, "version")?;
        let license = package["license"].as_str().unwrap_or_default();
        if license.is_empty() {
            return Err(i18n::message("licenses.missing", &[&name]).into());
        }
        if !accepted(license) {
            return Err(i18n::message("licenses.review", &[&name, &license]).into());
        }
        let upstream = package["repository"].as_str().map_or_else(
            || format!("https://crates.io/crates/{name}/{version}"),
            str::to_owned,
        );
        inventory.push(format!("| {name} | {version} | {license} | {upstream} |"));
        let source = Path::new(field(package, "manifest_path")?)
            .parent()
            .ok_or("cargo metadata: invalid manifest_path")?;
        let target = destination.join(format!("{name}-{version}"));
        let mut copied = 0;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            let file_name = entry.file_name().to_string_lossy().to_uppercase();
            if entry.file_type()?.is_file() && NOTICES.iter().any(|n| file_name.starts_with(n)) {
                fs::create_dir_all(&target)?;
                fs::copy(entry.path(), target.join(entry.file_name()))?;
                copied += 1;
            }
        }
        if copied == 0 {
            return Err(i18n::message("licenses.no_files", &[&name]).into());
        }
    }
    inventory.push(String::new());
    fs::write(
        root().join("licenses/DEPENDENCIES.md"),
        inventory.join("\n"),
    )?;
    println!("{}", i18n::text("licenses.complete"));
    Ok(())
}

/// Package ids reachable from the product crates through normal and build
/// dependencies, for every platform (the lock file is shared by all of them).
fn product_dependencies(metadata: &Value) -> Result<BTreeSet<&str>> {
    let members = metadata["workspace_members"].as_array();
    let nodes: BTreeMap<&str, &Value> = metadata["resolve"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|node| Ok((field(node, "id")?, node)))
        .collect::<Result<_>>()?;
    let mut pending: VecDeque<&str> = metadata["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|package| {
            PRODUCT.iter().any(|name| package["name"] == *name)
                && members.is_some_and(|m| m.contains(&package["id"]))
        })
        .map(|package| field(package, "id"))
        .collect::<Result<_>>()?;
    if pending.len() != PRODUCT.len() {
        return Err(i18n::text("package.missing_package").into());
    }
    let mut reachable = BTreeSet::new();
    while let Some(id) = pending.pop_front() {
        let node = nodes
            .get(id)
            .ok_or("cargo metadata: missing resolve node")?;
        for dependency in node["deps"].as_array().into_iter().flatten() {
            let shipped = dependency["dep_kinds"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|kind| kind["kind"] != "dev");
            let id = field(dependency, "pkg")?;
            if shipped && reachable.insert(id) {
                pending.push_back(id);
            }
        }
    }
    Ok(reachable)
}

/// Every identifier of an SPDX expression must be on the reviewed list.
fn accepted(expression: &str) -> bool {
    expression
        .replace(['(', ')', '/'], " ")
        .split_whitespace()
        .all(|token| token == "OR" || token == "AND" || ACCEPTED.contains(&token))
}

#[cfg(test)]
mod tests {
    use super::accepted;

    #[test]
    fn license_expressions_need_reviewed_identifiers() {
        for expression in [
            "MIT",
            "MIT OR Apache-2.0",
            "MIT/Apache-2.0",
            "(MIT OR Apache-2.0) AND Unicode-3.0",
        ] {
            assert!(accepted(expression), "{expression}");
        }
        for expression in [
            "GPL-3.0",
            "Apache-2.0 WITH LLVM-exception",
            "MIT OR GPL-2.0",
        ] {
            assert!(!accepted(expression), "{expression}");
        }
    }
}
