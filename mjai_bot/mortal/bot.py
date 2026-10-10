"""mortal-298k — Akagi bot wrapper around the official Mortal v4 engine.

This is a thin protocol proxy, not an AI: Akagi speaks "one JSON *array* of
mjai events per stdin line, exactly one reaction per line", while the upstream
engine (`python mortal.py <seat>` from https://github.com/Equim-chan/Mortal)
speaks "one single event per line, one output line per decision point". The
wrapper forwards each event of a batch individually, collects whatever the
engine emits, and replies to Akagi with the last reaction — or a `none` when
the batch triggered no decision.

Stdlib only on purpose: no pyproject.toml → Akagi activates the bot without
building an environment. All heavy dependencies (torch, libriichi) live in the
engine's own venv, spawned here as a grandchild process. That also keeps the
AGPL engine behind its own process boundary.

Weights: mortal_298k.pth from https://huggingface.co/VoidShine/mortal-298k
(AGPL-3.0, 4-player hanchan only). Never committed — `.gitignore` covers
`mjai_bot/mortal*`.
"""

import json
import math
import os
import queue
import subprocess
import sys
import threading
import time

BOT_DIR = os.path.dirname(os.path.abspath(__file__))
WEIGHTS_FILE = os.path.join(BOT_DIR, "mortal_298k.pth")
ENGINE_CONFIG_FILE = os.path.join(BOT_DIR, "engine.toml")
SETTINGS_FILE = os.path.join(BOT_DIR, "settings.toml")

NOTIFY_PREFIX = "@@AKAGI_NOTIFY@@ "

# Engine paths are machine-local; configure them in the Bots tab (persisted to
# the gitignored settings.toml) or by hand:
#   python_path = 'C:\...\mortal\.venv\Scripts\python.exe'
#   mortal_dir  = 'C:\...\mortal\mortal'
DEFAULT_PYTHON = ""
DEFAULT_MORTAL_DIR = ""

# Akagi aborts us after ~5 s per reaction; leave headroom. The engine's very
# first decision also pays a one-off torch + weights load (normally ~1.5 s,
# spiky under disk contention), which standalone drivers cannot overlap with
# game traffic the way Akagi does — give it a wider one-off budget.
EXPECT_TIMEOUT = 4.0
FIRST_EXPECT_TIMEOUT = 20.0
# Quiet window after the last engine line before we commit a reply, plus the
# grace given to batches that should not trigger any decision at all.
QUIET_GAP = 0.05
GRACE = 0.25

# Event types that, as the *last* event of a batch, mean Akagi expects a real
# decision from us: our own draw, an opponent's discard/added-kan (call
# window), our own meld (the follow-up discard), or our riichi being accepted
# (the declaration discard is still owed).
_CALL_WINDOW = {"dahai", "kakan"}
_OWN_FOLLOWUP = {"chi", "pon", "daiminkan", "ankan", "kakan", "reach_accepted"}

# ---------------------------------------------------------------------------
# meta.show translation (mortal → HUD card)
#
# mortal.py's meta is the mjai.app wire format: `q_values` holds the Q values
# of the *legal* actions only, in ascending order of the action ids whose bits
# are set in `mask_bits` (libriichi agent/mortal.rs gen_meta). The action-id
# space is the public mjai.app protocol (libriichi consts.rs):
#
#   0..=33 discard (1m..9m 1p..9p 1s..9s E S W N P F C)
#   34..=36 discard red five (5mr 5pr 5sr)
#   37 riichi / 38..=40 chi low/mid/high / 41 pon / 42 kan
#   43 agari / 44 kyuushu ryuukyoku / 45 pass
#
# Akagi's HUD (Overlay / BotShowTile) only renders `meta.show` envelopes, so
# we translate every engine reaction that carries q_values into a ranked
# candidate card. Each item also gets a numeric `prob` (softmax over the
# legal Q values), which is what src/autoplay/delay/probs.rs consumes.
_TILES_34 = [f"{n}{s}" for s in "mps" for n in range(1, 10)] + list("ESWNPFC")
_ACTION_TABLE = (
    [(f"Discard {t}", t) for t in _TILES_34]
    + [(f"Discard {t}", t) for t in ("5mr", "5pr", "5sr")]
    + [("Riichi", None), ("Chi (low)", None), ("Chi (mid)", None), ("Chi (high)", None),
       ("Pon", None), ("Kan", None), ("Agari", None), ("Kyuushu", None), ("Pass", None)]
)
_SHOW_MAX_ITEMS = 8


