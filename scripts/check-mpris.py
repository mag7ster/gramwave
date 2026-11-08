#!/usr/bin/env python3
"""Run inside dbus-run-session; exercises the real demo app's MPRIS commands."""
import pathlib
import subprocess
import time

binary = pathlib.Path(__file__).resolve().parents[1] / "target/debug/tg-player"
app = subprocess.Popen([str(binary), "--demo", "--scenario=playing"])
base = ["gdbus", "call", "--session", "--dest", "org.mpris.MediaPlayer2.tgplayer",
        "--object-path", "/org/mpris/MediaPlayer2", "--method"]
player = "org.mpris.MediaPlayer2.Player"


def call(method, *args):
    return subprocess.check_output(base + [method, *args], text=True, stderr=subprocess.DEVNULL)


def get(prop):
    return call("org.freedesktop.DBus.Properties.Get", player, prop)


def expect(prop, value):
    for _ in range(50):
        try:
            result = get(prop)
            if value in result:
                return
        except subprocess.CalledProcessError:
            pass
        time.sleep(0.1)
    raise AssertionError(f"{prop}: expected {value!r}, received {get(prop)!r}")


try:
    expect("LoopStatus", "Track")
    for repeat in ["Playlist", "None", "Track"]:
        call("org.freedesktop.DBus.Properties.Set", player, "LoopStatus", f"<'{repeat}'>")
        expect("LoopStatus", repeat)
    call(player + ".Next")
    expect("Metadata", "Slow motion 05")  # Manual Next must ignore Repeat One.
    call(player + ".Previous")
    expect("Metadata", "Ночной поезд")
    call(player + ".Pause")
    expect("PlaybackStatus", "Paused")
    call(player + ".Play")
    expect("PlaybackStatus", "Playing")
    call("org.freedesktop.DBus.Properties.Set", player, "Volume", "<0.32>")
    expect("Volume", "0.32")
    call("org.freedesktop.DBus.Properties.Set", player, "Shuffle", "<false>")
    expect("Shuffle", "false")
    call("org.mpris.MediaPlayer2.Quit")
    app.wait(timeout=5)
    assert app.returncode == 0
    print("MPRIS: repeat Off/All/One, Next, Previous, Play/Pause, volume, shuffle and Quit passed.")
finally:
    if app.poll() is None:
        app.terminate()
        app.wait(timeout=5)
