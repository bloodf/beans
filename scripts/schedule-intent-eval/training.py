"""The detector's data, read from the Runner's own copy so Python and Rust train and measure alike:
crates/cli/src/schedule_models/training.json (templates in nine languages) and held_out.json
(hand-written messages, never trained on)."""
import itertools, json, pathlib

DATA = pathlib.Path(__file__).resolve().parents[2] / "crates" / "cli" / "src" / "schedule_models"


def generate():
    """(language, label, message): 1 asks to repeat something; 0 is a one-off request, a bare task,
    a habit, or a question. The same expansion as `training_messages` in classifier.rs."""
    rows = []
    for lang, d in json.loads((DATA / "training.json").read_text())["languages"].items():
        for t, r, a in itertools.product(d["tasks"], d["rec"], d["ask"]):
            rows.append((lang, 1, a.format(t=t, r=r)))
        for t, o, a in itertools.product(d["tasks"], d["once"], d["ask"]):
            rows.append((lang, 0, a.format(t=t, r=o)))
        for t in d["tasks"]:
            rows.append((lang, 0, t))
        for h, v in itertools.product(d["habit"], d["hv"]):
            rows.append((lang, 0, h.format(h=v)))
        for q in d["q"]:
            for t in d["tasks"][:2]:
                rows.append((lang, 0, q.format(t=t)))
    return rows


def held_out():
    return [(m["language"], int(m["recurring"]), m["text"]) for m in json.loads((DATA / "held_out.json").read_text())["messages"]]
