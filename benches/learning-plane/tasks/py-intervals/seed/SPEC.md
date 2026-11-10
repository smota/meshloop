# Half-open integer intervals

An interval is a tuple `(start, end)` of `int`s meaning the half-open range
`[start, end)`. A normalized list is sorted by `start`, has no empty intervals and no
two intervals that overlap or touch.

`intervals.merge(intervals)`

- Accepts any iterable of `(start, end)` pairs (tuples or lists), in any order.
- `start > end` raises `ValueError`. Empty intervals (`start == end`) are dropped.
- Overlapping **and touching** intervals are merged: `(1, 3)` and `(3, 5)` become
  `(1, 5)`.
- Returns a new normalized list of tuples. The input is not modified.

`intervals.subtract(a, b)`

- Returns the normalized list of points that are in some interval of `a` and in no
  interval of `b`. `a` and `b` are arbitrary iterables as for `merge` (unsorted,
  overlapping, empty intervals allowed) and are validated the same way.
- Example: `subtract([(0, 10)], [(2, 3), (5, 7)]) == [(0, 2), (3, 5), (7, 10)]`.

Only `intervals.py` may change.
