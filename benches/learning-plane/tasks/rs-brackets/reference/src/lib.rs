/// Why `check` rejected its input. See SPEC.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mismatch {
    Unexpected { at: usize, found: char },
    Unclosed { at: usize, open: char },
    UnterminatedString { at: usize },
}

/// Check bracket balance. See SPEC.md.
pub fn check(input: &str) -> Result<(), Mismatch> {
    let mut stack: Vec<(usize, char)> = Vec::new();
    let mut string_start: Option<usize> = None;
    let mut escaped = false;
    for (at, c) in input.chars().enumerate() {
        if string_start.is_some() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                string_start = None;
            }
            continue;
        }
        match c {
            '"' => string_start = Some(at),
            '(' | '[' | '{' => stack.push((at, c)),
            ')' | ']' | '}' => {
                let want = match c {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                match stack.pop() {
                    Some((_, open)) if open == want => {}
                    _ => return Err(Mismatch::Unexpected { at, found: c }),
                }
            }
            _ => {}
        }
    }
    if let Some(at) = string_start {
        return Err(Mismatch::UnterminatedString { at });
    }
    match stack.pop() {
        Some((at, open)) => Err(Mismatch::Unclosed { at, open }),
        None => Ok(()),
    }
}
