//! Public type and function signatures extracted from AST skeletons.
//! Line-oriented on purpose: the skeleton already dropped bodies.

use crate::skeleton::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignatureKind {
    Function,
    Type,
    Alias,
    Constant,
    Impl,
    Heading,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signature {
    pub path: String,
    pub kind: SignatureKind,
    pub name: String,
    pub text: String,
}

/// Concatenate signatures of one file into a stable blob used by impact slicing.
pub fn file_signature_blob(signatures: &[Signature]) -> String {
    let mut lines: Vec<&str> = signatures.iter().map(|s| s.text.as_str()).collect();
    lines.sort_unstable();
    lines.join("\n")
}

pub fn extract_signatures(path: &str, skeleton: &str, language: Language) -> Vec<Signature> {
    let mut in_fence = false;
    let mut sigs = Vec::new();
    for line in skeleton.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(sig) = classify(path, line, language) {
            sigs.push(sig);
        }
    }
    sigs
}

fn classify(path: &str, line: &str, language: Language) -> Option<Signature> {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("//")
        || (trimmed.starts_with('#') && language == Language::Python)
    {
        return None;
    }
    let (kind, name) = match language {
        Language::Rust => rust_sig(trimmed)?,
        Language::TypeScript => ts_sig(trimmed)?,
        Language::Python => py_sig(trimmed)?,
        Language::Go => go_sig(trimmed)?,
        Language::CSharp => cs_sig(trimmed)?,
        Language::Php => php_sig(trimmed)?,
        Language::Cpp => cpp_sig(trimmed)?,
        Language::Markdown => md_sig(trimmed)?,
        Language::Unknown => {
            let name = first_ident(trimmed)?;
            (SignatureKind::Other, name)
        }
    };
    Some(Signature {
        path: path.to_string(),
        kind,
        name,
        text: trimmed.to_string(),
    })
}

fn rust_sig(t: &str) -> Option<(SignatureKind, String)> {
    if looks_like_fn(
        t,
        &[
            "fn ",
            "pub fn ",
            "pub async fn ",
            "async fn ",
            "pub(crate) fn ",
            "pub const fn ",
        ],
    ) {
        return Some((SignatureKind::Function, ident_after(t, "fn ")?));
    }
    if let Some(name) = type_after(
        t,
        &[
            "pub struct ",
            "struct ",
            "pub enum ",
            "enum ",
            "pub trait ",
            "trait ",
            "pub union ",
            "union ",
        ],
    ) {
        return Some((SignatureKind::Type, name));
    }
    if let Some(name) = type_after(t, &["pub type ", "type "]) {
        return Some((SignatureKind::Alias, name));
    }
    if let Some(name) = type_after(t, &["pub const ", "const ", "pub static ", "static "]) {
        return Some((SignatureKind::Constant, name));
    }
    if t.starts_with("impl ") || t.starts_with("pub impl ") {
        return Some((
            SignatureKind::Impl,
            ident_after(t, "impl ").unwrap_or_else(|| t.to_string()),
        ));
    }
    None
}

fn ts_sig(t: &str) -> Option<(SignatureKind, String)> {
    if looks_like_fn(
        t,
        &[
            "function ",
            "export function ",
            "export async function ",
            "async function ",
        ],
    ) {
        return Some((SignatureKind::Function, ident_after(t, "function ")?));
    }
    if let Some(name) = type_after(
        t,
        &[
            "export interface ",
            "interface ",
            "export class ",
            "class ",
            "export enum ",
            "enum ",
        ],
    ) {
        return Some((SignatureKind::Type, name));
    }
    if let Some(name) = type_after(t, &["export type ", "type "]) {
        return Some((SignatureKind::Alias, name));
    }
    None
}

fn py_sig(t: &str) -> Option<(SignatureKind, String)> {
    if t.starts_with("def ") || t.starts_with("async def ") {
        return Some((SignatureKind::Function, ident_after(t, "def ")?));
    }
    if t.starts_with("class ") {
        return Some((SignatureKind::Type, ident_after(t, "class ")?));
    }
    None
}

