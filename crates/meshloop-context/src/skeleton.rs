//! AST skeleton extraction for Rust, TypeScript, Python, Go, C#, PHP, and C++.
//! Prunes internal function and method bodies while preserving public types,
//! interfaces, trait definitions, signatures, and docstrings.

use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    Python,
    Go,
    CSharp,
    Php,
    Cpp,
    Markdown,
    Unknown,
}

impl Language {
    pub fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("rs") => Language::Rust,
            Some("ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs") => Language::TypeScript,
            Some("py") => Language::Python,
            Some("go") => Language::Go,
            Some("cs") => Language::CSharp,
            Some("php") => Language::Php,
            Some("cpp" | "cxx" | "cc" | "hpp" | "hh" | "h") => Language::Cpp,
            Some("md" | "markdown" | "mdown" | "mkd") => Language::Markdown,
            _ => Language::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkeletonResult {
    pub language: Language,
    pub original_lines: usize,
    pub pruned_lines: usize,
    pub original_approx_tokens: usize,
    pub pruned_approx_tokens: usize,
    pub skeleton: String,
}

impl SkeletonResult {
    pub fn savings_percentage(&self) -> f64 {
        if self.original_approx_tokens == 0 {
            return 0.0;
        }
        let saved = self
            .original_approx_tokens
            .saturating_sub(self.pruned_approx_tokens);
        (saved as f64 / self.original_approx_tokens as f64) * 100.0
    }
}

/// Rough token count approximation: ~4 characters per token on average for source code.
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Extracts a skeleton for the given source code in the specified language.
pub fn extract_skeleton(source: &str, language: Language) -> SkeletonResult {
    let original_lines = source.lines().count();
    let original_approx_tokens = estimate_tokens(source);

    let skeleton = match language {
        Language::Rust => prune_rust(source),
        Language::TypeScript => prune_typescript(source),
        Language::Python => prune_python(source),
        Language::Go => prune_go(source),
        Language::CSharp => prune_csharp(source),
        Language::Php => prune_php(source),
        Language::Cpp => prune_cpp(source),
        Language::Markdown => crate::doc_skeleton::prune_markdown(source),
        Language::Unknown => source.to_string(),
    };

    let pruned_lines = skeleton.lines().count();
    let pruned_approx_tokens = estimate_tokens(&skeleton);

    SkeletonResult {
        language,
        original_lines,
        pruned_lines,
        original_approx_tokens,
        pruned_approx_tokens,
        skeleton,
    }
}

// ---------------------------------------------------------------------------
// Rust Pruning
// ---------------------------------------------------------------------------

fn prune_rust(source: &str) -> String {
    let mut out = Vec::new();
    let mut in_fn = false;
    let mut brace_depth = 0;
    let mut fn_brace_depth = 0;

    for line in source.lines() {
        let trimmed = line.trim();

        if in_fn {
            for ch in line.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            if brace_depth <= fn_brace_depth {
                in_fn = false;
            }
            continue;
        }

        // Check if starting a function signature
        let is_fn_start = (trimmed.starts_with("fn ")
            || trimmed.starts_with("pub fn ")
            || trimmed.starts_with("pub async fn ")
            || trimmed.starts_with("async fn ")
            || trimmed.starts_with("pub(crate) fn ")
            || trimmed.starts_with("pub const fn "))
            && !trimmed.ends_with(';');

        if is_fn_start {
            if let Some(brace_pos) = line.find('{') {
                let sig = &line[..brace_pos].trim_end();
                out.push(format!("{sig} {{ /* ... */ }}"));
                brace_depth += 1;
                fn_brace_depth = brace_depth - 1;
                for ch in line[brace_pos + 1..].chars() {
                    if ch == '{' {
                        brace_depth += 1;
                    } else if ch == '}' {
                        brace_depth -= 1;
                    }
                }
                if brace_depth > fn_brace_depth {
                    in_fn = true;
                }
            } else {
                out.push(line.to_string());
                in_fn = true;
                fn_brace_depth = brace_depth;
            }
        } else {
            for ch in line.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            out.push(line.to_string());
        }
    }

    out.join("\n")
}

// ---------------------------------------------------------------------------
// TypeScript / JavaScript Pruning
// ---------------------------------------------------------------------------

fn prune_typescript(source: &str) -> String {
    let mut out = Vec::new();
    let mut in_fn = false;
    let mut brace_depth = 0;
    let mut fn_brace_depth = 0;

    for line in source.lines() {
        let trimmed = line.trim();

        if in_fn {
            for ch in line.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            if brace_depth <= fn_brace_depth {
                in_fn = false;
            }
            continue;
        }

        let is_fn_start = (trimmed.starts_with("function ")
            || trimmed.starts_with("export function ")
            || trimmed.starts_with("export async function ")
            || trimmed.starts_with("async function ")
            || (trimmed.contains(") {")
                && !trimmed.starts_with("if")
                && !trimmed.starts_with("while")
                && !trimmed.starts_with("for")))
            && !trimmed.ends_with(';');

        if is_fn_start {
            if let Some(brace_pos) = line.find('{') {
                let sig = &line[..brace_pos].trim_end();
                out.push(format!("{sig} {{ /* ... */ }}"));
                brace_depth += 1;
                fn_brace_depth = brace_depth - 1;
                for ch in line[brace_pos + 1..].chars() {
                    if ch == '{' {
                        brace_depth += 1;
                    } else if ch == '}' {
                        brace_depth -= 1;
                    }
                }
                if brace_depth > fn_brace_depth {
                    in_fn = true;
                }
            } else {
                out.push(line.to_string());
                in_fn = true;
                fn_brace_depth = brace_depth;
            }
        } else {
            for ch in line.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            out.push(line.to_string());
        }
    }

    out.join("\n")
}

// ---------------------------------------------------------------------------
// Python Pruning
// ---------------------------------------------------------------------------

fn prune_python(source: &str) -> String {
    let mut out = Vec::new();
    let mut in_fn_body = false;
    let mut fn_indent = 0;

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push(String::new());
            continue;
        }

