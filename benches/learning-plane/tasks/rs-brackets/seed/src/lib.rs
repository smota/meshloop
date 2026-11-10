/// Why `check` rejected its input. See SPEC.md.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mismatch {
    Unexpected { at: usize, found: char },
    Unclosed { at: usize, open: char },
    UnterminatedString { at: usize },
}

/// Check bracket balance. See SPEC.md.
pub fn check(input: &str) -> Result<(), Mismatch> {
    let _ = input;
    todo!()
}
