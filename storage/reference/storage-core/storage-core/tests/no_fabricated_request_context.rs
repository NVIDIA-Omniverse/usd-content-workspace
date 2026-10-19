// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

//! Guards against non-test storage-core code fabricating empty per-request context.
//!
//! Caller metadata (`request.metadata()`) and per-request context (`request.extensions()`, e.g.
//! `AdditionalHeaders`) must flow from the ingress through every layer to the backend. A fresh
//! `MetadataMap` / `Extensions` built below the ingress silently drops the caller's credentials
//! and headers: forward the ones on the request you were handed instead. A site that genuinely
//! needs an empty one must say why on the same line: `// fabricated-context: allow(<reason>)`.
//!
//! Scope: all of `storage-core/src`, plus the gRPC ingress handlers in
//! `storage-core-builder/src/api_services/**/grpc.rs`. A gRPC handler already holds the caller's
//! `tonic::Request` and must hand it on (`.into()`, `from_parts`). The REST handlers (`rest.rs`)
//! are deliberately not scanned: building the first `MetadataMap` / `Extensions` from the HTTP
//! request is their job.
//!
//! It flags `MetadataMap::new` / `default` / `with_capacity`, `Extensions::new` / `default`,
//! `<MetadataMap | Extensions as Default>::default`, `Request::new` and `.into_request()`.
//!
//! This parses the sources with `syn`, so comments and string literals cannot produce false
//! positives. It skips items marked `#[cfg(test)]`, `#[test]`, `#[tokio::test]`,
//! `#[test_log::test]` or `#[rstest]`, and files reached only through `#[cfg(test)] mod x;`.
//! It matches names, not resolved types, so it misses a type-inferred `Default::default()` or
//! `unwrap_or_default()`, a `#[derive(Default)]` struct that contains one of these types,
//! `MetadataMap::from_headers(HeaderMap::new())`, and a constructor reached through a renamed
//! import (`use tonic::Request as R;`). In the other direction, it flags every `Request::new`,
//! including `http::Request::new`. It also scans code that is test-only through
//! `#[cfg(all(test, ..))]`, a file-level `#![cfg(test)]`, a `#[cfg(test)]` block statement or a
//! `#[path]` module. Mark such a site with the allow comment.
//!
//! Run it on its own with
//! `cargo nextest run -p storage-core --all-features --locked -E 'binary(no_fabricated_request_context)'`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use proc_macro2::{Span, TokenStream, TokenTree};
use syn::visit::{self, Visit};

/// `(type, constructor)` pairs that create empty per-request context. `Request::new` (ours or
/// tonic's) and tonic's `IntoRequest::into_request` build an empty `MetadataMap` and `Extensions`
/// internally; a sub-request should be built with
/// `tonic::Request::from_parts(metadata, extensions, message)` from the incoming one.
const FABRICATORS: &[(&str, &str)] = &[
    ("MetadataMap", "new"),
    ("MetadataMap", "default"),
    ("MetadataMap", "with_capacity"),
    ("Extensions", "new"),
    ("Extensions", "default"),
    ("Request", "new"),
    ("IntoRequest", "into_request"),
];

const ALLOW_MARKER: &str = "fabricated-context: allow(";

/// `#[cfg(test)]`, `#[test]`, `#[tokio::test]`, `#[test_log::test]`, `#[rstest]`.
fn is_test_only(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        if path.is_ident("cfg") {
            return attr.parse_args::<syn::Ident>().is_ok_and(|ident| ident == "test");
        }
        path.segments
            .last()
            .is_some_and(|segment| segment.ident == "test" || segment.ident == "rstest")
    })
}

fn is_fabricator(ty: &str, ctor: &str) -> bool {
    FABRICATORS.iter().any(|&(t, c)| t == ty && c == ctor)
}

/// Directory holding the files of `file`'s child modules (`mod x;` -> `<dir>/x.rs`).
fn child_module_dir(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or(Path::new(""));
    match file.file_stem().and_then(|stem| stem.to_str()) {
        Some("mod" | "lib" | "main") | None => parent.to_path_buf(),
        Some(stem) => parent.join(stem),
    }
}

fn child_module_files(file: &Path, name: &syn::Ident) -> [PathBuf; 2] {
    let dir = child_module_dir(file);
    [
        dir.join(format!("{name}.rs")),
        dir.join(name.to_string()).join("mod.rs"),
    ]
}

/// Collects the files of out-of-line child modules, split by whether they are test-only.
struct ModuleDecls<'a> {
    file: &'a Path,
    test_only: Vec<PathBuf>,
    all: Vec<PathBuf>,
}