        let current_indent = line.len() - line.trim_start().len();

        if in_fn_body {
            if current_indent <= fn_indent && !trimmed.is_empty() {
                in_fn_body = false;
            } else {
                // Skip inner statements
                continue;
            }
        }

        if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
            out.push(line.to_string());
            let indent_spaces = " ".repeat(current_indent + 4);
            out.push(format!("{indent_spaces}..."));
            in_fn_body = true;
            fn_indent = current_indent;
        } else {
            out.push(line.to_string());
        }
    }

    out.join("\n")
}

// ---------------------------------------------------------------------------
// Go Pruning
// ---------------------------------------------------------------------------

fn prune_go(source: &str) -> String {
    let mut out = Vec::new();
    let mut in_fn = false;
    let mut brace_depth = 0;
    let mut fn_brace_depth = 0;

    for line in source.lines() {
        let trimmed = line.trim();

        if in_fn {
            for ch in line.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            if brace_depth <= fn_brace_depth {
                in_fn = false;
            }
            continue;
        }

        let is_func = trimmed.starts_with("func ") && !trimmed.ends_with(';');

        if is_func {
            if let Some(brace_pos) = line.find('{') {
                let sig = &line[..brace_pos].trim_end();
                out.push(format!("{sig} {{ /* ... */ }}"));
                brace_depth += 1;
                fn_brace_depth = brace_depth - 1;
                for ch in line[brace_pos + 1..].chars() {
                    if ch == '{' {
                        brace_depth += 1;
                    } else if ch == '}' {
                        brace_depth -= 1;
                    }
                }
                if brace_depth > fn_brace_depth {
                    in_fn = true;
                }
            } else {
                out.push(line.to_string());
                in_fn = true;
                fn_brace_depth = brace_depth;
            }
        } else {
            for ch in line.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            out.push(line.to_string());
        }
    }

    out.join("\n")
}

// ---------------------------------------------------------------------------
// C# Pruning
// ---------------------------------------------------------------------------