def build_show(reaction):
    """Return a `meta.show` envelope for an engine reaction, or None."""
    meta = reaction.get("meta")
    if not isinstance(meta, dict):
        return None
    q_values = meta.get("q_values")
    mask_bits = meta.get("mask_bits")
    if not isinstance(q_values, list) or not isinstance(mask_bits, int):
        return None

    ids = [i for i in range(len(_ACTION_TABLE)) if (mask_bits >> i) & 1]
    if not ids or len(ids) != len(q_values):
        return None

    try:
        qs = [float(q) for q in q_values]
    except (TypeError, ValueError):
        return None
    top = max(qs)
    exps = [math.exp(q - top) for q in qs]
    total = sum(exps)
    order = sorted(range(len(ids)), key=lambda k: exps[k], reverse=True)

    items = []
    for rank, k in enumerate(order[:_SHOW_MAX_ITEMS]):
        label, tile = _ACTION_TABLE[ids[k]]
        prob = exps[k] / total
        item = {"label": label, "prob": round(prob, 4), "value": f"{prob * 100:.2f}%"}
        if tile:
            item["pais"] = [tile]
        if rank == 0:
            item["color"] = "#00ff80"
        items.append(item)
    return {"title": "Mortal candidates", "items": items}


def enrich_with_show(reply):
    """Attach meta.show to an engine-originated reply line (never synthesized)."""
    try:
        parsed = json.loads(reply)
    except ValueError:
        return reply
    show = build_show(parsed)
    if show is None:
        return reply
    parsed.setdefault("meta", {})["show"] = show
    return json.dumps(parsed, separators=(",", ":"))


def notify(level, title, body=None):
    payload = {"level": level, "title": title}
    if body is not None:
        payload["body"] = body
    sys.stderr.write(NOTIFY_PREFIX + json.dumps(payload, separators=(",", ":")) + "\n")
    sys.stderr.flush()


def load_settings():
    """Settings precedence: AKAGI_BOT_CONFIG (Akagi merges manifest defaults
    with settings.toml for us) → the bot's own settings.toml (standalone use,
    e.g. sim_game.py or manual piping)."""
    cfg = {}
    path = os.environ.get("AKAGI_BOT_CONFIG")
    if path and os.path.isfile(path):
        try:
            with open(path, encoding="utf-8") as f:
                cfg = json.load(f)
        except (OSError, ValueError):
            pass
    elif os.path.isfile(SETTINGS_FILE):
        try:
            import tomllib

            with open(SETTINGS_FILE, "rb") as f:
                data = tomllib.load(f)
            cfg = {k: v for k, v in data.items() if isinstance(v, str)}
        except (OSError, ValueError, tomllib.TOMLDecodeError):
            pass
    return cfg


def resolve_seat():
    for candidate in (
        sys.argv[1] if len(sys.argv) > 1 else None,
        os.environ.get("AKAGI_PLAYER_ID"),
    ):
        try:
            seat = int(candidate)
            if 0 <= seat <= 3:
                return seat
        except (TypeError, ValueError):
            continue
    return 0


def expects_reaction(events, seat):
    if not events:
        return False
    last = events[-1]
    kind = last.get("type")
    actor = last.get("actor")
    if kind == "tsumo":
        return actor == seat
    if kind in _CALL_WINDOW:
        return actor != seat
    if kind in _OWN_FOLLOWUP:
        return actor == seat
    return False


