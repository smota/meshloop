# Semantic versions and ranges

Standard library only. Only `semver.py` may change.

## `parse(text) -> Version`

`Version` is already defined in `semver.py`. `parse` accepts exactly the Semantic
Versioning 2.0.0 grammar `MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]` and raises
`ValueError` for anything else:

- `MAJOR`, `MINOR`, `PATCH` are non-negative decimal integers without leading zeros
  (`0` is fine, `01` is not).
- `PRERELEASE` and `BUILD` are non-empty dot-separated identifiers of `[0-9A-Za-z-]`;
  no identifier may be empty. A numeric pre-release identifier must not have a leading
  zero (`alpha.01` is invalid); build identifiers may (`+001` is valid).
- No surrounding whitespace, no `v` prefix.

`prerelease` and `build` are tuples of strings (empty when absent).

## `compare(a, b) -> int`

`a` and `b` are strings. Returns `-1`, `0` or `1` by SemVer 2.0.0 precedence:

- Compare major, minor, patch numerically.
- A version with a pre-release is lower than the same version without one.
- Pre-release identifiers are compared left to right: numeric identifiers compare
  numerically, alphanumeric ones in ASCII order, and numeric is lower than
  alphanumeric. If all compared identifiers are equal, the shorter list is lower.
- Build metadata is ignored: `1.0.0+a` and `1.0.0+b` compare equal.

Example order: `1.0.0-alpha < 1.0.0-alpha.1 < 1.0.0-alpha.beta < 1.0.0-beta <
1.0.0-beta.2 < 1.0.0-beta.11 < 1.0.0-rc.1 < 1.0.0`.

## `satisfies(version, range) -> bool`

`version` is a version string; `range` is a range string. Malformed input in either
raises `ValueError`.

- A range is one or more comparator sets separated by `||` (spaces around it are
  allowed). The range is satisfied if any set is.
- A set is one or more comparators separated by whitespace. All must hold.
- A comparator is an operator immediately followed by a full version: `>=`, `<=`, `>`,
  `<`, `=`, or no operator (meaning `=`), and two shorthands:
  - `^X.Y.Z[-pre]`: `>=X.Y.Z[-pre]` and below the next version that changes the
    left-most non-zero part: `<(X+1).0.0` if `X > 0`; `<0.(Y+1).0` if `X == 0` and
    `Y > 0`; `<0.0.(Z+1)` if `X == Y == 0`.
  - `~X.Y.Z[-pre]`: `>=X.Y.Z[-pre]` and `<X.(Y+1).0`.
  Upper bounds produced by `^` and `~` carry no pre-release tag.
- Pre-release rule: if `version` has a pre-release tag, a set matches only if all its
  comparators hold **and** at least one comparator in that set has a version with the
  same `MAJOR.MINOR.PATCH` and a pre-release tag. So `1.2.3-beta` satisfies
  `>=1.2.3-alpha` but not `>=1.2.0`, and `^1.2.3-alpha` matches `1.2.3-beta` but not
  `1.2.4-beta`.
- Comparisons use `compare`; build metadata is ignored.
