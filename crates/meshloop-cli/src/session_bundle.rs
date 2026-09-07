//! Session pack embedded in the engine so `cargo install` can emit skills (ADR 0018).
//! Repo-root `skills/` is the git-clone source of truth; this copy is what crates.io builds.

use std::fs;
use std::path::{Path, PathBuf};

pub struct BundledFile {
    pub path: &'static str,
    pub contents: &'static str,
}

pub fn files() -> &'static [BundledFile] {
    &[
        BundledFile {
            path: "skills/README.md",
            contents: include_str!("../session-bundle/skills/README.md"),
        },
        BundledFile {
            path: "skills/meshloop-accept/SKILL.md",
            contents: include_str!("../session-bundle/skills/meshloop-accept/SKILL.md"),
        },
        BundledFile {
            path: "skills/meshloop-doctor/SKILL.md",
            contents: include_str!("../session-bundle/skills/meshloop-doctor/SKILL.md"),
        },
        BundledFile {
            path: "skills/meshloop-orchestrate/SKILL.md",
            contents: include_str!("../session-bundle/skills/meshloop-orchestrate/SKILL.md"),
        },
        BundledFile {
            path: "skills/meshloop-plan/SKILL.md",
            contents: include_str!("../session-bundle/skills/meshloop-plan/SKILL.md"),
        },
        BundledFile {
            path: "skills/meshloop-review-plan/SKILL.md",
            contents: include_str!("../session-bundle/skills/meshloop-review-plan/SKILL.md"),
        },
        BundledFile {
            path: "skills/meshloop-roles/SKILL.md",
            contents: include_str!("../session-bundle/skills/meshloop-roles/SKILL.md"),
        },
        BundledFile {
            path: "skills/meshloop-run/SKILL.md",
            contents: include_str!("../session-bundle/skills/meshloop-run/SKILL.md"),
        },
        BundledFile {
            path: "skills/meshloop-status/SKILL.md",
            contents: include_str!("../session-bundle/skills/meshloop-status/SKILL.md"),
        },
    ]
}

pub fn catalog_json(version: &str) -> String {
    format!(
        "{{\n  \"name\": \"meshloop-session-bundle\",\n  \"version\": \"{version}\",\n  \"namespace\": \"meshloop:\",\n  \"mcp\": \"meshloop mcp\",\n  \"slash_prefix\": \"/meshloop:\",\n  \"roles\": [\"meshloop:origin\",\"meshloop:planner\",\"meshloop:scout\",\"meshloop:worker\",\"meshloop:reviewer\"]\n}}\n"
    )
}

pub fn pack_readme() -> &'static str {
    "Meshloop session bundle\n\nCopy skills/meshloop-* into the origin harness skill folder.\nRun local MCP: meshloop mcp\nNever use unprefixed plan/reviewer/scout tools.\nFull setup: https://github.com/smota/meshloop/blob/main/docs/install.md\n"
}

pub fn write_to(dest: &Path) -> Result<Vec<PathBuf>, String> {
    fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let mut written = Vec::new();
    for file in files() {
        let path = dest.join(file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        fs::write(&path, file.contents)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        written.push(path);
    }
    let catalog = dest.join("meshloop-mcp-tools.json");
    fs::write(&catalog, catalog_json(env!("CARGO_PKG_VERSION")))
        .map_err(|e| format!("cannot write {}: {e}", catalog.display()))?;
    written.push(catalog);
    let readme = dest.join("README.md");
    fs::write(&readme, pack_readme())
        .map_err(|e| format!("cannot write {}: {e}", readme.display()))?;
    written.push(readme);
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_skills_match_workspace_pack_when_present() {
        let workspace_skills = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills");
        if !workspace_skills.join("README.md").is_file() {
            return;
        }
        for file in files() {
            let rel = file
                .path
                .strip_prefix("skills/")
                .expect("bundled path starts with skills/");
            let disk = fs::read_to_string(workspace_skills.join(rel))
                .unwrap_or_else(|e| panic!("read {}: {e}", rel));
            assert_eq!(
                normalize_newlines(&disk),
                normalize_newlines(file.contents),
                "embedded session pack drifted from repo-root skills/{rel}; run cargo run -p xtask -- bundle"
            );
        }
    }

    #[test]
    fn catalog_pins_meshloop_namespace() {
        let json = catalog_json("0.1.0");
        assert!(json.contains("\"namespace\": \"meshloop:\""));
        assert!(json.contains("meshloop mcp"));
        assert!(json.contains("0.1.0"));
    }

    fn normalize_newlines(s: &str) -> String {
        s.replace("\r\n", "\n")
    }
}
