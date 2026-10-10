"""Offline inference test: simulate one kyoku of 4p riichi mahjong vs the bot.

The three opponents are deterministic tsumogiri players; seat 0 (oya) is the
bot under test. The driver fabricates every event the game produces and feeds
the bot's own reactions back as events (mjai semantics: a bot's action is part
of the event stream).

Two targets:
  default   — through bot.py (Akagi batch protocol: one JSON array per line,
              exactly one reaction line per batch — like Akagi, batches are
              only cut at points where our seat may act or at boundaries)
  --engine  — through mortal.py directly (line protocol: one event per line,
              a reply printed only at decision points; an opponent's discard
              gets a *probe* instead of a blocking wait because the engine
              stays silent when the bot cannot call)

Run: python sim_game.py [--engine] [--seed N] [--report]
"""

import argparse
import json
import os
import queue
import random
import subprocess
import sys
import threading
import time

BOT_DIR = os.path.dirname(os.path.abspath(__file__))
MORTAL_ROOT = os.path.abspath(os.path.join(BOT_DIR, "..", "..", "..", "mortal"))
MORTAL_DIR = os.path.join(MORTAL_ROOT, "mortal")
VENV_PYTHON = os.path.join(MORTAL_ROOT, ".venv", "Scripts", "python.exe")
ENGINE_CONFIG = os.path.join(BOT_DIR, "engine.toml")

FIRST_REPLY_TIMEOUT = 120.0  # covers model load on the first decision
REPLY_TIMEOUT = 30.0
FLUSH_DRAIN = 2.0   # batch mode: bot.py answers a boundary batch within ~0.3 s
PROBE_GRACE = 0.3

KINDS = [f"{n}{s}" for s in "mps" for n in range(1, 10)] + list("ESWNPFC")
RED = {"5m": "5mr", "5p": "5pr", "5s": "5sr"}


def build_wall(rng):
    wall = [t for t in KINDS for _ in range(4)]
    for plain, red in RED.items():
        wall[wall.index(plain)] = red
    rng.shuffle(wall)
    return wall


def tile34(tile):
    """mjai tile string -> (kind index 0..33); red five counts as its plain kind."""
    t = tile[:-1] if tile.endswith("r") else tile
    return KINDS.index(t)


class Target:
    """The process under test. send() writes; read_wait()/read_probe() read."""

    def __init__(self, engine_mode, seat):
        if engine_mode:
            if not os.path.isfile(VENV_PYTHON):
                sys.exit(f"engine python not found: {VENV_PYTHON}")
            cmd = [VENV_PYTHON, "mortal.py", str(seat)]
            cwd, env = MORTAL_DIR, {**os.environ, "MORTAL_CFG": ENGINE_CONFIG}
        else:
            cmd = [sys.executable, os.path.join(BOT_DIR, "bot.py"), str(seat)]
            cwd, env = BOT_DIR, {**os.environ}
        self.engine_mode = engine_mode
        self.proc = subprocess.Popen(
            cmd, cwd=cwd, env=env,
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=None,
            text=True, encoding="utf-8", bufsize=1,
        )
        self.lines = queue.Queue()

        def pump():
            try:
                for line in self.proc.stdout:
                    self.lines.put(line.strip())
            except OSError:
                pass

        threading.Thread(target=pump, daemon=True).start()
        self.first = True

    def send(self, events, log):
        if self.engine_mode:
            for e in events:
                line = json.dumps(e, separators=(",", ":"))
                log.write(line + "\n")
                self.proc.stdin.write(line + "\n")
        else:
            line = json.dumps(events, separators=(",", ":"))
            log.write(line + "\n")
            self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def _read(self, timeout):
        start = time.monotonic()
        try:
            line = self.lines.get(timeout=timeout)
        except queue.Empty:
            raise TimeoutError(
                f"no reply within {timeout}s (process alive={self.proc.poll() is None})")
        return json.loads(line), time.monotonic() - start

    def read_wait(self):
        timeout = FIRST_REPLY_TIMEOUT if self.first else REPLY_TIMEOUT
        self.first = False
        return self._read(timeout)

    def read_probe(self):
        try:
            return self._read(PROBE_GRACE)
        except TimeoutError:
            return None

    def close(self, log):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=10)
        except Exception:
            self.proc.kill()
        log.close()


