import unittest

from intervals import merge, subtract


class HiddenMerge(unittest.TestCase):
    def test_touching_merges(self):
        self.assertEqual(merge([(3, 5), (1, 3)]), [(1, 5)])

    def test_disjoint_sorted(self):
        self.assertEqual(merge([(5, 6), (1, 2)]), [(1, 2), (5, 6)])

    def test_empty_dropped(self):
        self.assertEqual(merge([(2, 2), (4, 4)]), [])
        self.assertEqual(merge([(1, 3), (3, 3), (3, 4)]), [(1, 4)])

    def test_contained(self):
        self.assertEqual(merge([(0, 10), (2, 3), (9, 12)]), [(0, 12)])

    def test_negative(self):
        self.assertEqual(merge([(-5, -1), (-1, 2)]), [(-5, 2)])

    def test_invalid(self):
        with self.assertRaises(ValueError):
            merge([(3, 1)])

    def test_lists_and_generator(self):
        self.assertEqual(merge(iter([[1, 2], [2, 3]])), [(1, 3)])

    def test_returns_tuples_and_no_mutation(self):
        data = [[4, 5], [1, 2]]
        result = merge(data)
        self.assertEqual(data, [[4, 5], [1, 2]])
        self.assertTrue(all(isinstance(x, tuple) for x in result))

    def test_empty_input(self):
        self.assertEqual(merge([]), [])


class HiddenSubtract(unittest.TestCase):
    def test_nothing_removed(self):
        self.assertEqual(subtract([(0, 5)], []), [(0, 5)])

    def test_everything_removed(self):
        self.assertEqual(subtract([(2, 4)], [(0, 10)]), [])

    def test_edges(self):
        self.assertEqual(subtract([(0, 10)], [(0, 2), (8, 10)]), [(2, 8)])

    def test_b_unsorted_overlapping(self):
        self.assertEqual(subtract([(0, 10)], [(6, 8), (1, 3), (2, 4), (7, 9)]),
                         [(0, 1), (4, 6), (9, 10)])

    def test_a_unnormalized(self):
        self.assertEqual(subtract([(5, 8), (0, 3), (3, 5)], [(4, 6)]),
                         [(0, 4), (6, 8)])

    def test_b_outside(self):
        self.assertEqual(subtract([(10, 20)], [(0, 5), (25, 30)]), [(10, 20)])

    def test_multiple_a(self):
        self.assertEqual(subtract([(0, 3), (5, 9)], [(2, 6)]), [(0, 2), (6, 9)])

    def test_empty_b_interval_ignored(self):
        self.assertEqual(subtract([(0, 4)], [(2, 2)]), [(0, 4)])

    def test_invalid(self):
        with self.assertRaises(ValueError):
            subtract([(0, 4)], [(3, 1)])
        with self.assertRaises(ValueError):
            subtract([(4, 0)], [])


if __name__ == "__main__":
    unittest.main()
