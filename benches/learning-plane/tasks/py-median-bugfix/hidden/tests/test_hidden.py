import unittest

from stats import median


class HiddenTests(unittest.TestCase):
    def test_single(self):
        self.assertEqual(median([7]), 7)

    def test_even_two(self):
        self.assertEqual(median([1, 2]), 1.5)

    def test_even_returns_float(self):
        result = median([2, 4])
        self.assertEqual(result, 3.0)
        self.assertIsInstance(result, float)

    def test_does_not_mutate(self):
        data = [5, 3, 1, 4]
        median(data)
        self.assertEqual(data, [5, 3, 1, 4])

    def test_tuple(self):
        self.assertEqual(median((9, 1, 5)), 5)

    def test_generator(self):
        self.assertEqual(median(x for x in [10, 30, 20, 40]), 25.0)

    def test_negative_and_float(self):
        self.assertEqual(median([-1.5, -0.5, 2.0]), -0.5)

    def test_empty(self):
        with self.assertRaises(ValueError):
            median([])

    def test_empty_generator(self):
        with self.assertRaises(ValueError):
            median(x for x in [])


if __name__ == "__main__":
    unittest.main()
