def median(values):
    """Return the median of values. See SPEC.md."""
    ordered = sorted(values)
    n = len(ordered)
    if n == 0:
        raise ValueError("median of empty data")
    mid = n // 2
    if n % 2 == 1:
        return ordered[mid]
    return (ordered[mid - 1] + ordered[mid]) / 2
