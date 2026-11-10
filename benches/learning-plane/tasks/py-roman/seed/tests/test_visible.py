import unittest

from roman import from_roman, to_roman


class VisibleTests(unittest.TestCase):
    def test_to_roman(self):
        self.assertEqual(to_roman(1994), "MCMXCIV")

    def test_from_roman(self):
        self.assertEqual(from_roman("MMXXIV"), 2024)


if __name__ == "__main__":
    unittest.main()
