# median

`stats.median(values)` returns the median of a non-empty sequence of numbers.

- Odd length: the middle value of the sorted values.
- Even length: the arithmetic mean of the two middle values, as a float.
- The input must not be modified (callers pass lists they keep using).
- Any iterable of numbers is accepted, including tuples and generators.
- An empty input raises `ValueError`.

Only `stats.py` may change.
