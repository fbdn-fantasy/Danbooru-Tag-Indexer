"""Native X11 regression: close/relaunch, maximization, and first-visible geometry.

Run under an isolated Xvfb + window manager with APP_BINARY, XDG_DATA_HOME,
and XDG_CONFIG_HOME set. Requires xdotool, wmctrl, and xwininfo.
"""
import json
import os
from pathlib import Path
import subprocess
import time

binary = os.environ["APP_BINARY"]
state_path = Path(os.environ["XDG_CONFIG_HOME"]) / "io.tagcodex.desktop/.window-state.json"


def command(*args):
    return subprocess.check_output(args, text=True, stderr=subprocess.DEVNULL).strip()


def wait_for(check, timeout=60):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.02)
    raise AssertionError("Timed out waiting for native window state")


def visible_window():
    try:
        return command("xdotool", "search", "--onlyvisible", "--name", "^Danbooru Tag Indexer$").splitlines()[0]
    except (subprocess.CalledProcessError, IndexError):
        return None


def geometry(window):
    fields = dict(line.split("=", 1) for line in command("xdotool", "getwindowgeometry", "--shell", window).splitlines())
    return {k: int(fields[k]) for k in ("X", "Y", "WIDTH", "HEIGHT")}


def launch():
    process = subprocess.Popen([binary], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    window = wait_for(visible_window)
    # This observation happens as soon as the native window becomes viewable.
    first = geometry(window)
    return process, window, first


def close(process, window):
    subprocess.run(["wmctrl", "-ic", hex(int(window))], check=True)
    process.wait(timeout=15)
    return json.loads(state_path.read_text())["main"]


process, window, _ = launch()
subprocess.run(["wmctrl", "-ir", hex(int(window)), "-e", "0,140,110,980,660"], check=True)
wait_for(lambda: geometry(window)["WIDTH"] == 980 and geometry(window)["HEIGHT"] == 660)
time.sleep(0.3)
expected = geometry(window)
saved = close(process, window)
assert (saved["width"], saved["height"]) == (980, 660), saved

process, window, first = launch()
assert first == expected, {"expected": expected, "first_visible": first}
time.sleep(0.3)
assert geometry(window) == expected
subprocess.run(["wmctrl", "-ir", hex(int(window)), "-b", "add,maximized_vert,maximized_horz"], check=True)
wait_for(lambda: geometry(window)["WIDTH"] > 980)
time.sleep(0.3)
maximized = geometry(window)
saved = close(process, window)
assert saved["maximized"], saved

process, window, first_maximized = launch()
assert first_maximized == maximized, {"expected": maximized, "first_visible": first_maximized}
subprocess.run(["wmctrl", "-ir", hex(int(window)), "-b", "remove,maximized_vert,maximized_horz"], check=True)
wait_for(lambda: geometry(window)["WIDTH"] == 980)
time.sleep(0.3)
assert geometry(window) == expected, {"restored": geometry(window), "expected": expected}
close(process, window)

# Malformed preferences must not leave the application invisible.
state_path.write_text("{broken json")
process, window, fallback = launch()
assert fallback["WIDTH"] >= 760 and fallback["HEIGHT"] >= 540
close(process, window)
print(json.dumps({"window_state": "PASS", "normal": expected, "first_visible": first,
                  "maximized": first_maximized, "corrupt_preferences": "recovered"}, indent=2))
