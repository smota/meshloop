import unittest

from wordfreq import top_words


class VisibleTests(unittest.TestCase):
    def test_basic(self):
        self.assertEqual(top_words("a b a c b a", 2), [("a", 3), ("b", 2)])

    def test_case(self):
        self.assertEqual(top_words("Hello hello HELLO", 1), [("hello", 3)])


if __name__ == "__main__":
    unittest.main()