def ensure_engine_config():
    if os.path.isfile(ENGINE_CONFIG_FILE):
        return ENGINE_CONFIG_FILE
    if not os.path.isfile(WEIGHTS_FILE):
        raise FileNotFoundError(
            f"weights not found: {WEIGHTS_FILE} — download mortal_298k.pth from "
            "https://huggingface.co/VoidShine/mortal-298k"
        )
    state_file = WEIGHTS_FILE.replace("\\", "/")
    with open(ENGINE_CONFIG_FILE, "w", encoding="utf-8") as f:
        f.write(
            "# Engine config consumed by mortal.py (via MORTAL_CFG).\n"
            "# Regenerated by bot.py only when this file is missing.\n\n"
            "[control]\n"
            "version = 4\n"
            "online = false\n"
            f"state_file = '{state_file}'\n"
        )
    return ENGINE_CONFIG_FILE


class Engine:
    def __init__(self, python, mortal_dir, seat, engine_config):
        self.proc = subprocess.Popen(
            [python, "mortal.py", str(seat)],
            cwd=mortal_dir,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            # stderr passes through to Akagi's application log (bot=mortal-298k)
            stderr=None,
            env={**os.environ, "MORTAL_CFG": engine_config},
            text=True,
            encoding="utf-8",
            bufsize=1,
        )
        self.lines = queue.Queue()
        self.dead = False
        self.warm = False  # False until the engine has produced its first line

        def pump():
            try:
                for line in self.proc.stdout:
                    self.lines.put(line.strip())
            except OSError:
                pass
            finally:
                self.dead = True

        threading.Thread(target=pump, daemon=True).start()

    def send(self, event):
        if self.proc.poll() is not None:
            raise BrokenPipeError(f"engine exited with code {self.proc.returncode}")
        self.proc.stdin.write(json.dumps(event, separators=(",", ":")) + "\n")
        self.proc.stdin.flush()

    def collect(self, expect):
        """Drain engine output; return the last complete line or None."""
        if expect:
            timeout = FIRST_EXPECT_TIMEOUT if not self.warm else EXPECT_TIMEOUT
        else:
            timeout = GRACE
        deadline = time.monotonic() + timeout
        last = None
        while True:
            remaining = deadline - time.monotonic()
            if last is not None and remaining > 0:
                remaining = min(remaining, QUIET_GAP)
            if remaining <= 0:
                return last
            try:
                line = self.lines.get(timeout=remaining)
            except queue.Empty:
                return last
            if line:
                last = line
                self.warm = True

    def close(self):
        try:
            if self.proc.poll() is None:
                self.proc.stdin.close()
                try:
                    self.proc.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    self.proc.kill()
        except OSError:
            pass


def main():
    settings = load_settings()
    python = settings.get("python_path") or DEFAULT_PYTHON
    mortal_dir = settings.get("mortal_dir") or DEFAULT_MORTAL_DIR
    seat = resolve_seat()

    if not python or not os.path.isfile(python):
        notify("error", "mortal-298k: engine python not configured",
               "Set python_path/mortal_dir in the Bots tab or in "
               "mjai_bot/mortal/settings.toml")
        python = None
    elif not os.path.isfile(os.path.join(mortal_dir, "mortal.py")):
        notify("error", "mortal-298k: mortal.py not found", mortal_dir)
        python = None

    engine = None
    broken = False
    for raw in sys.stdin:
        raw = raw.strip()
        if not raw:
            continue
        try:
            events = json.loads(raw)
        except ValueError:
            events = []

        reply = None
        if python and not broken:
            if engine is None:
                try:
                    engine = Engine(python, mortal_dir, seat, ensure_engine_config())
                except (OSError, FileNotFoundError) as exc:
                    broken = True
                    notify("error", "mortal-298k: failed to start engine", str(exc))
            if engine is not None and not broken:
                try:
                    for event in events:
                        engine.send(event)
                    reply = engine.collect(expects_reaction(events, seat))
                except (BrokenPipeError, OSError) as exc:
                    broken = True
                    notify("error", "mortal-298k: engine died", str(exc))

        if reply:
            sys.stdout.write(enrich_with_show(reply) + "\n")
        else:
            sys.stdout.write('{"type":"none"}\n')
        sys.stdout.flush()

        if any(e.get("type") == "end_game" for e in events):
            break

    if engine is not None:
        engine.close()


if __name__ == "__main__":
    main()
