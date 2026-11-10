# LRUCache

`lru.LRUCache(capacity, on_evict=None)` is a least-recently-used cache.

- `capacity` is an `int`. Negative capacity, or a non-int (including `bool`), raises
  `ValueError`. Capacity `0` is allowed: the cache never stores anything.
- `get(key, default=None)` returns the value and marks the key most recently used. A
  missing key returns `default` and changes nothing.
- `put(key, value)` stores the value and marks the key most recently used. Updating an
  existing key does not evict anything. When a new key would exceed `capacity`, the
  least recently used key is evicted first.
- `on_evict(key, value)` is called once for every eviction caused by capacity, and
  never for updates, `delete` or capacity 0 (nothing is stored, so nothing is
  evicted).
- `delete(key)` removes the key and returns `True`, or returns `False` if it was
  absent.
- `peek(key, default=None)` returns the value without changing recency.
- `len(cache)` is the number of stored keys; `key in cache` tests membership without
  changing recency.
- `keys()` returns a list of keys from most to least recently used.
- `get`, `put`, `delete` and `peek` run in O(1) average time.

Only `lru.py` may change.
