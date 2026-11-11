use brackets::{check, Mismatch};

#[test]
fn empty_and_plain() {
    assert_eq!(check(""), Ok(()));
    assert_eq!(check("no brackets here"), Ok(()));
}

#[test]
fn nested_ok() {
    assert_eq!(check("{[()()]}([]{})"), Ok(()));
}

#[test]
fn stray_closer() {
    assert_eq!(check("a)"), Err(Mismatch::Unexpected { at: 1, found: ')' }));
}

#[test]
fn unclosed_reports_innermost() {
    assert_eq!(check("{ [ ( ) "), Err(Mismatch::Unclosed { at: 2, open: '[' }));
}

#[test]
fn first_problem_wins() {
    assert_eq!(check("(]  )"), Err(Mismatch::Unexpected { at: 1, found: ']' }));
}

#[test]
fn char_indices_not_bytes() {
    assert_eq!(check("é€(]"), Err(Mismatch::Unexpected { at: 3, found: ']' }));
    assert_eq!(check("ü{"), Err(Mismatch::Unclosed { at: 1, open: '{' }));
}

#[test]
fn strings_are_skipped() {
    assert_eq!(check("f(\")]}\")"), Ok(()));
    assert_eq!(check("[\"(\"]"), Ok(()));
}

#[test]
fn escaped_quote_inside_string() {
    assert_eq!(check("(\"a\\\")\")"), Ok(()));
    assert_eq!(check("\"\\\\\")"), Err(Mismatch::Unexpected { at: 4, found: ')' }));
}

#[test]
fn unterminated_string() {
    assert_eq!(check("( \"abc)"), Err(Mismatch::UnterminatedString { at: 2 }));
    assert_eq!(check("\"\\\""), Err(Mismatch::UnterminatedString { at: 0 }));
}

#[test]
fn single_quotes_are_ordinary() {
    assert_eq!(check("'(' )"), Ok(()));
    assert_eq!(check("')'"), Err(Mismatch::Unexpected { at: 1, found: ')' }));
}

#[test]
fn backslash_outside_string_is_ordinary() {
    assert_eq!(check("\\(\\)"), Ok(()));
}
