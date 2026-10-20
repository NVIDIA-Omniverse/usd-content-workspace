// SPDX-FileCopyrightText: Copyright 2026 NVIDIA CORPORATION & AFFILIATES
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use cargo_metadata::MetadataCommand;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;
use std::{env, fs};

fn looks_like_license_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("license")
        || lower.starts_with("licence")
        || lower.starts_with("copying")
        || lower.starts_with("unlicense")
        || lower.starts_with("notice")
}

/// Scan a directory for files whose name looks like a license file (case-insensitive).
fn find_license_files_in_dir(dir: &Path) -> Vec<(String, String)> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut results = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().is_ok_and(|ft| ft.is_file()) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if looks_like_license_file(&name)
            && let Ok(content) = fs::read_to_string(entry.path())
        {
            results.push((name, content));
        }
    }
    results.sort_by(|(a, _), (b, _)| a.cmp(b));
    results
}

/// Scan the package directory for license texts, including a `LICENSES/` subdirectory.
///
/// All regular files inside `LICENSES/` are included regardless of name (REUSE convention).
fn find_license_texts(pkg_dir: &Path) -> Vec<(String, String)> {
    let mut results = find_license_files_in_dir(pkg_dir);

    let licenses_dir = pkg_dir.join("LICENSES");
    if let Ok(entries) = fs::read_dir(&licenses_dir) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|ft| ft.is_file()) {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    results.push((format!("LICENSES/{name}"), content));
                }
            }
        }
    }

    results
}

/// Parse a GitHub repository URL into `(owner, repo, Option<subpath>)`.
fn parse_github_url(url: &str) -> Option<(String, String, Option<String>)> {
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let mut segments: Vec<&str> = rest.split('/').collect();
    if segments.len() < 2 {
        return None;
    }

    let owner = segments[0].to_string();
    let repo = segments[1].trim_end_matches(".git").to_string();

    let subpath = if segments.len() > 3 && segments[2] == "tree" {
        segments.drain(..4);
        if segments.is_empty() {
            None
        } else {
            Some(segments.join("/"))
        }
    } else {
        None
    };

    Some((owner, repo, subpath))
}

