use brackets::{check, Mismatch};

#[test]
fn balanced() {
    assert_eq!(check("fn f() { a[0] }"), Ok(()));
}

#[test]
fn wrong_closer() {
    assert_eq!(check("(]"), Err(Mismatch::Unexpected { at: 1, found: ']' }));
}
