"""Render STATE.md from events.jsonl (stdlib only). Usage: python orchestration/render_state.py [--watch]"""
import json, sys, time, pathlib

ROOT = pathlib.Path(__file__).parent
EVENTS = ROOT / "events.jsonl"
STATE = ROOT.parent / "STATE.md"


def render():
    last, notes, metrics = {}, [], {}
    for line in EVENTS.read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        e = json.loads(line)
        t = e.get("ticket", "-")
        if e["event"] == "state":
            last[t] = e
        elif e["event"] == "note":
            notes.append(e)
        elif e["event"] == "metric":
            metrics[(e.get("bench"), e.get("metric"))] = e
    out = ["# State (generated from orchestration/events.jsonl - do not hand-edit)", ""]
    for t, e in sorted(last.items(), key=lambda kv: (kv[1].get("workstream", ""), kv[0])):
        out.append(f"- {e.get('workstream','')} {t} [{e['state']}] {e.get('agent','')}: {e.get('summary','')}")
    out += ["", "Metrics:"]
    out += [f"- {b}/{m} = {e['value']}" for (b, m), e in sorted(metrics.items(), key=str)]
    out += ["", "Recent notes:"] + [f"- {n['ts'][11:16]} {n.get('ticket','-')} {n['note']}" for n in notes[-5:]]
    STATE.write_text("\n".join(out) + "\n", encoding="utf-8")
    return "\n".join(out)


if __name__ == "__main__":
    print(render())
    while "--watch" in sys.argv:
        time.sleep(2)
        print("\033[2J\033[H" + render())
