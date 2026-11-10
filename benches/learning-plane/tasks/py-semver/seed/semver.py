from typing import NamedTuple, Tuple


class Version(NamedTuple):
    major: int
    minor: int
    patch: int
    prerelease: Tuple[str, ...] = ()
    build: Tuple[str, ...] = ()


def parse(text):
    """Parse a SemVer 2.0.0 string into a Version. See SPEC.md."""
    raise NotImplementedError


def compare(a, b):
    """Compare two version strings by SemVer precedence. See SPEC.md."""
    raise NotImplementedError


def satisfies(version, range_):
    """Whether version satisfies the npm-style range. See SPEC.md."""
    raise NotImplementedError
