import unittest

from intervals import merge, subtract


class VisibleTests(unittest.TestCase):
    def test_merge_overlap(self):
        self.assertEqual(merge([(1, 4), (2, 6)]), [(1, 6)])

    def test_subtract_example(self):
        self.assertEqual(subtract([(0, 10)], [(2, 3), (5, 7)]),
                         [(0, 2), (3, 5), (7, 10)])


if __name__ == "__main__":
    unittest.main()
