import unittest

from checkpoint import first_losses


class FirstLossesTest(unittest.TestCase):
    def test_each_check_gets_its_own_first_revision_including_a_transient(self):
        style, ast = (("one", "style"), ("one", "ast"))
        broken = [set(), {ast}, set(), {style}, {ast, style}]
        located, unresolved = first_losses(broken, {style, ast}, lambda index, cohort: broken[index] & cohort)
        self.assertEqual(located, {style: 3, ast: 1})
        self.assertEqual(unresolved, set())


if __name__ == "__main__":
    unittest.main()
