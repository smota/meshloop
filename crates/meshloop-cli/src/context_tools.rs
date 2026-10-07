//! Read-only context-engineering verbs: `ast-skeleton` and `symbol-lookup` (issue #7).
//! Both stay inside a workspace root, cap what they read, and report measured timing
//! rather than claimed timing. MCP reaches them through the same CLI subprocess as
//! every other tool (ADR 0027).

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use serde_json::{Value, json};

use meshloop_context::{BitWidth, Language, SignatureIndex, extract_skeleton};

/// Largest single file either verb will read.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Upper bound on files indexed by one `symbol-lookup`.
pub const MAX_INDEX_FILES: usize = 20_000;
pub const DEFAULT_LOOKUP_K: usize = 10;
pub const MAX_LOOKUP_K: usize = 200;

/// Directories that hold build output, dependencies, or Meshloop scratch state.
const SKIPPED_DIRS: &[&str] = &["target", "node_modules", "dist", "vendor", "__pycache__"];

/// Resolve `raw` against `root` and refuse anything that lands outside it.
pub fn resolve_within(root: &Path, raw: &str) -> Result<PathBuf, String> {
    if raw.trim().is_empty() {
        return Err("path is empty".into());
    }
    let root = root
        .canonicalize()
        .map_err(|e| format!("cannot resolve workspace root {}: {e}", root.display()))?;
    let candidate = Path::new(raw);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let resolved = joined
        .canonicalize()
        .map_err(|e| format!("cannot resolve {raw}: {e}"))?;
    if !resolved.starts_with(&root) {
        return Err(format!("{raw} is outside the workspace root"));
    }
    Ok(resolved)
}

fn relative_display(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    parts.join("/")
}

fn read_bounded(path: &Path) -> Result<String, String> {
    let meta = fs::metadata(path).map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!(
            "{} is {} bytes; limit is {MAX_FILE_BYTES}",
            path.display(),
            meta.len()
        ));
    }
    fs::read_to_string(path).map_err(|e| format!("cannot read {} as UTF-8: {e}", path.display()))
}

fn language_name(language: Language) -> String {
    format!("{language:?}").to_lowercase()
}

pub fn ast_skeleton(root: &Path, raw_path: &str) -> Result<Value, String> {
    let root = root
        .canonicalize()
        .map_err(|e| format!("cannot resolve workspace root {}: {e}", root.display()))?;
    let path = resolve_within(&root, raw_path)?;
    let language = Language::from_path(&path);
    if language == Language::Unknown {
        return Err(format!(
            "{raw_path}: unsupported language (rs, ts/js, py, go, cs, php, c/c++, md)"
        ));
    }
    let source = read_bounded(&path)?;
    let result = extract_skeleton(&source, language);
    Ok(json!({
        "path": relative_display(&root, &path),
        "language": language_name(language),
        "original_lines": result.original_lines,
        "pruned_lines": result.pruned_lines,
        "original_approx_tokens": result.original_approx_tokens,
        "pruned_approx_tokens": result.pruned_approx_tokens,
        "savings_percentage": (result.savings_percentage() * 10.0).round() / 10.0,
        "skeleton": result.skeleton,
    }))
}

struct Walk {
    files: Vec<PathBuf>,
    truncated: bool,
}

/// Deterministic, bounded walk: sorted entries, no hidden or build directories,
/// no symlinks, only languages the skeleton extractor understands.
fn walk_sources(dir: &Path) -> Walk {
    let mut walk = Walk {
        files: Vec::new(),
        truncated: false,
    };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else {
            continue;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        let mut subdirs = Vec::new();
        for entry in entries {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if kind.is_dir() {
                if !name.starts_with('.') && !SKIPPED_DIRS.contains(&name.as_ref()) {
                    subdirs.push(entry.path());
                }
            } else if kind.is_file() {
                let path = entry.path();
                if Language::from_path(&path) == Language::Unknown {
                    continue;
                }
                if walk.files.len() >= MAX_INDEX_FILES {
                    walk.truncated = true;
                    return walk;
                }
                walk.files.push(path);
            }
        }
        // Reverse so the stack pops subdirectories in sorted order.
        stack.extend(subdirs.into_iter().rev());
    }
    walk
}

