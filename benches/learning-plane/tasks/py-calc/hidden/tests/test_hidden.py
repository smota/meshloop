import os
import re
import unittest
from fractions import Fraction

import calc
from calc import evaluate


class Values(unittest.TestCase):
    def check(self, expr, expected):
        result = evaluate(expr)
        self.assertEqual(result, expected, expr)
        self.assertIs(type(result), type(expected), expr)

    def test_integers(self):
        self.check("42", 42)
        self.check(" 7 ", 7)
        self.check("10 - 4 - 3", 3)
        self.check("2 * (3 + 4)", 14)
        self.check("((2))", 2)

    def test_division(self):
        self.check("8 / 4 / 2", 1)
        self.check("1 / 4 + 1 / 4", Fraction(1, 2))
        self.check("6 / 4", Fraction(3, 2))

    def test_decimals(self):
        self.check("0.1 + 0.2", Fraction(3, 10))
        self.check("1.5 * 2", 3)
        self.check("12.0", 12)
        self.check("0.125", Fraction(1, 8))

    def test_power(self):
        self.check("2 ^ 10", 1024)
        self.check("2 ^ 3 ^ 2", 512)
        self.check("-2 ^ 2", -4)
        self.check("(-2) ^ 2", 4)
        self.check("2 ^ -1", Fraction(1, 2))
        self.check("2 ^ -2 ^ 2", Fraction(1, 16))
        self.check("(1/2) ^ 2", Fraction(1, 4))
        self.check("4 ^ (2/2)", 4)
        self.check("0 ^ 0", 1)
        self.check("2 * 3 ^ 2", 18)
        self.check("-3 ^ 2 * 2", -18)

    def test_unary(self):
        self.check("-3", -3)
        self.check("--3", 3)
        self.check("+-+3", -3)
        self.check("2 * -3", -6)
        self.check("2 - -3", 5)
        self.check("-(1 + 2)", -3)


class Errors(unittest.TestCase):
    def test_value_errors(self):
        for expr in ["", "   ", "1 +", "* 2", "(1 + 2", "1 + 2)", "()", "1 2",
                     "2(3)", "(1)(2)", "1 $ 2", ".5", "5.", "1..2", "2 ^ 0.5",
                     "2 ^ 1001", "2 ^ (1/2)", "1e3", "abc", "1 + + "]:
            with self.assertRaises(ValueError, msg=repr(expr)):
                evaluate(expr)

    def test_zero_division(self):
        for expr in ["1 / 0", "1 / (2 - 2)", "0 ^ -1", "(1/0) + 1"]:
            with self.assertRaises(ZeroDivisionError, msg=repr(expr)):
                evaluate(expr)

    def test_exponent_bound_ok(self):
        self.assertEqual(evaluate("1 ^ 1000"), 1)
        self.assertEqual(evaluate("1 ^ -1000"), 1)


class NoEval(unittest.TestCase):
    def test_source_does_not_use_eval(self):
        path = os.path.join(os.path.dirname(os.path.abspath(calc.__file__)), "calc.py")
        with open(path, encoding="utf-8") as f:
            src = f.read()
        for pattern in (r"\b(eval|exec|compile|__import__)\s*\(",
                        r"^\s*(import|from)\s+ast\b"):
            self.assertIsNone(re.search(pattern, src, re.MULTILINE), pattern)


if __name__ == "__main__":
    unittest.main()
