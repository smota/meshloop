import unittest

from roman import from_roman, to_roman


class HiddenTests(unittest.TestCase):
    def test_known(self):
        cases = {1: "I", 4: "IV", 9: "IX", 14: "XIV", 40: "XL", 90: "XC",
                 400: "CD", 900: "CM", 3999: "MMMCMXCIX", 2421: "MMCDXXI"}
        for n, s in cases.items():
            self.assertEqual(to_roman(n), s)
            self.assertEqual(from_roman(s), n)

    def test_round_trip_all(self):
        seen = set()
        for n in range(1, 4000):
            s = to_roman(n)
            self.assertNotIn(s, seen)
            seen.add(s)
            self.assertEqual(from_roman(s), n)

    def test_to_roman_rejects(self):
        for bad in (0, -1, 4000, True, 1.0, "5", None):
            with self.assertRaises(ValueError, msg=repr(bad)):
                to_roman(bad)

    def test_from_roman_rejects(self):
        bad = ["", "iv", "I V", "IIII", "XXXX", "MMMM", "VV", "LL", "DD", "IL", "IC",
               "ID", "IM", "XD", "XM", "VX", "LC", "DM", "IXI", "XCX", "CMCM", "IIV",
               "IVI", "XLX", "CDC", "MCMC", "IXX", "A", "MMMCMXCIXI", "IIX"]
        for s in bad:
            with self.assertRaises(ValueError, msg=s):
                from_roman(s)

    def test_from_roman_rejects_non_str(self):
        with self.assertRaises(ValueError):
            from_roman(5)


if __name__ == "__main__":
    unittest.main()