fn prune_csharp(source: &str) -> String {
    let mut out = Vec::new();
    let mut in_fn = false;
    let mut awaiting_fn_brace = false;
    let mut brace_depth = 0;
    let mut fn_brace_depth = 0;

    for line in source.lines() {
        let trimmed = line.trim();

        if in_fn {
            for ch in line.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            if brace_depth <= fn_brace_depth {
                in_fn = false;
            }
            continue;
        }

        if awaiting_fn_brace {
            if let Some(brace_pos) = line.find('{') {
                let indent = &line[..line.len() - line.trim_start().len()];
                out.push(format!("{indent}{{ /* ... */ }}"));
                brace_depth += 1;
                fn_brace_depth = brace_depth - 1;
                for ch in line[brace_pos + 1..].chars() {
                    if ch == '{' {
                        brace_depth += 1;
                    } else if ch == '}' {
                        brace_depth -= 1;
                    }
                }
                awaiting_fn_brace = false;
                if brace_depth > fn_brace_depth {
                    in_fn = true;
                }
                continue;
            } else {
                out.push(line.to_string());
                continue;
            }
        }

        let is_auto_property = trimmed.contains("{ get;")
            || trimmed.contains("{ get }")
            || trimmed.contains("{ set;")
            || trimmed.contains("{ init;");

        let is_type_decl = trimmed.starts_with("namespace ")
            || trimmed.starts_with("class ")
            || trimmed.contains(" class ")
            || trimmed.starts_with("interface ")
            || trimmed.contains(" interface ")
            || trimmed.starts_with("struct ")
            || trimmed.contains(" struct ")
            || trimmed.starts_with("record ")
            || trimmed.contains(" record ")
            || trimmed.starts_with("enum ")
            || trimmed.contains(" enum ");

        let is_control_flow = trimmed.starts_with("if ")
            || trimmed.starts_with("if(")
            || trimmed.starts_with("for ")
            || trimmed.starts_with("for(")
            || trimmed.starts_with("foreach ")
            || trimmed.starts_with("foreach(")
            || trimmed.starts_with("while ")
            || trimmed.starts_with("while(")
            || trimmed.starts_with("switch ")
            || trimmed.starts_with("switch(")
            || trimmed.starts_with("catch ")
            || trimmed.starts_with("catch(")
            || trimmed.starts_with("using ")
            || trimmed.starts_with("using(")
            || trimmed.starts_with("lock ")
            || trimmed.starts_with("lock(");

        let is_expr_bodied_method = !is_auto_property
            && !is_type_decl
            && !is_control_flow
            && trimmed.contains("=>")
            && trimmed.ends_with(';')
            && trimmed.contains('(');

        let is_method = !is_auto_property
            && !is_type_decl
            && !is_control_flow
            && !trimmed.ends_with(';')
            && trimmed.contains('(')
            && trimmed.contains(')');

        if is_expr_bodied_method {
            if let Some(arrow_pos) = line.find("=>") {
                let sig = line[..arrow_pos].trim_end();
                out.push(format!("{sig} => default!;"));
            } else {
                out.push(line.to_string());
            }
        } else if is_method {
            if let Some(brace_pos) = line.find('{') {
                let sig = line[..brace_pos].trim_end();
                out.push(format!("{sig} {{ /* ... */ }}"));
                brace_depth += 1;
                fn_brace_depth = brace_depth - 1;
                for ch in line[brace_pos + 1..].chars() {
                    if ch == '{' {
                        brace_depth += 1;
                    } else if ch == '}' {
                        brace_depth -= 1;
                    }
                }
                if brace_depth > fn_brace_depth {
                    in_fn = true;
                }
            } else {
                out.push(line.to_string());
                awaiting_fn_brace = true;
                fn_brace_depth = brace_depth;
            }
        } else {
            for ch in line.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            out.push(line.to_string());
        }
    }

    out.join("\n")
}

// ---------------------------------------------------------------------------
// PHP Pruning
// ---------------------------------------------------------------------------

fn prune_php(source: &str) -> String {
    let mut out = Vec::new();
    let mut in_fn = false;
    let mut awaiting_fn_brace = false;
    let mut brace_depth = 0;
    let mut fn_brace_depth = 0;

    for line in source.lines() {
        let trimmed = line.trim();

        if in_fn {
            for ch in line.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            if brace_depth <= fn_brace_depth {
                in_fn = false;
            }
            continue;
        }

        if awaiting_fn_brace {
            if let Some(brace_pos) = line.find('{') {
                let indent = &line[..line.len() - line.trim_start().len()];
                out.push(format!("{indent}{{ /* ... */ }}"));
                brace_depth += 1;
                fn_brace_depth = brace_depth - 1;
                for ch in line[brace_pos + 1..].chars() {
                    if ch == '{' {
                        brace_depth += 1;
                    } else if ch == '}' {
                        brace_depth -= 1;
                    }
                }
                awaiting_fn_brace = false;
                if brace_depth > fn_brace_depth {
                    in_fn = true;
                }
                continue;
            } else {
                out.push(line.to_string());
                continue;
            }
        }

        let is_fn_start = (trimmed.contains("function ") || trimmed.starts_with("function("))
            && !trimmed.starts_with("abstract ")
            && !trimmed.ends_with(';');

        if is_fn_start {
            if let Some(brace_pos) = line.find('{') {
                let sig = &line[..brace_pos].trim_end();
                out.push(format!("{sig} {{ /* ... */ }}"));
                brace_depth += 1;
                fn_brace_depth = brace_depth - 1;
                for ch in line[brace_pos + 1..].chars() {
                    if ch == '{' {
                        brace_depth += 1;
                    } else if ch == '}' {
                        brace_depth -= 1;
                    }
                }
                if brace_depth > fn_brace_depth {
                    in_fn = true;
                }
            } else {
                out.push(line.to_string());
                awaiting_fn_brace = true;
                fn_brace_depth = brace_depth;
            }
        } else {
            for ch in line.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            out.push(line.to_string());
        }
    }

    out.join("\n")
}

