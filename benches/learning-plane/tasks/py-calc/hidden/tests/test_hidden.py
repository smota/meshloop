import ast
import os
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


BANNED_CALLS = {"eval", "exec", "compile", "__import__"}


def banned_uses(src):
    """Calls to the banned builtins and imports of `ast`, found with an AST walk.

    `re.compile(...)` and other attribute calls are allowed; `builtins.eval(...)` is not.
    """
    found = []
    for node in ast.walk(ast.parse(src)):
        if isinstance(node, ast.Call):
            func = node.func
            if isinstance(func, ast.Name) and func.id in BANNED_CALLS:
                found.append(func.id)
            elif (
                isinstance(func, ast.Attribute)
                and func.attr in BANNED_CALLS
                and isinstance(func.value, ast.Name)
                and func.value.id in ("builtins", "__builtins__")
            ):
                found.append(func.value.id + "." + func.attr)
        elif isinstance(node, ast.Import):
            found += [a.name for a in node.names if a.name.split(".")[0] == "ast"]
        elif isinstance(node, ast.ImportFrom) and (node.module or "").split(".")[0] == "ast":
            found.append(node.module)
    return found


class NoEval(unittest.TestCase):
    def test_checker_itself(self):
        self.assertEqual(banned_uses("import re\nP = re.compile(r'x')\n"), [])
        self.assertEqual(banned_uses("x = eval('1')"), ["eval"])
        self.assertEqual(banned_uses("import builtins\nbuiltins.exec('1')"), ["builtins.exec"])
        self.assertEqual(banned_uses("from ast import parse"), ["ast"])

    def test_source_does_not_use_eval(self):
        path = os.path.join(os.path.dirname(os.path.abspath(calc.__file__)), "calc.py")
        with open(path, encoding="utf-8") as f:
            src = f.read()
        self.assertEqual(banned_uses(src), [])


if __name__ == "__main__":
    unittest.main()
