import unittest

from lru import LRUCache


class HiddenTests(unittest.TestCase):
    def test_capacity_validation(self):
        for bad in (-1, 1.5, "2", True, None):
            with self.assertRaises(ValueError, msg=repr(bad)):
                LRUCache(bad)

    def test_capacity_zero(self):
        evicted = []
        c = LRUCache(0, on_evict=lambda k, v: evicted.append(k))
        c.put("a", 1)
        self.assertEqual(len(c), 0)
        self.assertIsNone(c.get("a"))
        self.assertNotIn("a", c)
        self.assertEqual(evicted, [])

    def test_update_refreshes_without_evicting(self):
        evicted = []
        c = LRUCache(2, on_evict=lambda k, v: evicted.append((k, v)))
        c.put("a", 1)
        c.put("b", 2)
        c.put("a", 10)
        self.assertEqual(evicted, [])
        c.put("c", 3)
        self.assertEqual(evicted, [("b", 2)])
        self.assertEqual(c.get("a"), 10)

    def test_get_default(self):
        c = LRUCache(1)
        self.assertEqual(c.get("x", 42), 42)
        self.assertEqual(c.peek("x", 7), 7)
        self.assertEqual(len(c), 0)

    def test_peek_and_contains_do_not_touch(self):
        c = LRUCache(2)
        c.put("a", 1)
        c.put("b", 2)
        self.assertEqual(c.peek("a"), 1)
        self.assertIn("a", c)
        c.put("c", 3)
        self.assertNotIn("a", c)
        self.assertEqual(c.keys(), ["c", "b"])

    def test_keys_order(self):
        c = LRUCache(3)
        for k in "abc":
            c.put(k, k)
        c.get("a")
        self.assertEqual(c.keys(), ["a", "c", "b"])

    def test_delete(self):
        evicted = []
        c = LRUCache(2, on_evict=lambda k, v: evicted.append(k))
        c.put("a", 1)
        self.assertTrue(c.delete("a"))
        self.assertFalse(c.delete("a"))
        self.assertEqual(len(c), 0)
        c.put("b", 2)
        c.put("c", 3)
        self.assertEqual(evicted, [])

    def test_falsy_values_stored(self):
        c = LRUCache(2)
        c.put("z", 0)
        c.put("n", None)
        self.assertEqual(c.get("z", "missing"), 0)
        self.assertIsNone(c.get("n", "missing"))
        self.assertIn("n", c)

    def test_many_evictions(self):
        evicted = []
        c = LRUCache(3, on_evict=lambda k, v: evicted.append(k))
        for i in range(10):
            c.put(i, i)
        self.assertEqual(evicted, list(range(7)))
        self.assertEqual(c.keys(), [9, 8, 7])


if __name__ == "__main__":
    unittest.main()
