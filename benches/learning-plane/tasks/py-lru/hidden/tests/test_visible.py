import unittest

from lru import LRUCache


class VisibleTests(unittest.TestCase):
    def test_evicts_least_recent(self):
        c = LRUCache(2)
        c.put("a", 1)
        c.put("b", 2)
        c.get("a")
        c.put("c", 3)
        self.assertIsNone(c.get("b"))
        self.assertEqual(c.get("a"), 1)
        self.assertEqual(c.get("c"), 3)


if __name__ == "__main__":
    unittest.main()