fn go_sig(t: &str) -> Option<(SignatureKind, String)> {
    if t.starts_with("func ") {
        let rest = t.trim_start_matches("func ").trim_start();
        if rest.starts_with('(') {
            // method: func (r *T) Name(
            let after = rest.split(')').nth(1)?.trim();
            return Some((SignatureKind::Function, first_ident(after)?));
        }
        return Some((SignatureKind::Function, first_ident(rest)?));
    }
    if t.starts_with("type ") {
        return Some((SignatureKind::Type, ident_after(t, "type ")?));
    }
    None
}

fn cs_sig(t: &str) -> Option<(SignatureKind, String)> {
    if let Some(name) = type_after(
        t,
        &[
            "public class ",
            "class ",
            "public interface ",
            "interface ",
            "public struct ",
            "struct ",
            "public enum ",
            "enum ",
            "public record ",
            "record ",
        ],
    ) {
        return Some((SignatureKind::Type, name));
    }
    None
}

fn php_sig(t: &str) -> Option<(SignatureKind, String)> {
    if t.contains("function ") {
        return Some((SignatureKind::Function, ident_after(t, "function ")?));
    }
    if let Some(name) = type_after(t, &["class ", "interface ", "trait "]) {
        return Some((SignatureKind::Type, name));
    }
    None
}

fn cpp_sig(t: &str) -> Option<(SignatureKind, String)> {
    if let Some(name) = type_after(t, &["class ", "struct ", "enum ", "enum class "]) {
        return Some((SignatureKind::Type, name));
    }
    None
}

fn md_sig(t: &str) -> Option<(SignatureKind, String)> {
    if !t.starts_with('#') {
        return None;
    }
    let hashes = t.bytes().take_while(|&b| b == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = t[hashes..].trim().trim_end_matches('#').trim();
    if rest.is_empty() {
        return None;
    }
    Some((SignatureKind::Heading, rest.to_string()))
}

fn looks_like_fn(t: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| t.starts_with(p))
}

fn type_after(t: &str, prefixes: &[&str]) -> Option<String> {
    for p in prefixes {
        if let Some(rest) = t.strip_prefix(p) {
            return first_ident(rest);
        }
    }
    None
}

fn ident_after(t: &str, marker: &str) -> Option<String> {
    let idx = t.find(marker)?;
    first_ident(&t[idx + marker.len()..])
}

fn first_ident(s: &str) -> Option<String> {
    let s = s.trim_start_matches(['(', '&', '*', '\'']);
    let ident: String = s
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
        .collect();
    if ident.is_empty() { None } else { Some(ident) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_trait_and_fn() {
        let src = "pub trait Auth { fn verify(&self); }\npub fn login() { /* ... */ }";
        let sigs = extract_signatures("src/auth.rs", src, Language::Rust);
        assert!(
            sigs.iter()
                .any(|s| s.name == "Auth" && s.kind == SignatureKind::Type)
        );
        assert!(
            sigs.iter()
                .any(|s| s.name == "login" && s.kind == SignatureKind::Function)
        );
    }

    #[test]
    fn python_class_and_def() {
        let src = "class Repo:\n    ...\ndef fetch():\n    ...";
        let sigs = extract_signatures("repo.py", src, Language::Python);
        assert!(sigs.iter().any(|s| s.name == "Repo"));
        assert!(sigs.iter().any(|s| s.name == "fetch"));
    }

    #[test]
    fn file_blob_is_order_independent() {
        let a = [
            Signature {
                path: "a.rs".into(),
                kind: SignatureKind::Function,
                name: "b".into(),
                text: "fn b()".into(),
            },
            Signature {
                path: "a.rs".into(),
                kind: SignatureKind::Function,
                name: "a".into(),
                text: "fn a()".into(),
            },
        ];
        let mut b = a.clone();
        b.reverse();
        assert_eq!(file_signature_blob(&a), file_signature_blob(&b));
    }

    #[test]
    fn markdown_headings_extracted_as_signatures() {
        let src = "# Architecture Overview\n## Context\nParagraph\n### Invariant Rules ###\n";
        let sigs = extract_signatures("docs/arch.md", src, Language::Markdown);
        assert_eq!(sigs.len(), 3);
        assert_eq!(sigs[0].kind, SignatureKind::Heading);
        assert_eq!(sigs[0].name, "Architecture Overview");
        assert_eq!(sigs[1].kind, SignatureKind::Heading);
        assert_eq!(sigs[1].name, "Context");
        assert_eq!(sigs[2].kind, SignatureKind::Heading);
        assert_eq!(sigs[2].name, "Invariant Rules");
    }
}