impl<'ast> Visit<'ast> for ModuleDecls<'_> {
    fn visit_item_mod(&mut self, node: &'ast syn::ItemMod) {
        if node.content.is_none() {
            let files = child_module_files(self.file, &node.ident);
            if is_test_only(&node.attrs) {
                self.test_only.extend(files.iter().cloned());
            }
            self.all.extend(files);
        }
        visit::visit_item_mod(self, node);
    }
}

struct Finding {
    file: PathBuf,
    line: usize,
    what: String,
    source: String,
}

struct Scanner<'a> {
    file: &'a Path,
    lines: Vec<&'a str>,
    findings: Vec<Finding>,
}

impl Scanner<'_> {
    fn report(&mut self, span: Span, what: String) {
        let line = span.start().line;
        let source = self.lines.get(line.wrapping_sub(1)).copied().unwrap_or_default();
        if !source.contains(ALLOW_MARKER) {
            self.findings.push(Finding {
                file: self.file.to_path_buf(),
                line,
                what,
                source: source.trim().to_string(),
            });
        }
    }

    /// `syn` does not parse macro bodies (`vec![..]`, `tokio::select! {..}`), so match their tokens:
    /// `T :: ctor`, `< T as Default > :: default` and `. into_request`.
    fn scan_tokens(&mut self, tokens: TokenStream) {
        let mut flat = Vec::new();
        flatten(tokens, &mut flat);
        let ident = |i: usize| match flat.get(i) {
            Some(TokenTree::Ident(ident)) => Some(ident.to_string()),
            _ => None,
        };
        let punct = |i: usize, ch: char| matches!(flat.get(i), Some(TokenTree::Punct(p)) if p.as_char() == ch);
        for (i, token) in flat.iter().enumerate() {
            let TokenTree::Ident(ty) = token else { continue };
            let ty = ty.to_string();
            if ty == "into_request" && i.checked_sub(1).is_some_and(|prev| punct(prev, '.')) {
                self.report(token.span(), "into_request (in a macro)".to_string());
            } else if punct(i + 1, ':') && punct(i + 2, ':') {
                if let Some(ctor) = ident(i + 3).filter(|ctor| is_fabricator(&ty, ctor)) {
                    self.report(token.span(), format!("{ty}::{ctor} (in a macro)"));
                }
            } else if ident(i + 1).as_deref() == Some("as")
                && ident(i + 2).as_deref() == Some("Default")
                && punct(i + 3, '>')
                && punct(i + 4, ':')
                && punct(i + 5, ':')
                && ident(i + 6).as_deref() == Some("default")
                && is_fabricator(&ty, "default")
            {
                self.report(token.span(), format!("<{ty} as Default>::default (in a macro)"));
            }
        }
    }
}

fn flatten(tokens: TokenStream, out: &mut Vec<TokenTree>) {
    for token in tokens {
        match token {
            TokenTree::Group(group) => flatten(group.stream(), out),
            other => out.push(other),
        }
    }
}

fn type_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path.path.segments.last().map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

/// Skips the whole subtree of any node that carries a test-only attribute.
macro_rules! skip_test_only {
    ($($method:ident($ty:ty) => $walk:path;)*) => {
        $(
            fn $method(&mut self, node: &'ast $ty) {
                if !is_test_only(&node.attrs) {
                    $walk(self, node);
                }
            }
        )*
    };
}

