def median(values):
    """Return the median of values. See SPEC.md."""
    values.sort()
    n = len(values)
    mid = n // 2
    if n % 2 == 1:
        return values[mid]
    return values[mid] + values[mid + 1] / 2