// ---------------------------------------------------------------------------
// C++ Pruning
// ---------------------------------------------------------------------------

fn prune_cpp(source: &str) -> String {
    let mut out = Vec::new();
    let mut in_fn = false;
    let mut awaiting_fn_brace = false;
    let mut brace_depth = 0;
    let mut fn_brace_depth = 0;

    for line in source.lines() {
        let trimmed = line.trim();

        if in_fn {
            for ch in line.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            if brace_depth <= fn_brace_depth {
                in_fn = false;
            }
            continue;
        }

        if awaiting_fn_brace {
            if let Some(brace_pos) = line.find('{') {
                let indent = &line[..line.len() - line.trim_start().len()];
                out.push(format!("{indent}{{ /* ... */ }}"));
                brace_depth += 1;
                fn_brace_depth = brace_depth - 1;
                for ch in line[brace_pos + 1..].chars() {
                    if ch == '{' {
                        brace_depth += 1;
                    } else if ch == '}' {
                        brace_depth -= 1;
                    }
                }
                awaiting_fn_brace = false;
                if brace_depth > fn_brace_depth {
                    in_fn = true;
                }
                continue;
            } else {
                out.push(line.to_string());
                continue;
            }
        }

        let is_type_or_decl = trimmed.starts_with('#')
            || trimmed.starts_with("namespace ")
            || trimmed.starts_with("class ")
            || trimmed.contains(" class ")
            || trimmed.starts_with("struct ")
            || trimmed.contains(" struct ")
            || trimmed.starts_with("enum ")
            || trimmed.contains(" enum ")
            || trimmed.starts_with("union ")
            || trimmed.starts_with("template")
            || trimmed.starts_with("typedef ")
            || trimmed.starts_with("using ")
            || trimmed.starts_with("extern ");

        let is_control_flow = trimmed.starts_with("if ")
            || trimmed.starts_with("if(")
            || trimmed.starts_with("for ")
            || trimmed.starts_with("for(")
            || trimmed.starts_with("while ")
            || trimmed.starts_with("while(")
            || trimmed.starts_with("switch ")
            || trimmed.starts_with("switch(")
            || trimmed.starts_with("catch ")
            || trimmed.starts_with("catch(");

        let is_fn = !is_type_or_decl
            && !is_control_flow
            && !trimmed.ends_with(';')
            && trimmed.contains('(')
            && (trimmed.contains(')') || trimmed.ends_with('{'));

        if is_fn {
            if let Some(brace_pos) = line.find('{') {
                let sig = &line[..brace_pos].trim_end();
                out.push(format!("{sig} {{ /* ... */ }}"));
                brace_depth += 1;
                fn_brace_depth = brace_depth - 1;
                for ch in line[brace_pos + 1..].chars() {
                    if ch == '{' {
                        brace_depth += 1;
                    } else if ch == '}' {
                        brace_depth -= 1;
                    }
                }
                if brace_depth > fn_brace_depth {
                    in_fn = true;
                }
            } else {
                out.push(line.to_string());
                awaiting_fn_brace = true;
                fn_brace_depth = brace_depth;
            }
        } else {
            for ch in line.chars() {
                if ch == '{' {
                    brace_depth += 1;
                } else if ch == '}' {
                    brace_depth -= 1;
                }
            }
            out.push(line.to_string());
        }
    }

    out.join("\n")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prunes_rust_function_bodies_preserving_signatures() {
        let code = r#"
/// A public database interface.
pub trait Database {
    fn query(&self, sql: &str) -> Result<Vec<String>, String>;
}

pub struct UserStore {
    conn: String,
}

impl UserStore {
    pub fn new(conn: &str) -> Self {
        let trimmed = conn.trim();
        println!("Connecting to {}", trimmed);
        Self { conn: trimmed.to_string() }
    }

    pub fn get_user(&self, id: u64) -> Option<String> {
        if id == 0 {
            return None;
        }
        Some("Alice".into())
    }
}
"#;

        let res = extract_skeleton(code, Language::Rust);
        assert!(res.skeleton.contains("pub trait Database"));
        assert!(res.skeleton.contains("pub struct UserStore"));
        assert!(
            res.skeleton
                .contains("pub fn new(conn: &str) -> Self { /* ... */ }")
        );
        assert!(
            res.skeleton
                .contains("pub fn get_user(&self, id: u64) -> Option<String> { /* ... */ }")
        );
        assert!(!res.skeleton.contains("Connecting to"));
        assert!(!res.skeleton.contains("id == 0"));
        assert!(res.savings_percentage() > 30.0);
    }

    #[test]
    fn prunes_typescript_preserving_interfaces_and_types() {
        let code = r#"
export interface User {
    id: string;
    email: string;
}

export type Role = "admin" | "member";

export function authenticate(token: string): boolean {
    const decoded = atob(token);
    console.log("validating", decoded);
    return decoded.length > 10;
}
"#;

        let res = extract_skeleton(code, Language::TypeScript);
        assert!(res.skeleton.contains("export interface User"));
        assert!(res.skeleton.contains("export type Role"));
        assert!(
            res.skeleton
                .contains("export function authenticate(token: string): boolean { /* ... */ }")
        );
        assert!(!res.skeleton.contains("atob(token)"));
    }

    #[test]
    fn prunes_python_preserving_classes_and_signatures() {
        let code = r#"
class AuthManager:
    """Manages user authentication sessions."""

    def __init__(self, secret: str):
        self.secret = secret
        self.sessions = {}

    def verify_token(self, token: str) -> bool:
        if not token:
            return False
        parts = token.split(".")
        return len(parts) == 3
"#;

        let res = extract_skeleton(code, Language::Python);
        assert!(res.skeleton.contains("class AuthManager:"));
        assert!(res.skeleton.contains("def __init__(self, secret: str):"));
        assert!(
            res.skeleton
                .contains("def verify_token(self, token: str) -> bool:")
        );
        assert!(res.skeleton.contains("..."));
        assert!(!res.skeleton.contains("parts = token.split"));
    }

    #[test]
    fn prunes_go_preserving_interfaces_and_structs() {
        let code = r#"
package auth

type Service interface {
    Authenticate(token string) (bool, error)
}

type TokenValidator struct {
    secret string
}

func (v *TokenValidator) Authenticate(token string) (bool, error) {
    if len(token) < 8 {
        return false, errors.New("token too short")
    }
    return true, nil
}
"#;

        let res = extract_skeleton(code, Language::Go);
        assert!(res.skeleton.contains("package auth"));
        assert!(res.skeleton.contains("type Service interface"));
        assert!(res.skeleton.contains("type TokenValidator struct"));
        assert!(res.skeleton.contains(
            "func (v *TokenValidator) Authenticate(token string) (bool, error) { /* ... */ }"
        ));
        assert!(!res.skeleton.contains("token too short"));
    }

    #[test]
    fn prunes_csharp_preserving_interfaces_and_auto_properties() {
        let code = r#"
namespace App.Services;

public interface ICalculator
{
    int Calculate(int a, int b);
}

public class Calculator : ICalculator
{
    public int State { get; set; }

    public Calculator(int seed)
    {
        State = seed * 10;
        Console.WriteLine("initialized");
    }

    public int Calculate(int a, int b)
    {
        var result = a + b;
        for (int i = 0; i < 10; i++)
        {
            result += i * State;
            Console.WriteLine($"Computing step {i}: {result}");
        }
        return result;
    }

    public int FastAdd(int a, int b) => a + b;
}
"#;

        let res = extract_skeleton(code, Language::CSharp);
        assert!(res.skeleton.contains("public interface ICalculator"));
        assert!(res.skeleton.contains("int Calculate(int a, int b);"));
        assert!(
            res.skeleton
                .contains("public class Calculator : ICalculator")
        );
        assert!(res.skeleton.contains("public int State { get; set; }"));
        assert!(res.skeleton.contains("public Calculator(int seed)"));
        assert!(res.skeleton.contains("public int Calculate(int a, int b)"));
        assert!(res.skeleton.contains("{ /* ... */ }"));
        assert!(
            res.skeleton
                .contains("public int FastAdd(int a, int b) => default!;")
        );
        assert!(!res.skeleton.contains("State = seed * 10"));
        assert!(!res.skeleton.contains("Computing step"));
        assert!(res.savings_percentage() > 30.0);
    }

    #[test]
    fn prunes_php_preserving_interfaces_and_type_hints() {
        let code = r#"
<?php

namespace App\Http;

interface ControllerInterface
{
    public function handle(Request $request): Response;
}

class UserController implements ControllerInterface
{
    private Logger $logger;

    public function __construct(Logger $logger)
    {
        $this->logger = $logger;
    }

    public function handle(Request $request): Response
    {
        $user = $request->getUser();
        $this->logger->info("handling user " . $user->getId());
        return new JsonResponse($user);
    }
}
"#;

        let res = extract_skeleton(code, Language::Php);
        assert!(res.skeleton.contains("interface ControllerInterface"));
        assert!(
            res.skeleton
                .contains("public function handle(Request $request): Response;")
        );
        assert!(
            res.skeleton
                .contains("class UserController implements ControllerInterface")
        );
        assert!(
            res.skeleton
                .contains("public function __construct(Logger $logger)")
        );
        assert!(
            res.skeleton
                .contains("public function handle(Request $request): Response")
        );
        assert!(res.skeleton.contains("{ /* ... */ }"));
        assert!(!res.skeleton.contains("$this->logger->info"));
        assert!(res.savings_percentage() > 30.0);
    }

    #[test]
    fn prunes_cpp_preserving_classes_and_declarations() {
        let code = r#"
#pragma once
#include <string>

namespace engine {

class EngineService {
public:
    int get_status() const;

    void start(int timeout) {
        setup_network();
        wait_for_handshake(timeout);
        ready_ = true;
    }

private:
    bool ready_{false};
};

int calculate_checksum(const std::string& input)
{
    int sum = 0;
    for (char c : input) {
        sum += static_cast<int>(c);
    }
    return sum;
}

} // namespace engine
"#;

        let res = extract_skeleton(code, Language::Cpp);
        assert!(res.skeleton.contains("#include <string>"));
        assert!(res.skeleton.contains("class EngineService"));
        assert!(res.skeleton.contains("int get_status() const;"));
        assert!(
            res.skeleton
                .contains("void start(int timeout) { /* ... */ }")
        );
        assert!(
            res.skeleton
                .contains("int calculate_checksum(const std::string& input)")
        );
        assert!(res.skeleton.contains("{ /* ... */ }"));
        assert!(!res.skeleton.contains("setup_network()"));
        assert!(!res.skeleton.contains("sum += static_cast<int>(c)"));
        assert!(res.savings_percentage() > 30.0);
    }

    #[test]
    fn language_resolution_from_path() {
        assert_eq!(
            Language::from_path(Path::new("src/main.rs")),
            Language::Rust
        );
        assert_eq!(
            Language::from_path(Path::new("app.tsx")),
            Language::TypeScript
        );
        assert_eq!(
            Language::from_path(Path::new("service.py")),
            Language::Python
        );
        assert_eq!(Language::from_path(Path::new("pkg/auth.go")), Language::Go);
        assert_eq!(
            Language::from_path(Path::new("Domain/User.cs")),
            Language::CSharp
        );
        assert_eq!(
            Language::from_path(Path::new("routes/api.php")),
            Language::Php
        );
        assert_eq!(
            Language::from_path(Path::new("native/core.cpp")),
            Language::Cpp
        );
        assert_eq!(
            Language::from_path(Path::new("include/core.hpp")),
            Language::Cpp
        );
        assert_eq!(
            Language::from_path(Path::new("README.md")),
            Language::Markdown
        );
        assert_eq!(
            Language::from_path(Path::new("docs/spec.markdown")),
            Language::Markdown
        );
        assert_eq!(
            Language::from_path(Path::new("archive.bin")),
            Language::Unknown
        );
    }
}
