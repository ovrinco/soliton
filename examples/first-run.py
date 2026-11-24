#!/usr/bin/env python3
"""Exercise the engine with invented data in a disposable workspace."""
import json
import subprocess
import sys
import tempfile
from pathlib import Path

binary = Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/soliton").resolve()

def open_engine(workspace):
    return subprocess.Popen([str(binary), workspace], stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, text=True)

def call(engine, command, args=None):
    engine.stdin.write(json.dumps({"command": command, "args": args or {}}) + "\n")
    engine.stdin.flush()
    reply = json.loads(engine.stdout.readline())
    if not reply.get("ok"):
        raise RuntimeError(reply)
    return reply["result"]

def close(engine):
    call(engine, "shutdown")
    if engine.wait(timeout=10):
        raise RuntimeError("Engine exited unsuccessfully")

with tempfile.TemporaryDirectory(prefix="soliton-example-") as workspace:
    engine = open_engine(workspace)
    goal = call(engine, "save", {"kind": "goal", "title": "Example project"})
    task = call(engine, "save", {"kind": "task", "title": "Review the example",
                                 "goal_id": goal["id"]})
    call(engine, "planner-trash", {"kind": "task", "id": task["id"]})
    call(engine, "planner-trash-restore", {"kind": "task", "id": task["id"]})
    close(engine)
    engine = open_engine(workspace)
    records = call(engine, "state")["records"]
    restored = next(row for row in records if row["id"] == task["id"])
    assert restored["title"] == task["title"]
    assert restored["goal_id"] == goal["id"]
    close(engine)
    print("PASS: goal and task created; task trashed, restored and retained after restart.")