pub fn symbol_lookup(
    root: &Path,
    query: &str,
    k: usize,
    scope: Option<&str>,
) -> Result<Value, String> {
    if query.trim().is_empty() {
        return Err("query is empty".into());
    }
    if k == 0 || k > MAX_LOOKUP_K {
        return Err(format!("k must be between 1 and {MAX_LOOKUP_K}"));
    }
    let root = root
        .canonicalize()
        .map_err(|e| format!("cannot resolve workspace root {}: {e}", root.display()))?;
    let base = match scope {
        Some(s) => resolve_within(&root, s)?,
        None => root.clone(),
    };
    if !base.is_dir() {
        return Err(format!("{} is not a directory", base.display()));
    }

    let index_started = Instant::now();
    let walk = walk_sources(&base);
    let mut index = SignatureIndex::new(BitWidth::Two);
    let mut skipped = 0usize;
    for file in &walk.files {
        let Ok(source) = read_bounded(file) else {
            skipped += 1;
            continue;
        };
        let rel = relative_display(&root, file);
        let skeleton = extract_skeleton(&source, Language::from_path(file));
        index.add_skeleton(&rel, &skeleton.skeleton);
    }
    let index_us = index_started.elapsed().as_micros();

    let search_started = Instant::now();
    let hits = index.search(query, k);
    let search_us = search_started.elapsed().as_micros();

    let hits: Vec<Value> = hits
        .into_iter()
        .map(|h| {
            json!({
                "path": h.path,
                "name": h.name,
                "kind": format!("{:?}", h.kind).to_lowercase(),
                "score": (f64::from(h.score) * 10_000.0).round() / 10_000.0,
            })
        })
        .collect();
    Ok(json!({
        "query": query,
        "k": k,
        "scope": relative_display(&root, &base),
        "files_indexed": walk.files.len() - skipped,
        "files_skipped": skipped,
        "truncated": walk.truncated,
        "entries_indexed": index.len(),
        "index_build_us": index_us,
        "search_us": search_us,
        "hits": hits,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "meshloop-context-tools-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        dir
    }

    const RUST_SAMPLE: &str = "pub struct Ledger { entries: Vec<u64> }\n\
        impl Ledger {\n\
        \x20   pub fn reconcile_balance(&mut self, delta: u64) -> u64 {\n\
        \x20       let mut total = 0;\n\
        \x20       for e in &self.entries { total += e; }\n\
        \x20       total += delta;\n\
        \x20       self.entries.push(delta);\n\
        \x20       total\n\
        \x20   }\n\
        }\n";

    #[test]
    fn skeleton_elides_bodies_and_reports_savings() {
        let dir = scratch("skel");
        fs::write(dir.join("src/ledger.rs"), RUST_SAMPLE).unwrap();
        let v = ast_skeleton(&dir, "src/ledger.rs").unwrap();
        assert_eq!(v["language"], "rust");
        assert_eq!(v["path"], "src/ledger.rs");
        let skel = v["skeleton"].as_str().unwrap();
        assert!(skel.contains("pub fn reconcile_balance"));
        assert!(!skel.contains("self.entries.push(delta)"));
        assert!(v["savings_percentage"].as_f64().unwrap() > 0.0);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn paths_outside_root_are_rejected() {
        let dir = scratch("escape");
        let outside = dir.parent().unwrap().join(format!(
            "meshloop-context-tools-outside-{}.rs",
            std::process::id()
        ));
        fs::write(&outside, RUST_SAMPLE).unwrap();
        let err = ast_skeleton(&dir, outside.to_str().unwrap()).unwrap_err();
        assert!(err.contains("outside the workspace root"), "{err}");
        let rel = format!("../{}", outside.file_name().unwrap().to_string_lossy());
        let err = ast_skeleton(&dir, &rel).unwrap_err();
        assert!(err.contains("outside the workspace root"), "{err}");
        fs::remove_file(&outside).ok();
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_language_is_an_error() {
        let dir = scratch("unknown");
        fs::write(dir.join("notes.txt"), "hello").unwrap();
        assert!(
            ast_skeleton(&dir, "notes.txt")
                .unwrap_err()
                .contains("unsupported")
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lookup_ranks_the_declaring_file_and_skips_build_dirs() {
        let dir = scratch("lookup");
        fs::write(dir.join("src/ledger.rs"), RUST_SAMPLE).unwrap();
        fs::write(
            dir.join("src/render.py"),
            "def draw_widget(canvas):\n    return canvas.paint()\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("target/debug")).unwrap();
        fs::write(dir.join("target/debug/ledger.rs"), RUST_SAMPLE).unwrap();
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join(".git/hook.py"), "def x(): pass\n").unwrap();

        let v = symbol_lookup(&dir, "reconcile_balance Ledger", 3, None).unwrap();
        assert_eq!(v["files_indexed"], 2);
        assert_eq!(v["truncated"], false);
        let hits = v["hits"].as_array().unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0]["path"], "src/ledger.rs");
        assert!(
            hits.iter()
                .all(|h| !h["path"].as_str().unwrap().starts_with("target"))
        );
        assert!(v["search_us"].is_u64());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lookup_validates_inputs() {
        let dir = scratch("inputs");
        assert!(symbol_lookup(&dir, "  ", 3, None).is_err());
        assert!(symbol_lookup(&dir, "x", 0, None).is_err());
        assert!(symbol_lookup(&dir, "x", MAX_LOOKUP_K + 1, None).is_err());
        assert!(symbol_lookup(&dir, "x", 3, Some("../")).is_err());
        fs::remove_dir_all(&dir).ok();
    }
}