impl<'ast> Visit<'ast> for Scanner<'_> {
    skip_test_only! {
        visit_item_const(syn::ItemConst) => visit::visit_item_const;
        visit_item_fn(syn::ItemFn) => visit::visit_item_fn;
        visit_item_impl(syn::ItemImpl) => visit::visit_item_impl;
        visit_item_macro(syn::ItemMacro) => visit::visit_item_macro;
        visit_item_mod(syn::ItemMod) => visit::visit_item_mod;
        visit_item_static(syn::ItemStatic) => visit::visit_item_static;
        visit_item_trait(syn::ItemTrait) => visit::visit_item_trait;
        visit_impl_item_const(syn::ImplItemConst) => visit::visit_impl_item_const;
        visit_impl_item_fn(syn::ImplItemFn) => visit::visit_impl_item_fn;
        visit_trait_item_fn(syn::TraitItemFn) => visit::visit_trait_item_fn;
        visit_local(syn::Local) => visit::visit_local;
        visit_stmt_macro(syn::StmtMacro) => visit::visit_stmt_macro;
    }

    /// `MetadataMap::new`, `tonic::metadata::MetadataMap::default`, `http::Extensions::new`,
    /// `<MetadataMap as Default>::default`, called or passed as a function (`unwrap_or_else(MetadataMap::new)`).
    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        let segments: Vec<&syn::PathSegment> = node.path.segments.iter().collect();
        if let Some(ctor) = segments.last() {
            let ty = match &node.qself {
                Some(qself) => type_name(&qself.ty),
                None => segments
                    .len()
                    .checked_sub(2)
                    .and_then(|i| segments.get(i))
                    .map(|segment| segment.ident.to_string()),
            };
            if let Some(ty) = ty.filter(|ty| is_fabricator(ty, &ctor.ident.to_string())) {
                let what = match node.qself {
                    Some(_) => format!("<{ty} as Default>::{}", ctor.ident),
                    None => format!("{ty}::{}", ctor.ident),
                };
                self.report(ctor.ident.span(), what);
            }
        }
        visit::visit_expr_path(self, node);
    }

    /// `message.into_request()` (tonic's `IntoRequest`) wraps a bare message exactly like `Request::new`.
    /// On a value that is already a `tonic::Request` it is a no-op; such a site takes the allow marker.
    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if node.method == "into_request" && node.args.is_empty() {
            self.report(node.method.span(), "into_request".to_string());
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_macro(&mut self, node: &'ast syn::Macro) {
        self.scan_tokens(node.tokens.clone());
        visit::visit_macro(self, node);
    }
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
            .path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Parses every `.rs` file under `src` and scans the non-test ones that `check` selects.
///
/// The whole tree is parsed even when only part of it is checked, so that a file reached only
/// through `#[cfg(test)] mod x;` (directly or transitively) is recognised as test code. Returns the
/// number of files checked alongside the findings, so a caller can tell an empty scope from a clean one.
fn scan_tree(src: &Path, check: impl Fn(&Path) -> bool) -> (usize, Vec<Finding>) {
    let mut paths = Vec::new();
    rust_files(src, &mut paths);
    paths.sort();
    assert!(!paths.is_empty(), "no sources found under {}", src.display());

    let files: Vec<(PathBuf, String, syn::File)> = paths
        .into_iter()
        .map(|path| {
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let ast = syn::parse_file(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
            (path, text, ast)
        })
        .collect();

    // Files reachable only through `#[cfg(test)] mod x;`, and everything they declare in turn.
    let mut test_only = BTreeSet::new();
    let mut children = Vec::new();
    for (path, _, ast) in &files {
        let mut decls = ModuleDecls {
            file: path,
            test_only: Vec::new(),
            all: Vec::new(),
        };
        decls.visit_file(ast);
        test_only.extend(decls.test_only);
        children.push((path.clone(), decls.all));
    }
    loop {
        let before = test_only.len();
        for (path, declared) in &children {
            if test_only.contains(path) {
                test_only.extend(declared.iter().cloned());
            }
        }
        if test_only.len() == before {
            break;
        }
    }

    let mut checked = 0;
    let mut findings = Vec::new();
    for (path, text, ast) in &files {
        if test_only.contains(path) || !check(path) {
            continue;
        }
        checked += 1;
        let mut scanner = Scanner {
            file: path,
            lines: text.lines().collect(),
            findings: Vec::new(),
        };
        scanner.visit_file(ast);
        findings.extend(scanner.findings);
    }
    (checked, findings)
}

#[test]
fn non_test_code_does_not_fabricate_request_context() {
    // Read at runtime: CI runs this from a nextest archive, which remaps the package directory.
    let manifest_dir =
        std::env::var_os("CARGO_MANIFEST_DIR").map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from);
    let workspace = manifest_dir
        .parent()
        .unwrap_or_else(|| panic!("{} has no parent workspace directory", manifest_dir.display()));

    let src = manifest_dir.join("src");
    let (checked, mut findings) = scan_tree(&src, |_| true);
    assert!(checked > 0, "no non-test sources checked under {}", src.display());

    // gRPC handlers only: a `grpc.rs` module, or anything under a `grpc/` module directory.
    let builder_src = workspace.join("storage-core-builder").join("src");
    let api_services = builder_src.join("api_services");
    let (checked, grpc_findings) = scan_tree(&builder_src, |path| {
        path.strip_prefix(&api_services).is_ok_and(|relative| {
            relative
                .components()
                .any(|component| component.as_os_str() == "grpc" || component.as_os_str() == "grpc.rs")
        })
    });
    assert!(checked > 0, "no gRPC handlers checked under {}", api_services.display());
    findings.extend(grpc_findings);

    let report: Vec<String> = findings
        .iter()
        .map(|f| {
            let file = f.file.strip_prefix(workspace).unwrap_or(&f.file);
            format!("  {}:{}: {}\n      {}", file.display(), f.line, f.what, f.source)
        })
        .collect();
    assert!(
        findings.is_empty(),
        "non-test code fabricates empty per-request context; forward `request.metadata()` / \
         `request.extensions()` from the incoming request instead, or justify the site with \
         `// {ALLOW_MARKER}<reason>)`:\n{}",
        report.join("\n")
    );
}
