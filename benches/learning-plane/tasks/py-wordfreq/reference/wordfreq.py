from collections import Counter


def _words(text):
    word = []
    i = 0
    chars = list(text)
    while i < len(chars):
        c = chars[i]
        if c.isalpha():
            word.append(c)
        elif (
            c == "'"
            and word
            and word[-1].isalpha()
            and i + 1 < len(chars)
            and chars[i + 1].isalpha()
        ):
            word.append(c)
        else:
            if word:
                yield "".join(word)
                word = []
        i += 1
    if word:
        yield "".join(word)


def top_words(text, n):
    """Return the n most frequent words as (word, count) tuples. See SPEC.md."""
    if n <= 0:
        return []
    counts = Counter(w.casefold() for w in _words(text))
    ranked = sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))
    return ranked[:n]
