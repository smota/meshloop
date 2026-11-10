# Bracket checker

`brackets::check(input: &str) -> Result<(), Mismatch>` checks that the brackets `()`,
`[]` and `{}` in `input` are balanced and correctly nested.

- Positions (`at`) are **character** indices (Unicode scalar values, as counted by
  `str::chars`), not byte offsets.
- Double-quoted strings are skipped: brackets between `"` and the next unescaped `"`
  are ignored. Inside a string, a backslash escapes the next character, so `"\""` is
  one complete string. Single quotes are ordinary characters.
- The first problem found scanning left to right is returned:
  - `Mismatch::Unexpected { at, found }`: a closing bracket that does not match the
    innermost open bracket, or has none.
  - At end of input, if a string is still open: `Mismatch::UnterminatedString { at }`,
    where `at` is the index of its opening quote.
  - Otherwise at end of input, if brackets are still open:
    `Mismatch::Unclosed { at, open }` for the innermost one (opened last).
- All other characters are ignored.

The `Mismatch` type is already defined in `src/lib.rs`; keep it as is. Only
`src/lib.rs` may change, and no dependencies may be added.
