from collections import OrderedDict

_MISSING = object()


class LRUCache:
    """A least-recently-used cache. See SPEC.md."""

    def __init__(self, capacity, on_evict=None):
        if not isinstance(capacity, int) or isinstance(capacity, bool) or capacity < 0:
            raise ValueError("capacity must be a non-negative int")
        self._capacity = capacity
        self._on_evict = on_evict
        self._data = OrderedDict()

    def get(self, key, default=None):
        value = self._data.get(key, _MISSING)
        if value is _MISSING:
            return default
        self._data.move_to_end(key)
        return value

    def peek(self, key, default=None):
        return self._data.get(key, default)

    def put(self, key, value):
        if self._capacity == 0:
            return
        if key in self._data:
            self._data[key] = value
            self._data.move_to_end(key)
            return
        if len(self._data) >= self._capacity:
            old_key, old_value = self._data.popitem(last=False)
            if self._on_evict is not None:
                self._on_evict(old_key, old_value)
        self._data[key] = value

    def delete(self, key):
        if key in self._data:
            del self._data[key]
            return True
        return False

    def keys(self):
        return list(reversed(self._data.keys()))

    def __len__(self):
        return len(self._data)

    def __contains__(self, key):
        return key in self._data
