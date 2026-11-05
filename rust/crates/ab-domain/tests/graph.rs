use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    process::Command,
};

#[test]
#[allow(clippy::too_many_lines)]
fn enforce_workspace_dependency_boundaries() -> Result<(), Box<dyn std::error::Error>> {
    let result = Command::new(env!("CARGO"))
        .args(["metadata", "--format-version", "1", "--locked"])
        .output()?;
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let metadata: Value = serde_json::from_slice(&result.stdout)?;
    let packages = metadata["packages"].as_array().ok_or("packages")?;
    let names: BTreeMap<&str, &str> = packages
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p["name"].as_str()?)))
        .collect();
    let nodes = metadata["resolve"]["nodes"].as_array().ok_or("nodes")?;
    let edges: BTreeMap<&str, Vec<&str>> = nodes
        .iter()
        .map(|n| {
            let id = n["id"].as_str().ok_or("node id")?;
            let deps = n["dependencies"]
                .as_array()
                .ok_or("dependencies")?
                .iter()
                .map(|d| d.as_str().ok_or("dependency id"))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((id, deps))
        })
        .collect::<Result<_, &str>>()?;
    let rules: &[(&str, &[&str])] = &[
        ("ab-domain", &[]),
        ("ab-wire", &["ab-domain"]),
        ("ab-commands", &["ab-wire"]),
        ("ab-catalog", &["ab-domain"]),
        ("ab-commands", &["ab-wire"]),
        ("ab-state", &["ab-domain", "ab-wire"]),
        ("ab-trust", &["ab-domain", "ab-wire", "ab-sys"]),
        ("ab-blobs", &["ab-domain", "ab-trust"]),
        ("ab-commands", &["ab-wire"]),
        (
            "ab-operations",
            &[
                "ab-state",
                "ab-blobs",
                "ab-trust",
                "ab-catalog",
                "ab-commands",
            ],
        ),
        ("ab-applepeer", &["ab-wire", "ab-operations"]),
        ("ab-replication", &["ab-state", "ab-blobs", "ab-trust"]),
        (
            "ab-frontdoor",
            &[
                "ab-operations",
                "ab-replication",
                "ab-blobs",
                "ab-trust",
                "ab-catalog",
                "ab-pages",
            ],
        ),
        ("ab-remote", &["ab-trust", "ab-wire"]),
        ("ab-iphone", &["ab-trust", "ab-operations", "ab-wire"]),
        ("ab-native", &["ab-domain", "ab-wire"]),
        ("ab-session", &["ab-native", "ab-wire", "ab-sys"]),
        ("ab-sys", &[]),
    ];
    for (id, name) in &names {
        if !metadata["workspace_members"]
            .as_array()
            .ok_or("members")?
            .iter()
            .any(|v| v.as_str() == Some(id))
        {
            continue;
        }
        let deps = edges.get(id).ok_or("edges")?;
        if let Some((_, allowed)) = rules.iter().find(|(n, _)| n == name) {
            for dep in deps {
                let dependency = names.get(dep).ok_or("name")?;
                assert!(
                    !dependency.starts_with("ab-") || allowed.contains(dependency),
                    "{name} cannot depend on {dependency}"
                );
            }
        }
        if name.starts_with("lab-") {
            assert!(
                deps.iter()
                    .all(|d| names.get(d).is_none_or(|n| !n.starts_with("ab-"))),
                "lab must be independent"
            );
        }
        if ["ab-domain", "ab-native", "ab-session"].contains(name) {
            let mut stack = deps.clone();
            let mut visited = BTreeSet::new();
            while let Some(dep) = stack.pop() {
                if !visited.insert(dep) {
                    continue;
                }
                let dependency = names.get(dep).ok_or("name")?;
                assert!(
                    ![
                        "tokio", "hyper", "axum", "reqwest", "rustls", "socket2", "mio", "smux",
                        "quinn"
                    ]
                    .contains(dependency),
                    "{name} reaches network crate {dependency}"
                );
                if *name == "ab-domain" {
                    assert!(!["rusqlite", "libsqlite3-sys"].contains(dependency));
                }
                if let Some(next) = edges.get(dep) {
                    stack.extend(next);
                }
            }
        }
    }
    Ok(())
}