/// Use the GitHub Contents API to list a directory and download license-like files.
///
/// Returns empty vec on any error (rate-limited, private repo, etc).
fn fetch_licenses_from_github_dir(
    client: &reqwest::blocking::Client,
    owner: &str,
    repo: &str,
    dir_path: &str,
) -> Vec<(String, String)> {
    let api_url = if dir_path.is_empty() {
        format!("https://api.github.com/repos/{owner}/{repo}/contents")
    } else {
        format!("https://api.github.com/repos/{owner}/{repo}/contents/{dir_path}")
    };

    let resp = match client.get(&api_url).send() {
        Ok(r) if r.status().is_success() => r,
        _ => return Vec::new(),
    };

    let items: Vec<serde_json::Value> = match resp.json() {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut results = Vec::new();
    let mut licenses_dir_present = false;

    for item in &items {
        let name = item["name"].as_str().unwrap_or_default();
        let item_type = item["type"].as_str().unwrap_or_default();

        if item_type == "dir" && name.eq_ignore_ascii_case("licenses") {
            licenses_dir_present = true;
            continue;
        }

        if item_type != "file" || !looks_like_license_file(name) {
            continue;
        }

        if let Some(download_url) = item["download_url"].as_str()
            && let Ok(resp) = client.get(download_url).send()
            && let Ok(text) = resp.text()
        {
            results.push((name.to_string(), text));
        }
    }

    if licenses_dir_present {
        let sub = if dir_path.is_empty() {
            "LICENSES".to_string()
        } else {
            format!("{dir_path}/LICENSES")
        };
        results.extend(fetch_all_files_from_github_dir(client, owner, repo, &sub));
    }

    results
}

/// Download all files from a GitHub directory (used for `LICENSES/` dirs).
fn fetch_all_files_from_github_dir(
    client: &reqwest::blocking::Client,
    owner: &str,
    repo: &str,
    dir_path: &str,
) -> Vec<(String, String)> {
    let api_url = format!("https://api.github.com/repos/{owner}/{repo}/contents/{dir_path}");

    let resp = match client.get(&api_url).send() {
        Ok(r) if r.status().is_success() => r,
        _ => return Vec::new(),
    };

    let items: Vec<serde_json::Value> = match resp.json() {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut results = Vec::new();
    for item in &items {
        let name = item["name"].as_str().unwrap_or_default();
        if item["type"].as_str() != Some("file") {
            continue;
        }
        if let Some(download_url) = item["download_url"].as_str()
            && let Ok(resp) = client.get(download_url).send()
            && let Ok(text) = resp.text()
        {
            let label = dir_path
                .rsplit_once('/')
                .map(|(_, dir)| format!("{dir}/{name}"))
                .unwrap_or_else(|| format!("{dir_path}/{name}"));
            results.push((label, text));
        }
    }
    results
}

/// Try to fetch license files from a GitHub repository via the Contents API.
///
/// For monorepo URLs (containing `/tree/<branch>/<path>`), tries the subcrate
/// directory first, then falls back to the repository root.
/// Results are cached by `(owner/repo, dir_path)` to avoid redundant API calls.
fn fetch_license_from_repo(
    client: &reqwest::blocking::Client,
    repo_url: &str,
    cache: &mut HashMap<String, Vec<(String, String)>>,
) -> Vec<(String, String)> {
    let (owner, repo, subpath) = match parse_github_url(repo_url) {
        Some(parsed) => parsed,
        None => return Vec::new(),
    };

    let dirs_to_try: Vec<String> = match &subpath {
        Some(sub) => vec![sub.clone(), String::new()],
        None => vec![String::new()],
    };

    for dir_path in &dirs_to_try {
        let cache_key = format!("{owner}/{repo}:{dir_path}");
        if let Some(cached) = cache.get(&cache_key) {
            if !cached.is_empty() {
                return cached.clone();
            }
            continue;
        }

        let results = fetch_licenses_from_github_dir(client, &owner, &repo, dir_path);
        cache.insert(cache_key, results.clone());
        if !results.is_empty() {
            return results;
        }
    }

    Vec::new()
}

const SPDX_TEXT_BASE_URL: &str =
    "https://raw.githubusercontent.com/spdx/license-list-data/main/text";

/// Extract individual SPDX license identifiers from an SPDX expression.
///
/// Splits on `OR`, `AND`, `WITH` and strips parentheses / exception suffixes.
fn extract_spdx_ids(expr: &str) -> Vec<String> {
    let cleaned = expr.replace(['(', ')'], " ");
    let mut ids = Vec::new();
    let mut skip_next = false;
    for token in cleaned.split_whitespace() {
        if skip_next {
            skip_next = false;
            continue;
        }
        match token {
            "OR" | "AND" => continue,
            "WITH" => {
                skip_next = true;
                continue;
            }
            id => ids.push(id.to_string()),
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

/// Fetch canonical license texts for each SPDX identifier in an expression.
///
/// Downloads from the spdx/license-list-data GitHub repository. Results are
/// cached across calls so each identifier is fetched at most once.
fn fetch_spdx_license_texts(
    client: &reqwest::blocking::Client,
    spdx_expr: &str,
    cache: &mut HashMap<String, Option<String>>,
) -> Vec<(String, String)> {
    let ids = extract_spdx_ids(spdx_expr);
    let mut results = Vec::new();

    for id in &ids {
        let text = cache
            .entry(id.clone())
            .or_insert_with(|| {
                let url = format!("{SPDX_TEXT_BASE_URL}/{id}.txt");
                eprint!("  Fetching SPDX text for {id}... ");
                match client.get(&url).send() {
                    Ok(resp) if resp.status().is_success() => match resp.text() {
                        Ok(t) => {
                            eprintln!("ok");
                            Some(t)
                        }
                        Err(_) => {
                            eprintln!("failed to read body");
                            None
                        }
                    },
                    _ => {
                        eprintln!("not found");
                        None
                    }
                }
            })
            .clone();

        if let Some(text) = text {
            results.push((id.clone(), text));
        }
    }

    results
}

struct Attribution {
    license: String,
    display_license: String,
    texts: Vec<(String, String)>,
    source: LicenseSource,
}

#[derive(Default)]
struct Stats {
    file: u32,
    repo: u32,
    spdx: u32,
    missing: u32,
}

impl Stats {
    fn total(&self) -> u32 {
        self.file + self.repo + self.spdx + self.missing
    }
}

enum LicenseSource {
    File,
    Repo,
    Spdx,
    Missing,
}

fn is_runtime_dep(dep: &cargo_metadata::NodeDep) -> bool {
    dep.dep_kinds
        .iter()
        .any(|dk| dk.kind != cargo_metadata::DependencyKind::Development)
}

/// Collect all transitive non-dev dependency package IDs for a given workspace
/// crate, excluding the crate itself and other workspace members.
fn resolve_transitive_deps(
    metadata: &cargo_metadata::Metadata,
    crate_name: &str,
) -> HashSet<cargo_metadata::PackageId> {
    let resolve = metadata.resolve.as_ref().expect("missing resolve graph");

    let root_id = metadata
        .workspace_packages()
        .iter()
        .find(|p| p.name.as_str() == crate_name)
        .unwrap_or_else(|| panic!("workspace crate `{crate_name}` not found"))
        .id
        .clone();

    let workspace_ids: HashSet<_> = metadata.workspace_members.iter().collect();

    let nodes: HashMap<_, _> = resolve.nodes.iter().map(|n| (&n.id, &n.deps)).collect();

    let mut visited = HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(&root_id);

    while let Some(id) = queue.pop_front() {
        if let Some(deps) = nodes.get(id) {
            for dep in deps.iter().filter(|d| is_runtime_dep(d)) {
                if visited.insert(dep.pkg.clone()) {
                    queue.push_back(&dep.pkg);
                }
            }
        }
    }

    visited
        .into_iter()
        .filter(|id| !workspace_ids.contains(id))
        .collect()
}

fn main() -> ExitCode {
    let workspace_root = env::args().nth(1).unwrap_or_else(|| ".".to_string());
    let manifest_path = Path::new(&workspace_root).join("Cargo.toml");

    let metadata = MetadataCommand::new()
        .manifest_path(&manifest_path)
        .exec()
        .expect("failed to execute `cargo metadata`");

    let target_deps = resolve_transitive_deps(&metadata, "microsoft-graph-userinfo-service");

    let client = reqwest::blocking::Client::builder()
        .user_agent("generate-attributions")
        .build()
        .expect("failed to build HTTP client");

    let mut entries: BTreeMap<String, Attribution> = BTreeMap::new();
    let mut repo_cache: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut spdx_cache: HashMap<String, Option<String>> = HashMap::new();
    let mut stats = Stats::default();

    for package in &metadata.packages {
        if !target_deps.contains(&package.id) {
            continue;
        }

        let license = package
            .license
            .clone()
            .unwrap_or_else(|| "Unknown".to_string());

        let pkg_dir = package.manifest_path.parent().expect("manifest has parent");

        let mut texts = find_license_texts(pkg_dir.as_std_path());
        let mut source = LicenseSource::File;

        if texts.is_empty()
            && let Some(license_file) = package.license_file()
            && let Ok(content) = fs::read_to_string(license_file.as_std_path())
        {
            texts.push(("LICENSE".to_string(), content));
        }

        if texts.is_empty()
            && let Some(repo_url) = &package.repository
        {
            eprintln!(
                "  Fetching license for {} {} from repo...",
                package.name, package.version,
            );
            texts = fetch_license_from_repo(&client, repo_url, &mut repo_cache);
            if !texts.is_empty() {
                source = LicenseSource::Repo;
            }
        }

        if texts.is_empty() && license != "Unknown" {
            texts = fetch_spdx_license_texts(&client, &license, &mut spdx_cache);
            if !texts.is_empty() {
                source = LicenseSource::Spdx;
            }
        }

        if texts.is_empty() {
            source = LicenseSource::Missing;
        }

        let texts = filter_copyleft_texts(texts);
        let display_license = permissive_only_expr(&license);

        let key = format!("{} {}", package.name, package.version);
        entries.insert(
            key,
            Attribution {
                license,
                display_license,
                texts,
                source,
            },
        );
    }

    let mut output = String::from("# Third-Party Licenses\n\n");
    let mut missing_names = Vec::new();

    for (name, attr) in &entries {
        writeln!(output, "## {name}\n").unwrap();
        writeln!(output, "{}\n", attr.display_license).unwrap();
        match attr.source {
            LicenseSource::Missing => {
                writeln!(output, "*No license file found.*\n").unwrap();
                missing_names.push(name.as_str());
                stats.missing += 1;
            }
            LicenseSource::Repo => {
                stats.repo += 1;
                write_license_texts(&mut output, &attr.texts);
            }
            LicenseSource::Spdx => {
                stats.spdx += 1;
                write_license_texts(&mut output, &attr.texts);
            }
            LicenseSource::File => {
                stats.file += 1;
                write_license_texts(&mut output, &attr.texts);
            }
        }
    }

    let out_path = Path::new(&workspace_root).join("ATTRIBUTIONS.md");
    fs::write(&out_path, &output).expect("failed to write ATTRIBUTIONS.md");

    println!(
        "Wrote {} entries to {} ({} from files, {} from repo, {} from SPDX defaults, {} missing)",
        stats.total(),
        out_path.display(),
        stats.file,
        stats.repo,
        stats.spdx,
        stats.missing,
    );

    if !missing_names.is_empty() {
        eprintln!(
            "\nWarning: {} package(s) had no license text resolved:",
            missing_names.len()
        );
        for name in &missing_names {
            eprintln!("  - {name}");
        }
    }

    let pkg_by_label: HashMap<&str, &cargo_metadata::PackageId> = metadata
        .packages
        .iter()
        .map(|p| {
            let label = entries
                .keys()
                .find(|k| k.as_str() == format!("{} {}", p.name, p.version))
                .map(|k| k.as_str())
                .unwrap_or_default();
            (label, &p.id)
        })
        .collect();

    let copyleft: Vec<_> = entries
        .iter()
        .filter(|(_, attr)| is_only_copyleft(&attr.license))
        .collect();

    if !copyleft.is_empty() {
        let target_ids: Vec<_> = copyleft
            .iter()
            .filter_map(|(name, _)| pkg_by_label.get(name.as_str()).copied())
            .collect();

        let chains = dependency_chains(&metadata, "microsoft-graph-userinfo-service", &target_ids);

        eprintln!(
            "\n\x1b[1;31mError: {} package(s) have non-permissive (copyleft) licenses:\x1b[0m",
            copyleft.len()
        );
        for (name, attr) in &copyleft {
            eprintln!("\n  \x1b[31m{name} ({})\x1b[0m\n", attr.license);
            if let Some(pkg_id) = pkg_by_label.get(name.as_str())
                && let Some(chain) = chains.get(*pkg_id)
            {
                print_dependency_chain(chain);
            }
        }
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

const COPYLEFT_PATTERNS: &[&str] = &[
    "AGPL",
    "GPL",
    "LGPL",
    "MPL",
    "EUPL",
    "CPAL",
    "OSL",
    "SSPL",
    "CC-BY-SA",
    "CC-BY-NC",
    "Sleepycat",
    "Watcom",
    "RPL",
    "CPL",
    "EPL",
    "CECILL",
];

fn is_copyleft_id(id: &str) -> bool {
    let upper = id.to_uppercase();
    COPYLEFT_PATTERNS
        .iter()
        .any(|pat| upper.contains(&pat.to_uppercase()))
}

/// Returns true if all `OR` alternatives in the expression are copyleft.
fn is_only_copyleft(spdx_expr: &str) -> bool {
    spdx_expr
        .split(" OR ")
        .all(|alt| extract_spdx_ids(alt).iter().any(|id| is_copyleft_id(id)))
}

/// Return the SPDX expression with copyleft alternatives removed.
fn permissive_only_expr(spdx_expr: &str) -> String {
    spdx_expr
        .split(" OR ")
        .filter(|alt| !extract_spdx_ids(alt).iter().any(|id| is_copyleft_id(id)))
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Check whether a license text label (filename or SPDX id) looks copyleft.
fn label_looks_copyleft(label: &str) -> bool {
    let upper = label.to_uppercase();
    COPYLEFT_PATTERNS
        .iter()
        .any(|pat| upper.contains(&pat.to_uppercase()))
}

/// Remove copyleft license texts from the collected entries.
fn filter_copyleft_texts(texts: Vec<(String, String)>) -> Vec<(String, String)> {
    texts
        .into_iter()
        .filter(|(label, _)| !label_looks_copyleft(label))
        .collect()
}

/// Build a human-readable dependency chain from the root crate to each target
/// package. Uses BFS on the forward dependency graph to find the shortest path.
fn dependency_chains(
    metadata: &cargo_metadata::Metadata,
    root_crate: &str,
    targets: &[&cargo_metadata::PackageId],
) -> HashMap<cargo_metadata::PackageId, Vec<String>> {
    let resolve = metadata.resolve.as_ref().expect("missing resolve graph");
    let pkg_name: HashMap<_, _> = metadata
        .packages
        .iter()
        .map(|p| (&p.id, format!("{} {}", p.name, p.version)))
        .collect();

    let root_id = metadata
        .workspace_packages()
        .iter()
        .find(|p| p.name.as_str() == root_crate)
        .expect("root crate not found")
        .id
        .clone();

    let fwd: HashMap<_, Vec<_>> = resolve
        .nodes
        .iter()
        .map(|n| {
            (
                &n.id,
                n.deps
                    .iter()
                    .filter(|d| is_runtime_dep(d))
                    .map(|d| &d.pkg)
                    .collect(),
            )
        })
        .collect();

    let mut result = HashMap::new();

    for &target in targets {
        let mut parents: HashMap<&cargo_metadata::PackageId, &cargo_metadata::PackageId> =
            HashMap::new();
        let mut queue = std::collections::VecDeque::new();
        let mut visited = HashSet::new();
        queue.push_back(&root_id);
        visited.insert(&root_id);
        let mut found = false;

        while let Some(id) = queue.pop_front() {
            if id == target {
                found = true;
                break;
            }
            if let Some(deps) = fwd.get(id) {
                for dep in deps {
                    if visited.insert(dep) {
                        parents.insert(dep, id);
                        queue.push_back(dep);
                    }
                }
            }
        }

        if found {
            let mut chain = Vec::new();
            let mut cur = target;
            while cur != &root_id {
                chain.push(
                    pkg_name
                        .get(cur)
                        .cloned()
                        .unwrap_or_else(|| cur.to_string()),
                );
                cur = parents[cur];
            }
            chain.push(
                pkg_name
                    .get(&root_id)
                    .cloned()
                    .unwrap_or_else(|| root_id.to_string()),
            );
            chain.reverse();
            result.insert(target.clone(), chain);
        }
    }

    result
}

/// Print a dependency chain as an indented tree.
fn print_dependency_chain(chain: &[String]) {
    for (i, name) in chain.iter().enumerate() {
        if i == 0 {
            eprintln!("  \x1b[31m{name}\x1b[0m");
        } else {
            let indent = "  ".repeat(i);
            let connector = if i == chain.len() - 1 {
                "└── "
            } else {
                "├── "
            };
            if i == chain.len() - 1 {
                eprintln!("  \x1b[31m{indent}{connector}{name}\x1b[0m");
            } else {
                eprintln!("  {indent}{connector}{name}");
            }
        }
    }
}

fn write_license_texts(output: &mut String, texts: &[(String, String)]) {
    for (label, content) in texts {
        if texts.len() > 1 {
            writeln!(output, "### {label}\n").unwrap();
        }
        writeln!(output, "```\n{}\n```\n", content.trim()).unwrap();
    }
}
