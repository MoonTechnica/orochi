import importlib.util
from pathlib import Path
import unittest
s=importlib.util.spec_from_file_location('quota_terminal', Path(__file__).resolve().parents[1]/'src/quota_terminal.py')
m=importlib.util.module_from_spec(s);s.loader.exec_module(m)

class QuotaTerminal(unittest.TestCase):
    def test_partial_ansi_updates_replace_old_percentages(self):
        s=m.Screen()
        s.feed('Current session\r\n40% used\r\nResets 3:40am (Asia/Tokyo)')
        s.feed('\x1b[2;1H4')
        s.feed('2% used\x1b[2;9H\x1b[K')
        w=m.parse_screen('claude_usage',s.text(),1789487000)
        self.assertEqual(len(w),1)
        self.assertAlmostEqual(w[0]['remaining'],.58)
        self.assertGreater(w[0]['reset_at'],1789487000)
    def test_context_usage_and_unlabeled_percentages_are_not_quota(self):
        for kind in ('claude_usage','antigravity_usage'):
            self.assertEqual(m.parse_screen(kind,'Context window remaining: 99%\nTokens: 70% used',0),[])
    def test_model_scoped_antigravity_windows_require_direction_and_id(self):
        w=m.parse_screen('antigravity_usage','Model Quotas\ngemini-3.1-pro 23% remaining\nclaude-sonnet 100% used\nFriendly Model 55%',0)
        self.assertEqual(len(w),2)
        self.assertEqual(w[0]['model'],'gemini-3.1-pro')
        self.assertEqual(w[1]['remaining'],0)
    def test_split_escape_and_stale_text_clearing(self):
        s=m.Screen();s.feed('Current session\r\n42% used\x1b[');s.feed('2J\x1b[HContext 90% used')
        self.assertEqual(m.parse_screen('claude_usage',s.text(),0),[])

if __name__ == '__main__': unittest.main()
