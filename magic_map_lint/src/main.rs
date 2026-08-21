//! Finds hand-written std conversion impls in a codebase that maps with
//! `magic_map`.
//!
//! One mapping mechanism means `From` / `Into` / `TryFrom` / `TryInto` impls
//! are the escape hatch that grows back — a conversion written there skips
//! the leaf funnel, the infallibility check, and sealing. This walks the
//! given paths and flags every such impl, with two deliberate exemptions:
//!
//! * **Error conversions.** `impl From<MappingError> for ApiError` is how `?`
//!   bubbles between layers; an impl where either side's type name ends in
//!   `Error` is not a data mapping.
//! * **An allowlist**, one rendered signature per line (`# ` starts a
//!   comment), for the cases that are genuinely not mappings. The file only
//!   ever shrinks: an allowlisted signature that no longer exists fails the
//!   run, so stale entries cannot hide new violations.
//!
//! Usage: `magic-map-lint [--allow <file>] <path>...`
//! Exit code 1 when violations (or stale allowlist entries) are found.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use syn::visit::Visit;

const CONVERSION_TRAITS: [&str; 4] = ["From", "TryFrom", "Into", "TryInto"];

struct Finding {
    file: PathBuf,
    line: usize,
    signature: String,
}

struct Finder<'a> {
    file: &'a Path,
    findings: Vec<Finding>,
}

/// The last path segment's identifier, which is how humans read the type.
fn tail(path: &syn::Path) -> Option<String> {
    path.segments.last().map(|s| s.ident.to_string())
}

fn type_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(p) => tail(&p.path),
        syn::Type::Reference(r) => type_name(&r.elem),
        _ => None,
    }
}

/// The trait's first generic argument (`From<THIS> for ...`).
fn trait_arg_name(path: &syn::Path) -> Option<String> {
    let seg = path.segments.last()?;
    if let syn::PathArguments::AngleBracketed(args) = &seg.arguments {
        args.args.iter().find_map(|a| match a {
            syn::GenericArgument::Type(t) => type_name(t),
            _ => None,
        })
    } else {
        None
    }
}

fn is_error_name(name: &Option<String>) -> bool {
    name.as_deref().is_some_and(|n| n.ends_with("Error"))
}

impl Visit<'_> for Finder<'_> {
    fn visit_item_impl(&mut self, item: &syn::ItemImpl) {
        if let Some((_, trait_path, _)) = &item.trait_ {
            if let Some(trait_name) = tail(trait_path) {
                if CONVERSION_TRAITS.contains(&trait_name.as_str()) {
                    let source = trait_arg_name(trait_path);
                    let dest = type_name(&item.self_ty);
                    if !is_error_name(&source) && !is_error_name(&dest) {
                        let sig = format!(
                            "impl {}<{}> for {}",
                            trait_name,
                            source.as_deref().unwrap_or("_"),
                            dest.as_deref().unwrap_or("_"),
                        );
                        self.findings.push(Finding {
                            file: self.file.to_path_buf(),
                            line: item.impl_token.span.start().line,
                            signature: sig,
                        });
                    }
                }
            }
        }
        syn::visit::visit_item_impl(self, item);
    }
}

fn walk(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        // Skip build output; everything else is fair game.
        if path.file_name().is_some_and(|n| n == "target") {
            return;
        }
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            walk(&entry.path(), out);
        }
    } else if path.extension().is_some_and(|e| e == "rs") {
        out.push(path.to_path_buf());
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1).peekable();
    let mut allow_file = None;
    let mut roots = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--allow" => allow_file = args.next().map(PathBuf::from),
            _ => roots.push(PathBuf::from(arg)),
        }
    }
    if roots.is_empty() {
        eprintln!("usage: magic-map-lint [--allow <file>] <path>...");
        return ExitCode::from(2);
    }

    let allowed: BTreeSet<String> = allow_file
        .as_deref()
        .map(|f| {
            std::fs::read_to_string(f)
                .unwrap_or_else(|e| panic!("cannot read allowlist {}: {e}", f.display()))
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    let mut files = Vec::new();
    for root in &roots {
        walk(root, &mut files);
    }
    files.sort();

    let mut findings = Vec::new();
    for file in &files {
        let Ok(source) = std::fs::read_to_string(file) else {
            continue;
        };
        let Ok(ast) = syn::parse_file(&source) else {
            // Unparseable files are the compiler's problem, not ours.
            continue;
        };
        let mut finder = Finder {
            file,
            findings: Vec::new(),
        };
        finder.visit_file(&ast);
        findings.extend(finder.findings);
    }

    let mut used: BTreeSet<&str> = BTreeSet::new();
    let mut violations = 0;
    for f in &findings {
        if allowed.contains(&f.signature) {
            used.insert(&f.signature);
        } else {
            println!(
                "{}:{}: {} — declare it with magic_map! instead",
                f.file.display(),
                f.line,
                f.signature
            );
            violations += 1;
        }
    }

    // Ratchet: an allowlist entry nothing matched is debt already paid off.
    let mut stale = 0;
    for a in &allowed {
        if !used.contains(a.as_str()) {
            println!("stale allowlist entry (remove it): {a}");
            stale += 1;
        }
    }

    if violations + stale > 0 {
        println!("{violations} conversion impl(s) outside magic_map, {stale} stale allowlist entr(ies)");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
