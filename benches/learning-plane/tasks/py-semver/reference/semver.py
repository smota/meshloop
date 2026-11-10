import re
from typing import NamedTuple, Tuple


class Version(NamedTuple):
    major: int
    minor: int
    patch: int
    prerelease: Tuple[str, ...] = ()
    build: Tuple[str, ...] = ()


_NUM = r"0|[1-9][0-9]*"
_IDENT = r"[0-9A-Za-z-]+"
_RE = re.compile(
    rf"({_NUM})\.({_NUM})\.({_NUM})"
    rf"(?:-({_IDENT}(?:\.{_IDENT})*))?"
    rf"(?:\+({_IDENT}(?:\.{_IDENT})*))?"
)


def parse(text):
    """Parse a SemVer 2.0.0 string into a Version. See SPEC.md."""
    if not isinstance(text, str):
        raise ValueError("version must be a string")
    m = _RE.fullmatch(text)
    if not m:
        raise ValueError(f"invalid version {text!r}")
    pre = tuple(m.group(4).split(".")) if m.group(4) else ()
    for ident in pre:
        if ident.isdigit() and len(ident) > 1 and ident[0] == "0":
            raise ValueError(f"leading zero in {ident!r}")
    build = tuple(m.group(5).split(".")) if m.group(5) else ()
    return Version(int(m.group(1)), int(m.group(2)), int(m.group(3)), pre, build)


def _cmp(x, y):
    return (x > y) - (x < y)


def _cmp_ident(a, b):
    a_num, b_num = a.isdigit(), b.isdigit()
    if a_num and b_num:
        return _cmp(int(a), int(b))
    if a_num:
        return -1
    if b_num:
        return 1
    return _cmp(a, b)


def _compare(va, vb):
    c = _cmp(va[:3], vb[:3])
    if c:
        return c
    pa, pb = va.prerelease, vb.prerelease
    if not pa or not pb:
        return _cmp(not pa, not pb)
    for x, y in zip(pa, pb):
        c = _cmp_ident(x, y)
        if c:
            return c
    return _cmp(len(pa), len(pb))


def compare(a, b):
    """Compare two version strings by SemVer precedence. See SPEC.md."""
    return _compare(parse(a), parse(b))


_OPS = (">=", "<=", ">", "<", "=")


def _comparators(token):
    if token.startswith("^") or token.startswith("~"):
        v = parse(token[1:])
        lower = (">=", v)
        if token[0] == "~":
            upper = Version(v.major, v.minor + 1, 0)
        elif v.major > 0:
            upper = Version(v.major + 1, 0, 0)
        elif v.minor > 0:
            upper = Version(0, v.minor + 1, 0)
        else:
            upper = Version(0, 0, v.patch + 1)
        return [lower, ("<", upper)]
    for op in _OPS:
        if token.startswith(op):
            return [(op, parse(token[len(op):]))]
    return [("=", parse(token))]


def _holds(op, v, target):
    c = _compare(v, target)
    return {
        ">=": c >= 0,
        "<=": c <= 0,
        ">": c > 0,
        "<": c < 0,
        "=": c == 0,
    }[op]


def satisfies(version, range_):
    """Whether version satisfies the npm-style range. See SPEC.md."""
    v = parse(version)
    if not isinstance(range_, str):
        raise ValueError("range must be a string")
    sets = []
    for part in range_.split("||"):
        tokens = part.split()
        if not tokens:
            raise ValueError(f"empty comparator set in {range_!r}")
        comps = []
        for token in tokens:
            comps.extend(_comparators(token))
        sets.append(comps)
    for comps in sets:
        if not all(_holds(op, v, t) for op, t in comps):
            continue
        if v.prerelease and not any(
            t.prerelease and t[:3] == v[:3] for _, t in comps
        ):
            continue
        return True
    return False
