_TABLE = [
    (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"),
    (100, "C"), (90, "XC"), (50, "L"), (40, "XL"),
    (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I"),
]
_CANON = {}


def to_roman(n):
    """See SPEC.md."""
    if not isinstance(n, int) or isinstance(n, bool) or not 1 <= n <= 3999:
        raise ValueError(f"cannot convert {n!r}")
    out = []
    for value, sym in _TABLE:
        while n >= value:
            out.append(sym)
            n -= value
    return "".join(out)


def from_roman(s):
    """See SPEC.md."""
    if not _CANON:
        for n in range(1, 4000):
            _CANON[to_roman(n)] = n
    if not isinstance(s, str) or s not in _CANON:
        raise ValueError(f"not a canonical numeral: {s!r}")
    return _CANON[s]
