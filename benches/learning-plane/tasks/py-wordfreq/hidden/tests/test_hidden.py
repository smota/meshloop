import unittest

from wordfreq import top_words


class HiddenTests(unittest.TestCase):
    def test_tie_break_alphabetical(self):
        self.assertEqual(top_words("pear apple pear apple fig", 3),
                         [("apple", 2), ("pear", 2), ("fig", 1)])

    def test_punctuation_and_digits_split(self):
        self.assertEqual(top_words("one,two;one 3three one.", 5),
                         [("one", 3), ("three", 1), ("two", 1)])

    def test_inner_apostrophe(self):
        self.assertEqual(top_words("don't Don't dont", 5),
                         [("don't", 2), ("dont", 1)])

    def test_outer_apostrophes_dropped(self):
        self.assertEqual(top_words("'quoted' dogs' 'tis", 5),
                         [("dogs", 1), ("quoted", 1), ("tis", 1)])

    def test_double_apostrophe_splits(self):
        self.assertEqual(top_words("rock''n roll", 5),
                         [("n", 1), ("rock", 1), ("roll", 1)])

    def test_casefold(self):
        self.assertEqual(top_words("Straße STRASSE strasse", 1), [("strasse", 3)])

    def test_unicode_letters(self):
        self.assertEqual(top_words("café CAFÉ naïve", 2), [("café", 2), ("naïve", 1)])

    def test_n_larger_than_vocab(self):
        self.assertEqual(top_words("x y", 10), [("x", 1), ("y", 1)])

    def test_n_zero_and_negative(self):
        self.assertEqual(top_words("x y", 0), [])
        self.assertEqual(top_words("x y", -1), [])

    def test_empty_text(self):
        self.assertEqual(top_words("", 3), [])
        self.assertEqual(top_words("123 !!! '", 3), [])


if __name__ == "__main__":
    unittest.main()
