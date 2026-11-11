def _validated(intervals):
    out = []
    for start, end in intervals:
        if start > end:
            raise ValueError(f"start {start} > end {end}")
        if start < end:
            out.append((start, end))
    return out


def merge(intervals):
    """Merge half-open intervals. See SPEC.md."""
    result = []
    for start, end in sorted(_validated(intervals)):
        if result and start <= result[-1][1]:
            prev_start, prev_end = result[-1]
            result[-1] = (prev_start, max(prev_end, end))
        else:
            result.append((start, end))
    return result


def subtract(a, b):
    """Points in a and not in b. See SPEC.md."""
    left = merge(a)
    right = merge(b)
    result = []
    for start, end in left:
        cur = start
        for b_start, b_end in right:
            if b_end <= cur or b_start >= end:
                continue
            if b_start > cur:
                result.append((cur, b_start))
            cur = max(cur, b_end)
            if cur >= end:
                break
        if cur < end:
            result.append((cur, end))
    return result
