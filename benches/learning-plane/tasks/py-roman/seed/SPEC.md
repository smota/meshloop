# Roman numerals

`roman.to_roman(n)` converts an `int` from 1 to 3999 to its canonical (shortest,
standard subtractive) uppercase Roman numeral, for example `1994 -> "MCMXCIV"`.
Anything else (0, negatives, 4000 and above, `bool`, `float`, `str`) raises
`ValueError`.

`roman.from_roman(s)` converts a **canonical** uppercase numeral back to an `int`. It
raises `ValueError` for anything that `to_roman` would not produce, including:

- empty strings, lowercase letters, spaces or other characters;
- more than three repeats (`"IIII"`, `"XXXX"`, `"MMMM"`) or any repeat of `V`, `L`, `D`;
- invalid subtractive pairs (`"IL"`, `"IC"`, `"XD"`, `"VX"`) and non-canonical forms
  (`"IXI"`, `"XCX"`, `"CMCM"`, `"IIV"`).

For every `n` in 1..3999, `from_roman(to_roman(n)) == n`.

Only `roman.py` may change.
