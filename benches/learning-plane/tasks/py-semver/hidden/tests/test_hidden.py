import unittest

from semver import Version, compare, parse, satisfies


class Parse(unittest.TestCase):
    def test_valid(self):
        self.assertEqual(parse("0.0.0"), Version(0, 0, 0))
        self.assertEqual(parse("10.20.30"), Version(10, 20, 30))
        self.assertEqual(parse("1.0.0-0.3.7"), Version(1, 0, 0, ("0", "3", "7")))
        self.assertEqual(parse("1.0.0-x-y-z.--"), Version(1, 0, 0, ("x-y-z", "--")))
        self.assertEqual(parse("1.0.0+001"), Version(1, 0, 0, (), ("001",)))
        self.assertEqual(parse("1.0.0-alpha0.valid"), Version(1, 0, 0, ("alpha0", "valid")))

    def test_invalid(self):
        for bad in ["", "1", "1.2", "1.2.3.4", "01.2.3", "1.02.3", "1.2.03",
                    "v1.2.3", " 1.2.3", "1.2.3 ", "1.2.3-", "1.2.3+", "1.2.3-a..b",
                    "1.2.3-01", "1.2.3-alpha.01", "1.2.3+a..b", "1.2.3-a_b",
                    "-1.2.3", "1.2.-3", "a.b.c", "1.2.3-+b"]:
            with self.assertRaises(ValueError, msg=repr(bad)):
                parse(bad)


class Compare(unittest.TestCase):
    def test_chain(self):
        chain = ["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-alpha.beta", "1.0.0-beta",
                 "1.0.0-beta.2", "1.0.0-beta.11", "1.0.0-rc.1", "1.0.0", "1.0.1",
                 "1.1.0", "2.0.0", "10.0.0"]
        for i, a in enumerate(chain):
            for j, b in enumerate(chain):
                expected = (i > j) - (i < j)
                self.assertEqual(compare(a, b), expected, (a, b))

    def test_numeric_vs_alnum(self):
        self.assertEqual(compare("1.0.0-2", "1.0.0-10"), -1)
        self.assertEqual(compare("1.0.0-10", "1.0.0-a"), -1)
        self.assertEqual(compare("1.0.0-B", "1.0.0-a"), -1)

    def test_build_ignored(self):
        self.assertEqual(compare("1.0.0+a", "1.0.0+b"), 0)
        self.assertEqual(compare("1.0.0-rc.1+x", "1.0.0-rc.1"), 0)

    def test_invalid(self):
        with self.assertRaises(ValueError):
            compare("1.0", "1.0.0")


class Satisfies(unittest.TestCase):
    def test_simple_ops(self):
        self.assertTrue(satisfies("1.2.3", "1.2.3"))
        self.assertTrue(satisfies("1.2.3", "=1.2.3"))
        self.assertTrue(satisfies("1.2.3+build", "1.2.3"))
        self.assertTrue(satisfies("1.2.3", ">=1.2.3"))
        self.assertFalse(satisfies("1.2.3", ">1.2.3"))
        self.assertTrue(satisfies("1.2.2", "<1.2.3"))
        self.assertTrue(satisfies("1.2.3", "<=1.2.3"))

    def test_and_or(self):
        self.assertTrue(satisfies("1.5.0", ">=1.2.0 <2.0.0"))
        self.assertFalse(satisfies("2.0.0", ">=1.2.0 <2.0.0"))
        self.assertTrue(satisfies("3.1.0", "<1.0.0 || >=3.0.0"))
        self.assertTrue(satisfies("0.5.0", "<1.0.0||>=3.0.0"))
        self.assertFalse(satisfies("2.0.0", "<1.0.0 || >=3.0.0"))
        self.assertTrue(satisfies("1.5.0", "  >=1.0.0   <2.0.0  "))

    def test_caret(self):
        self.assertTrue(satisfies("1.2.3", "^1.2.3"))
        self.assertFalse(satisfies("1.2.2", "^1.2.3"))
        self.assertTrue(satisfies("0.2.9", "^0.2.3"))
        self.assertFalse(satisfies("0.3.0", "^0.2.3"))
        self.assertTrue(satisfies("0.0.3", "^0.0.3"))
        self.assertFalse(satisfies("0.0.4", "^0.0.3"))
        self.assertFalse(satisfies("0.1.0", "^0.0.0"))

    def test_tilde(self):
        self.assertTrue(satisfies("1.2.9", "~1.2.3"))
        self.assertFalse(satisfies("1.3.0", "~1.2.3"))
        self.assertTrue(satisfies("0.0.5", "~0.0.1"))
        self.assertFalse(satisfies("0.1.0", "~0.0.1"))

    def test_prerelease_rule(self):
        self.assertTrue(satisfies("1.2.3-beta", ">=1.2.3-alpha"))
        self.assertFalse(satisfies("1.2.3-beta", ">=1.2.0"))
        self.assertFalse(satisfies("1.2.3-beta", "^1.2.0"))
        self.assertTrue(satisfies("1.2.3-beta", "^1.2.3-alpha"))
        self.assertFalse(satisfies("1.2.4-beta", "^1.2.3-alpha"))
        self.assertTrue(satisfies("1.2.4", "^1.2.3-alpha"))
        self.assertFalse(satisfies("2.0.0-rc.1", "^1.2.3"))
        self.assertFalse(satisfies("2.0.0-rc.1", "<2.0.0"))
        self.assertTrue(satisfies("1.2.3-beta", ">1.2.3-alpha <1.2.3"))
        self.assertTrue(satisfies("1.2.3-beta", ">=1.0.0 || >=1.2.3-alpha"))
        self.assertFalse(satisfies("1.2.3-alpha", ">1.2.3-alpha"))
        self.assertTrue(satisfies("1.2.3-rc.1", "~1.2.3-beta"))

    def test_invalid(self):
        for bad in ["", "  ", "||", ">=1.2", ">= 1.2.3", "^1.x", "~>1.2.3",
                    "1.2.3 ||", ">=1.2.3 <", "=>1.2.3", "^^1.2.3"]:
            with self.assertRaises(ValueError, msg=repr(bad)):
                satisfies("1.2.3", bad)
        with self.assertRaises(ValueError):
            satisfies("1.2", ">=1.0.0")


if __name__ == "__main__":
    unittest.main()
