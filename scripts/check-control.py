#!/usr/bin/env python3
"""Exercise the running demo through the actual CLI and private instance socket."""
import json
import os
import pathlib
import subprocess
import tempfile
import time

binary = pathlib.Path(os.environ.get("TG_PLAYER_BINARY", pathlib.Path(__file__).resolve().parents[1] / "target/debug/tg-player"))


with tempfile.TemporaryDirectory(prefix="tg-player-control-") as runtime:
    env = os.environ.copy()
    # Keep the real compositor socket accessible while isolating the instance.
    display = env.get("WAYLAND_DISPLAY")
    if display and not os.path.isabs(display):
        env["WAYLAND_DISPLAY"] = str(pathlib.Path(env["XDG_RUNTIME_DIR"]) / display)
    env["XDG_RUNTIME_DIR"] = runtime

    def call(*args, success=True):
        result = subprocess.run([str(binary), "--demo", "--ctl", *map(str, args)],
                                env=env, text=True, capture_output=True, timeout=8)
        reply = json.loads(result.stdout)
        assert reply["ok"] == success, reply
        assert (result.returncode == 0) == success, result
        return reply.get("data", reply)

    assert "not running" in call("status", success=False)["error"]
    call("select", 0, success=False)
    call("seek", "NaN", success=False)
    with tempfile.TemporaryFile(mode="w+") as log:
        app = subprocess.Popen([str(binary), "--demo"], env=env, stdout=log, stderr=log)
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

            assert len(call("playlists")["items"]) == 5
            call("open", 0)
            assert call("playlist")["items"][4]["row"] == 5
            call("select", 2)
            assert call("select", 5)["current"]["message_id"] == 4
            assert call("previous")["current"]["message_id"] == 3
            assert call("next")["current"]["message_id"] == 4

            call("select", 2)
            entry_id = call("queue")["items"][2]["entry_id"]
            assert call("queue-play", entry_id)["current"]["message_id"] == 4
            assert call("previous")["current"]["message_id"] == 3
            call("queue-play", 0, success=False)
            call("open", 999, success=False)

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
            print("CLI: playlist/queue selection, 2 → 5 → Previous → 4, shuffle, transport and errors passed.")
        finally:
            if app.poll() is None:
                app.terminate()
                app.wait(timeout=8)
    assert "not running" in call("status", success=False)["error"]
