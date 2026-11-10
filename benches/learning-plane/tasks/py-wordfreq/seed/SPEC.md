# top_words

`wordfreq.top_words(text, n)` returns the `n` most frequent words in `text` as a list
of `(word, count)` tuples.

- A word is a maximal run of letters (`str.isalpha()`, so Unicode letters count) and
  apostrophes, where an apostrophe counts only between two letters: in `don't` it is
  part of the word, in `'quoted'` or `dogs'` it is not.
- Words are compared case-insensitively using `str.casefold()`, and are returned
  casefolded (`"Straße"` and `"STRASSE"` are the same word, returned as `"strasse"`).
- Order: count descending, then word ascending (plain string order).
- Fewer than `n` distinct words: return all of them. `n <= 0`: return `[]`.

Only `wordfreq.py` may change.
