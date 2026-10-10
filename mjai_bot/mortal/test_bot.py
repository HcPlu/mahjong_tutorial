"""Unit tests for bot.py's protocol proxy, using a fake mortal.py engine.

The fake engine lives in a temp dir and records every line it receives
(one file per line, to avoid buffering subtleties). It replies to a
tsumo/dahai for seat 0 with a canned reaction and stays silent otherwise,
mirroring how libriichi.mjai.Bot.react only returns at decision points.

Run: python test_bot.py   (stdlib only)
"""

import json
import os
import subprocess
import sys
import tempfile
import threading
import time
import unittest

BOT_DIR = os.path.dirname(os.path.abspath(__file__))

FAKE_ENGINE = r'''
import json
import os
import sys
sys.stdin.reconfigure(encoding='utf-8')
out = open(os.environ["FAKE_ENGINE_LOG"], 'w', encoding='utf-8')
mortal_style = os.environ.get("FAKE_STYLE") == "mortal"
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    out.write(line + "\n")
    out.flush()
    e = json.loads(line)
    if e.get("type") == "tsumo" and e.get("actor") == 0:
        if mortal_style:
            # legal: id0 discard 1m, id1 discard 2m, id37 riichi (mjai.app format)
            print('{"type":"dahai","actor":0,"pai":"1m","tsumogiri":false,'
                  '"meta":{"q_values":[0.12,-4.5,-5.0],'
                  '"mask_bits":' + str((1 << 0) | (1 << 1) | (1 << 37)) + ','
                  '"is_greedy":true,"shanten":2}}', flush=True)
        else:
            print('{"type":"dahai","actor":0,"pai":"1m","tsumogiri":false}', flush=True)
    if e.get("type") == "dahai" and e.get("actor") == 1:
        if mortal_style:
            # declined call: id41 pon, id45 pass — pass has the higher Q
            print('{"type":"none",'
                  '"meta":{"q_values":[-3.0,-0.1],'
                  '"mask_bits":' + str((1 << 41) | (1 << 45)) + '}}', flush=True)
        else:
            print('{"type":"none","meta":{"note":"pass"}}', flush=True)
    if e.get("type") == "end_game":
        print('{"type":"none"}', flush=True)
        break
out.close()
'''