class Sim:
    def __init__(self, target, seed, report):
        self.t = target
        self.rng = random.Random(seed)
        self.report = report
        self.latencies = []
        self.reactions = []
        self.wall = build_wall(self.rng)
        self.dora_marker = self.wall.pop(0)
        self.hands = [[self.wall.pop(0) for _ in range(13)] for _ in range(4)]
        self.hand = list(self.hands[0])  # bot's hand, for call-window detection
        self.live_draws = 0
        self.log = open(os.path.join(BOT_DIR, "game_log.jsonl"), "w", encoding="utf-8")
        self.pending = []  # events accumulated since the last batch was cut

    # ---- target interaction ------------------------------------------------
    def emit(self, event):
        """Record a game event into the pending batch."""
        self.pending.append(event)

    def ask(self, desc):
        """Cut a batch the bot must answer; returns its reaction, or None when
        the engine stayed silent (engine mode, no call window after all)."""
        self.t.send(self.pending, self.log)
        self.pending = []
        if self.t.engine_mode:
            got = self._engine_reply(desc)
            if got is None:
                return None
        else:
            got = self.t.read_wait()
        reaction, latency = got
        self.latencies.append(latency)
        self.reactions.append(reaction)
        if self.report:
            print(f"[{latency*1000:7.1f} ms] {desc} -> {reaction.get('type')}")
        return reaction

    def _engine_reply(self, desc):
        # engine mode: own-turn/follow-up events always get a reply; a call
        # window may stay silent, so probe instead of blocking
        if desc.split()[0] in {"tsumo", "chi", "pon", "reach_accepted",
                               "daiminkan", "ankan", "kakan"}:
            return self.t.read_wait()
        return self.t.read_probe()

    def flush(self):
        """Cut a boundary batch nobody reacts to; consume the reply if batch mode."""
        if not self.pending:
            return
        self.t.send(self.pending, self.log)
        self.pending = []
        if not self.t.engine_mode:
            reply, _ = self.t._read(FLUSH_DRAIN)
            if reply.get("type") != "none":
                print(f"warning: boundary batch got {reply.get('type')}")

    # ---- bot hand tracking (for call-window detection only) ----------------
    def apply_to_hand(self, event):
        kind = event.get("type")
        if kind == "tsumo":
            self.hand.append(event["pai"])
        elif kind == "dahai":
            self.hand.remove(event["pai"])
        elif kind in ("chi", "pon", "daiminkan"):
            for t in event.get("consumed", []):
                self.hand.remove(t)
        elif kind == "ankan":
            for t in event.get("consumed", []):
                self.hand.remove(t)
        elif kind == "kakan":
            self.hand.remove(event["pai"])

    def call_window(self, tile):
        """Could the bot call on this discarded tile? (conservative)"""
        idx = tile34(tile)
        plain = [tile34(t) for t in self.hand]
        if plain.count(idx) >= 2:  # pon / daiminkan
            return True
        n = idx % 9
        suit = idx // 9
        if suit < 3:  # chi possible only on suited tiles
            lo, hi = suit * 9, suit * 9 + 9
            has = lambda k: lo <= k < hi and plain.count(k) >= 1
            if 0 <= n - 2 and has(idx - 2) and has(idx - 1):
                return True
            if 0 <= n - 1 and n + 1 < 9 and has(idx - 1) and has(idx + 1):
                return True
            if n + 2 < 9 and has(idx + 1) and has(idx + 2):
                return True
        return False

    # ---- game ----------------------------------------------------------------
    def run(self):
        self.emit({"type": "start_game", "names": ["MORTAL", "P1", "P2", "P3"], "id": 0})
        self.emit({"type": "start_kyoku", "bakaze": "E", "kyoku": 1, "honba": 0,
                   "kyotaku": 0, "oya": 0, "dora_marker": self.dora_marker,
                   "scores": [25000] * 4,
                   "tehais": [self.hands[0], ["?"] * 13, ["?"] * 13, ["?"] * 13]})
        self.flush()

        turn = 0
        while self.wall and self.live_draws < 70:  # 70 live tiles; 14 are dead wall
            actor = turn % 4
            draw = self.wall.pop(0)
            self.live_draws += 1

            if actor != 0:
                self.emit({"type": "tsumo", "actor": actor, "pai": draw})
                self.emit({"type": "dahai", "actor": actor, "pai": draw, "tsumogiri": True})
                if self.call_window(draw):
                    call = self.ask(f"p{actor} dahai {draw}")
                    if call is not None and call.get("type") == "hora":
                        self.finish_hora(call)
                        return
                    if call is not None and call.get("type") != "none":
                        if not self.handle_call(call):
                            return
                        turn = 1  # bot discarded; play continues after seat 0
                        continue
                turn += 1
                continue

            self.emit({"type": "tsumo", "actor": 0, "pai": draw})
            self.apply_to_hand(self.pending[-1])
            reaction = self.ask(f"tsumo p0 {draw}")
            while True:
                rtype = reaction.get("type")
                self.apply_to_hand(reaction)
                if rtype == "dahai":
                    self.emit(reaction)
                    self.flush()
                    turn += 1
                    break
                if rtype == "reach":
                    self.emit(reaction)
                    self.emit({"type": "reach_accepted", "actor": 0})
                    reaction = self.ask("reach_accepted (riichi discard)")
                    continue
                if rtype in ("ankan", "kakan"):
                    self.emit(reaction)
                    self.emit(self.new_dora())
                    rinshan = self.wall.pop()
                    self.emit({"type": "tsumo", "actor": 0, "pai": rinshan})
                    self.apply_to_hand(self.pending[-1])
                    reaction = self.ask(f"{rtype} -> rinshan {rinshan}")
                    continue
                if rtype == "hora":
                    self.finish_hora(reaction)
                    return
                raise RuntimeError(f"bad reaction on own turn: {reaction}")

        self.finish_ryukyoku()

    def handle_call(self, call):
        """Apply the bot's chi/pon/daiminkan; False if the kyoku ended."""
        rtype = call.get("type")
        self.apply_to_hand(call)
        self.emit(call)
        if rtype == "daiminkan":
            self.emit(self.new_dora())
            rinshan = self.wall.pop()
            self.emit({"type": "tsumo", "actor": 0, "pai": rinshan})
            self.apply_to_hand(self.pending[-1])
            reaction = self.ask(f"daiminkan -> rinshan {rinshan}")
        else:
            reaction = self.ask(f"{rtype} follow-up discard")
        self.apply_to_hand(reaction)
        self.emit(reaction)
        self.flush()
        return True

    def new_dora(self):
        return {"type": "dora", "dora_marker": self.wall.pop()}

    def finish_hora(self, reaction):
        self.emit(reaction)
        self.finish()

    def finish_ryukyoku(self):
        self.emit({"type": "ryukyoku"})
        self.finish()

    def finish(self):
        self.emit({"type": "end_kyoku"})
        self.flush()
        self.emit({"type": "end_game"})
        self.flush()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--engine", action="store_true", help="drive mortal.py directly")
    ap.add_argument("--seed", type=int, default=42)
    ap.add_argument("--report", action="store_true", help="print per-decision latency")
    args = ap.parse_args()

    target = Target(args.engine, seat=0)
    sim = Sim(target, args.seed, args.report)
    try:
        sim.run()
    finally:
        target.close(sim.log)

    n = len(sim.latencies)
    if n:
        print(f"\nkyoku finished: {n} decisions, "
              f"avg {sum(sim.latencies)/n*1000:.1f} ms, max {max(sim.latencies)*1000:.1f} ms")
    types = [r.get("type") for r in sim.reactions]
    print(f"reactions: {len(types)} "
          f"(dahai={types.count('dahai')}, none={types.count('none')}, "
          f"reach={types.count('reach')}, calls="
          f"{sum(1 for t in types if t in ('chi', 'pon', 'daiminkan'))}, "
          f"kan={sum(1 for t in types if t in ('ankan', 'kakan'))}, "
          f"hora={types.count('hora')})")
    print("full log: game_log.jsonl")
    return 0


if __name__ == "__main__":
    sys.exit(main())
