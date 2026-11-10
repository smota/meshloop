import unittest

from semver import Version, compare, parse, satisfies


class VisibleTests(unittest.TestCase):
    def test_parse(self):
        self.assertEqual(parse("1.2.3-rc.1+build.5"),
                         Version(1, 2, 3, ("rc", "1"), ("build", "5")))

    def test_compare(self):
        self.assertEqual(compare("1.0.0-alpha", "1.0.0"), -1)

    def test_caret(self):
        self.assertTrue(satisfies("1.9.0", "^1.2.3"))
        self.assertFalse(satisfies("2.0.0", "^1.2.3"))


if __name__ == "__main__":
    unittest.main()
