from fractions import Fraction

_OPS = set("+-*/^()")
MAX_EXPONENT = 1000


def _tokens(expr):
    out = []
    i = 0
    n = len(expr)
    while i < n:
        c = expr[i]
        if c in " \t\n":
            i += 1
        elif c in _OPS:
            out.append(c)
            i += 1
        elif c.isdigit() and c.isascii():
            j = i
            while j < n and expr[j].isdigit() and expr[j].isascii():
                j += 1
            if j < n and expr[j] == ".":
                k = j + 1
                while k < n and expr[k].isdigit() and expr[k].isascii():
                    k += 1
                if k == j + 1:
                    raise ValueError("bad number")
                j = k
            out.append(Fraction(expr[i:j]))
            i = j
        else:
            raise ValueError(f"unexpected character {c!r}")
    return out


class _Parser:
    def __init__(self, tokens):
        self.t = tokens
        self.i = 0

    def peek(self):
        return self.t[self.i] if self.i < len(self.t) else None

    def take(self):
        tok = self.peek()
        if tok is None:
            raise ValueError("unexpected end")
        self.i += 1
        return tok

    def expr(self):
        value = self.term()
        while self.peek() in ("+", "-"):
            op = self.take()
            rhs = self.term()
            value = value + rhs if op == "+" else value - rhs
        return value

    def term(self):
        value = self.unary()
        while self.peek() in ("*", "/"):
            op = self.take()
            rhs = self.unary()
            if op == "*":
                value = value * rhs
            else:
                if rhs == 0:
                    raise ZeroDivisionError("division by zero")
                value = value / rhs
        return value

    def unary(self):
        tok = self.peek()
        if tok == "-":
            self.take()
            return -self.unary()
        if tok == "+":
            self.take()
            return self.unary()
        return self.power()

    def power(self):
        base = self.atom()
        if self.peek() == "^":
            self.take()
            exponent = self.unary()
            if exponent.denominator != 1 or abs(exponent) > MAX_EXPONENT:
                raise ValueError("bad exponent")
            if base == 0 and exponent < 0:
                raise ZeroDivisionError("zero to a negative power")
            return base ** int(exponent)
        return base

    def atom(self):
        tok = self.take()
        if isinstance(tok, Fraction):
            return tok
        if tok == "(":
            value = self.expr()
            if self.take() != ")":
                raise ValueError("expected )")
            return value
        raise ValueError(f"unexpected {tok!r}")


def evaluate(expr):
    """Evaluate an arithmetic expression exactly. See SPEC.md."""
    tokens = _tokens(expr)
    if not tokens:
        raise ValueError("empty expression")
    parser = _Parser(tokens)
    value = parser.expr()
    if parser.peek() is not None:
        raise ValueError(f"unexpected {parser.peek()!r}")
    if value.denominator == 1:
        return int(value)
    return value
