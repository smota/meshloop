# Exact expression evaluator

`calc.evaluate(expr)` evaluates an arithmetic expression string exactly.

Syntax:

- Numbers: decimal literals made of digits with an optional fractional part
  (`7`, `0.25`, `12.0`). No exponents, no leading `.` or trailing `.`.
- Binary operators `+`, `-`, `*`, `/` and `^` (power), unary `+` and `-`, and
  parentheses. Whitespace between tokens is ignored.
- Precedence, highest first: `^`; unary `+`/`-`; `*` and `/`; binary `+` and `-`.
  `*`, `/`, `+`, `-` are left-associative. `^` is right-associative, and its right
  operand may carry a unary sign: `2^3^2 == 512`, `-2^2 == -4`, `2^-1 == 1/2`,
  `(-2)^2 == 4`.

Semantics:

- Arithmetic is exact rational arithmetic (`fractions.Fraction`); `0.1 + 0.2` is
  exactly `3/10`. No floats appear anywhere.
- The result is an `int` when it is a whole number, otherwise a `Fraction` in lowest
  terms.
- The exponent of `^` must be a whole number with absolute value at most 1000; anything
  else raises `ValueError`. `0 ^ -1` raises `ZeroDivisionError`. `0 ^ 0 == 1`.
- Division by zero raises `ZeroDivisionError`.
- Any malformed input raises `ValueError`: empty or blank input, unknown characters,
  unbalanced parentheses, empty parentheses, a missing operand, two numbers in a row
  (`1 2`) or implicit multiplication (`2(3)`, `(1)(2)`).

`eval`, `exec`, `compile` and the `ast` module must not be used.

Only `calc.py` may change.
