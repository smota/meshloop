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


BANNED_NAMES = {"eval", "exec", "compile", "__import__"}
BANNED_MODULES = {"builtins", "importlib", "ast"}


def banned_uses(src):
    """Uses of the banned built-ins, found with an AST walk (lp-3).

    Flags any name `eval`, `exec`, `compile` or `__import__` in any context (a call, an
    alias such as `e = eval`, an argument), a string constant equal to one of those
    names (`getattr(builtins, "eval")`, `__builtins__["eval"]`), an attribute access
    `<x>.eval` and friends on `builtins`/`__builtins__`, and any import of `builtins`,
    `importlib` or `ast`. Attribute calls on other modules, such as `re.compile`, are
    allowed.
    """
    found = []
    for node in ast.walk(ast.parse(src)):
        if isinstance(node, ast.Name) and node.id in BANNED_NAMES:
            found.append(node.id)
        elif isinstance(node, ast.Constant) and node.value in BANNED_NAMES:
            found.append(repr(node.value))
        elif (
            isinstance(node, ast.Attribute)
            and node.attr in BANNED_NAMES
            and isinstance(node.value, ast.Name)
            and node.value.id in ("builtins", "__builtins__")
        ):
            found.append(node.value.id + "." + node.attr)
        elif isinstance(node, ast.Import):
            found += [a.name for a in node.names if a.name.split(".")[0] in BANNED_MODULES]
        elif isinstance(node, ast.ImportFrom):
            module = (node.module or "").split(".")[0]
            if module in BANNED_MODULES:
                found.append(node.module)
    return found


class NoEval(unittest.TestCase):
    def test_checker_allows(self):
        for src in ["import re\nP = re.compile(r'x')\n",
                    "from fractions import Fraction\nx = Fraction('1/2')\n",
                    "def evaluate(expr):\n    return expr  # no eval here\n"]:
            self.assertEqual(banned_uses(src), [], src)

    def test_checker_rejects(self):
        for src in ["x = eval('1')",
                    "e = eval\ne('1')",
                    "from builtins import eval as e",
                    "import builtins\ngetattr(builtins, 'eval')('1')",
                    "__builtins__['eval']('1')",
                    "import builtins as b\nb.eval('1')",
                    "import importlib\nimportlib.import_module('ast')",
                    "import builtins\nbuiltins.exec('1')",
                    "from ast import parse",
                    "f = compile",
                    "g = __import__('os')"]:
            self.assertNotEqual(banned_uses(src), [], src)

    def test_source_does_not_use_eval(self):
        path = os.path.join(os.path.dirname(os.path.abspath(calc.__file__)), "calc.py")
        with open(path, encoding="utf-8") as f:
            src = f.read()
        self.assertEqual(banned_uses(src), [])


if __name__ == "__main__":
    unittest.main()
