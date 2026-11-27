#!/usr/bin/env python3
"""Exercise the running demo through the actual CLI and private instance socket."""
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time

binary = pathlib.Path(os.environ.get("GRAMWAVE_BINARY", pathlib.Path(__file__).resolve().parents[1] / "target/debug/gramwave-dev"))


headless = "--headless" in sys.argv[1:]

with tempfile.TemporaryDirectory(prefix="gramwave-control-") as runtime:
    env = os.environ.copy()
    # Keep the real compositor socket accessible while isolating the instance.
    display = env.get("WAYLAND_DISPLAY")
    if headless:
        env.pop("DISPLAY", None)
        env.pop("WAYLAND_DISPLAY", None)
    elif display and not os.path.isabs(display):
        env["WAYLAND_DISPLAY"] = str(pathlib.Path(env["XDG_RUNTIME_DIR"]) / display)
    env["XDG_RUNTIME_DIR"] = runtime
    env["XDG_STATE_HOME"] = runtime

    def call(*args, success=True):
        result = subprocess.run([str(binary), "--demo", "--ctl", "--json", *map(str, args)],
                                env=env, text=True, capture_output=True, timeout=25)
        reply = json.loads(result.stdout)
        assert reply["ok"] == success, reply
        assert (result.returncode == 0) == success, result
        return reply.get("data", reply)

    def text(*args):
        result = subprocess.run([str(binary), "--demo", "--ctl", *map(str, args)],
                                env=env, text=True, capture_output=True, timeout=25)
        assert result.returncode == 0, result
        return result.stdout

    assert "not running" in call("status", success=False)["error"]
    call("select", 0, success=False)
    call("seek", "NaN", success=False)
    with tempfile.TemporaryFile(mode="w+") as log:
        launch = [str(binary), "--demo"]
        if headless:
            launch.append("--headless")
        app = subprocess.Popen(launch, env=env, stdout=log, stderr=log)
        try:
            for _ in range(60):
                if app.poll() is not None:
                    log.seek(0)
                    raise AssertionError(f"Demo exited: {log.read()}")
                try:
                    call("status")
                    break
                except (AssertionError, json.JSONDecodeError):
                    time.sleep(0.1)
            else:
                raise AssertionError("Demo control did not become available")

            assert "No track selected" in text("status")
            assert len(call("playlists")["items"]) == 5
            assert "Jazz" in text("search", "jazz")
            found = call("search", "jazz")["items"]
            assert len(found) == 1 and found[0]["chat_id"] == 2
            call("open", found[0]["chat_id"])
            assert call("playlist")["items"][0]["chat_id"] == 2
            for _ in range(2):
                assert call("unpin", 2)["pinned"] is False
            assert 2 not in [item["chat_id"] for item in call("pinned")["items"]]
            for _ in range(2):
                assert call("pin", 2)["pinned"] is True
            assert [item["chat_id"] for item in call("pinned")["items"]] == [0, 4, 2]
            call("open", 0)
            assert call("playlist")["items"][4]["row"] == 5
            call("select", 2)
            assert call("select", 5)["current"]["message_id"] == 4
            assert call("previous")["current"]["message_id"] == 3
            assert call("next")["current"]["message_id"] == 4

            call("select", 2)
            entry_id = call("queue")["items"][2]["entry_id"]
            assert call("queue-play", "--id", entry_id)["current"]["message_id"] == 4
            assert call("previous")["current"]["message_id"] == 3
            call("queue-play", 0, success=False)
            call("open", 999, success=False)

            assert "> 4  " in text("queue")
            history = call("queue", -2)
            assert [item["row"] for item in history["items"]] == [2, 3]
            assert call("queue-play", 2)["current"]["message_id"] == 1
            call("queue-add", "1-2", 4, "--next", "--reverse")
            inserted = call("queue")["items"][:3]
            assert [item["message_id"] for item in inserted] == [3, 1, 0]
            call("queue-reverse", "3-5")
            assert [item["message_id"] for item in call("queue")["items"][:3]] == [0, 1, 3]
            call("queue-remove", "2-3", 5)
            assert call("queue")["current"]["message_id"] == 1
            assert call("queue")["current"]["entry_id"] == inserted[1]["entry_id"]

            call("shuffle", "on")
            call("select", 5)
            queue = call("queue")["items"]
            assert sorted(item["message_id"] for item in queue) == [i for i in range(40) if i != 4]
            assert call("previous")["current"]["message_id"] == 4
            first = queue[0]["message_id"]
            assert call("next")["current"]["message_id"] == first
            assert call("previous")["current"]["message_id"] == 4
            assert call("pause")["paused"] is True
            assert call("resume")["paused"] is False
            assert call("repeat", "all")["repeat"] == "all"
            call("seek", 0)
            assert "Player stopped." in text("quit")
            assert app.wait(timeout=2) == 0
            print("CLI: search, pins, playlist/queue selection, transport and graceful quit passed.")
        finally:
            if app.poll() is None:
                app.terminate()
                app.wait(timeout=8)
    assert "not running" in call("status", success=False)["error"]

    if headless:
        failed = subprocess.run([str(binary), "--demo", "--daemon"], env=env,
                                text=True, capture_output=True, timeout=35)
        assert failed.returncode != 0 and "Log file" in failed.stderr, failed
        assert "not running" in call("status", success=False)["error"]

    for _ in range(2 if headless else 1):
        daemon = subprocess.run([*launch, "--daemon"], env=env, text=True,
                                capture_output=True, timeout=35)
        assert daemon.returncode == 0 and "Player started" in daemon.stdout, daemon
        try:
            duplicate = subprocess.run([str(binary), "--demo", "--daemon"], env=env,
                               text=True, capture_output=True, timeout=8)
            assert duplicate.returncode == 0 and "already running" in duplicate.stdout, duplicate
            assert call("status")["opened_chat"] == 0
        finally:
            assert call("quit")["stopped"] is True
    print("Daemon: detached startup, duplicate detection, quit and restart passed.")