class BotProxyTestCase(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory()
        cls.engine_log = os.path.join(cls.tmp.name, "engine_input.log")
        with open(os.path.join(cls.tmp.name, "mortal.py"), "w", encoding="utf-8") as f:
            f.write(FAKE_ENGINE)
        cls.settings = os.path.join(cls.tmp.name, "bot_config.json")
        with open(cls.settings, "w", encoding="utf-8") as f:
            json.dump(
                {"python_path": sys.executable, "mortal_dir": cls.tmp.name},
                f,
            )

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def run_bot(self, batches, style=None):
        env = {**os.environ, "AKAGI_BOT_CONFIG": self.settings,
               "FAKE_ENGINE_LOG": self.engine_log}
        if style:
            env["FAKE_STYLE"] = style
        proc = subprocess.run(
            [sys.executable, os.path.join(BOT_DIR, "bot.py"), "0"],
            input="".join(json.dumps(b) + "\n" for b in batches),
            capture_output=True,
            text=True,
            encoding="utf-8",
            env=env,
            timeout=60,
        )
        return proc

    def engine_lines(self):
        with open(self.engine_log, encoding="utf-8") as f:
            return [line.strip() for line in f if line.strip()]

    def test_forwards_batch_as_single_events_and_replies_once(self):
        batch = [
            {"type": "start_game", "names": ["A", "B", "C", "D"], "id": 0},
            {"type": "start_kyoku", "bakaze": "E", "kyoku": 1, "honba": 0, "oya": 0,
             "dora_marker": "1m", "scores": [25000, 25000, 25000, 25000],
             "tehais": [["?"] * 13] * 4},
            {"type": "tsumo", "actor": 0, "pai": "2m"},
        ]
        proc = self.run_bot([batch])
        self.assertEqual(proc.returncode, 0, proc.stderr)
        replies = [line for line in proc.stdout.splitlines() if line.strip()]
        self.assertEqual(len(replies), 1, f"expected exactly one reply, got {replies}")
        reply = json.loads(replies[0])
        self.assertEqual(reply["type"], "dahai")
        self.assertEqual(reply["pai"], "1m")

        sent = [json.loads(line) for line in self.engine_lines()]
        self.assertEqual(len(sent), 3, "each batch event must be forwarded individually")
        self.assertEqual([e["type"] for e in sent], ["start_game", "start_kyoku", "tsumo"])

    def test_non_decision_batch_gets_none(self):
        batch = [{"type": "dahai", "actor": 1, "pai": "9s", "tsumogiri": True}]
        proc = self.run_bot([batch])
        self.assertEqual(proc.returncode, 0, proc.stderr)
        replies = [line for line in proc.stdout.splitlines() if line.strip()]
        # The fake engine does answer a call window with a pass — the proxy
        # must forward that rather than synthesizing its own none.
        self.assertEqual(len(replies), 1)
        reply = json.loads(replies[0])
        self.assertEqual(reply["type"], "none")
        self.assertIn("meta", reply)

    def test_end_game_exits_cleanly(self):
        batches = [
            [{"type": "start_game", "names": ["A", "B", "C", "D"], "id": 0}],
            [{"type": "end_game"}],
        ]
        proc = self.run_bot(batches)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        replies = [line for line in proc.stdout.splitlines() if line.strip()]
        self.assertEqual(len(replies), 2)
        self.assertEqual(json.loads(replies[-1])["type"], "none")
        self.assertIn('{"type":"end_game"}', self.engine_lines())

    def test_mortal_meta_becomes_show_card(self):
        self.settings = os.path.join(self.tmp.name, "bot_config_mortal.json")
        with open(self.settings, "w", encoding="utf-8") as f:
            json.dump(
                {"python_path": sys.executable, "mortal_dir": self.tmp.name},
                f,
            )
        batch = [{"type": "tsumo", "actor": 0, "pai": "2m"}]
        proc = self.run_bot([batch], style="mortal")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        reply = json.loads(proc.stdout.splitlines()[0])
        self.assertEqual(reply["type"], "dahai")
        show = reply["meta"]["show"]
        items = show["items"]
        self.assertEqual(items[0]["label"], "Discard 1m")
        self.assertEqual(items[0]["pais"], ["1m"])
        self.assertEqual(items[0]["color"], "#00ff80")
        self.assertGreater(float(items[0]["value"].rstrip("%")), 90.0)
        labels = [i["label"] for i in items]
        self.assertIn("Discard 2m", labels)
        self.assertIn("Riichi", labels)

    def test_declined_call_none_gets_show_card(self):
        self.settings = os.path.join(self.tmp.name, "bot_config_mortal.json")
        with open(self.settings, "w", encoding="utf-8") as f:
            json.dump(
                {"python_path": sys.executable, "mortal_dir": self.tmp.name},
                f,
            )
        batch = [{"type": "dahai", "actor": 1, "pai": "5s", "tsumogiri": True}]
        proc = self.run_bot([batch], style="mortal")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        reply = json.loads(proc.stdout.splitlines()[0])
        self.assertEqual(reply["type"], "none")
        items = reply["meta"]["show"]["items"]
        self.assertEqual(items[0]["label"], "Pass")
        self.assertGreater(items[0]["prob"], items[1]["prob"])
        self.assertEqual(items[1]["label"], "Pon")


class BuildShowTestCase(unittest.TestCase):
    """Direct unit tests for the mjai.app meta → meta.show translation."""

    @classmethod
    def setUpClass(cls):
        sys.path.insert(0, BOT_DIR)
        import bot

        cls.bot = bot

    def test_red_five_discard_decodes(self):
        # ascending ids: 0 (discard 1m), 34 (discard 5mr) — 5mr has the higher Q
        reaction = {"type": "dahai", "meta": {
            "q_values": [0.9, 1.0], "mask_bits": (1 << 0) | (1 << 34)}}
        show = self.bot.build_show(reaction)
        self.assertEqual(show["items"][0]["label"], "Discard 5mr")
        self.assertEqual(show["items"][0]["pais"], ["5mr"])

    def test_mismatched_lengths_skip_translation(self):
        reaction = {"type": "dahai", "meta": {"q_values": [1.0], "mask_bits": 0b11}}
        self.assertIsNone(self.bot.build_show(reaction))

    def test_missing_meta_returns_none(self):
        self.assertIsNone(self.bot.build_show({"type": "none"}))
        self.assertIsNone(self.bot.build_show({"type": "none", "meta": {"note": "x"}}))

    def test_softmax_probabilities_sum_to_one(self):
        reaction = {"type": "none", "meta": {
            "q_values": [0.0, -1.0], "mask_bits": (1 << 41) | (1 << 45)}}
        show = self.bot.build_show(reaction)
        self.assertAlmostEqual(sum(i["prob"] for i in show["items"]), 1.0, places=3)
        self.assertEqual(show["items"][0]["label"], "Pon")


if __name__ == "__main__":
    unittest.main(verbosity=2)
