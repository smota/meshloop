import unittest
from fractions import Fraction

from calc import evaluate


class VisibleTests(unittest.TestCase):
    def test_precedence(self):
        self.assertEqual(evaluate("1 + 2 * 3"), 7)

    def test_exact_division(self):
        self.assertEqual(evaluate("1 / 3"), Fraction(1, 3))


if __name__ == "__main__":
    unittest.main()
