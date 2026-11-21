"""Temporary (draft PR, never merged): the Windows real-app diagnostic for #15549, #15556 and #15870.

Starts the built photocraft.exe with the control server, drives it through the control channel and
prints the results and the app's own DIAG lines; then drags the window's corner with SendInput while
sampling the screen, on this build and on the v0.23.1 release. Every line starts with DIAG. Nothing
here fails: each step reports what happened and goes on.
"""

import base64
import ctypes
import ctypes.wintypes as wt
import json
import os
import secrets
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
import zipfile
import zlib

PORT = 47811
OUT = tempfile.mkdtemp(prefix="win-diag-")
RELEASE_ZIP = "https://github.com/storytold/photocraft/releases/download/v0.23.1/photocraft-0.23.1-windows-x64-portable.zip"


def say(line):
    print(f"DIAG {line}", flush=True)


# ---- Win32 ----

user32 = ctypes.WinDLL("user32", use_last_error=True)
gdi32 = ctypes.WinDLL("gdi32", use_last_error=True)
try:
    user32.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))
except Exception as e:  # noqa: BLE001
    say(f"dpi awareness: {e}")

user32.GetForegroundWindow.restype = wt.HWND
user32.GetDC.restype = wt.HDC
gdi32.CreateCompatibleDC.restype = wt.HDC
gdi32.CreateCompatibleBitmap.restype = wt.HBITMAP
gdi32.SelectObject.restype = wt.HGDIOBJ
EnumWindowsProc = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)

ULONG_PTR = ctypes.c_size_t


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", wt.LONG), ("dy", wt.LONG), ("mouseData", wt.DWORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD), ("dwExtraInfo", ULONG_PTR)]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", wt.WORD), ("wScan", wt.WORD), ("dwFlags", wt.DWORD), ("time", wt.DWORD), ("dwExtraInfo", ULONG_PTR)]


class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [("uMsg", wt.DWORD), ("wParamL", wt.WORD), ("wParamH", wt.WORD)]


class _INPUTUNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT), ("hi", HARDWAREINPUT)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", wt.DWORD), ("u", _INPUTUNION)]


class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [
        ("biSize", wt.DWORD), ("biWidth", wt.LONG), ("biHeight", wt.LONG), ("biPlanes", wt.WORD), ("biBitCount", wt.WORD),
        ("biCompression", wt.DWORD), ("biSizeImage", wt.DWORD), ("biXPelsPerMeter", wt.LONG), ("biYPelsPerMeter", wt.LONG),
        ("biClrUsed", wt.DWORD), ("biClrImportant", wt.DWORD),
    ]


MOUSEEVENTF_MOVE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_ABSOLUTE = 0x1, 0x2, 0x4, 0x8000
KEYEVENTF_KEYUP, VK_MENU = 0x2, 0x12
SRCCOPY, CAPTUREBLT = 0x00CC0020, 0x40000000
SCREEN_W, SCREEN_H = user32.GetSystemMetrics(0), user32.GetSystemMetrics(1)


def send_mouse(flags, x=0, y=0):
    i = INPUT(type=0)
    if flags & MOUSEEVENTF_ABSOLUTE:
        i.u.mi.dx = int(x * 65535 / max(SCREEN_W - 1, 1))
        i.u.mi.dy = int(y * 65535 / max(SCREEN_H - 1, 1))
    i.u.mi.dwFlags = flags
    user32.SendInput(1, ctypes.byref(i), ctypes.sizeof(INPUT))


def mouse_to(x, y):
    send_mouse(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, x, y)


def tap_alt():
    for flags in (0, KEYEVENTF_KEYUP):
        i = INPUT(type=1)
        i.u.ki.wVk = VK_MENU
        i.u.ki.dwFlags = flags
        user32.SendInput(1, ctypes.byref(i), ctypes.sizeof(INPUT))


def window_of(pid, timeout):
    """The largest visible top-level window of process `pid`."""
    end = time.time() + timeout
    while time.time() < end:
        found = []

        def each(hwnd, _):
            p = wt.DWORD()
            user32.GetWindowThreadProcessId(hwnd, ctypes.byref(p))
            if p.value == pid and user32.IsWindowVisible(hwnd):
                r = rect_of(hwnd)
                found.append(((r[2] - r[0]) * (r[3] - r[1]), hwnd))
            return True

        user32.EnumWindows(EnumWindowsProc(each), 0)
        found = [f for f in found if f[0] > 200 * 200]
        if found:
            return max(found)[1]
        time.sleep(0.5)
    return None


def rect_of(hwnd):
    r = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    return (r.left, r.top, r.right, r.bottom)


def cursor():
    p = wt.POINT()
    user32.GetCursorPos(ctypes.byref(p))
    return (p.x, p.y)


def place(hwnd, x, y, w, h):
    user32.ShowWindow(hwnd, 9)  # SW_RESTORE
    ok = user32.SetWindowPos(hwnd, 0, x, y, w, h, 0x0004 | 0x0010)  # SWP_NOZORDER | SWP_NOACTIVATE
    time.sleep(1.0)
    say(f"window placed ok={ok} rect={rect_of(hwnd)} foreground={user32.GetForegroundWindow() == hwnd}")


def focus(hwnd):
    if user32.GetForegroundWindow() != hwnd:
        tap_alt()
        user32.SetForegroundWindow(hwnd)
        time.sleep(0.5)
    say(f"focus asked: foreground={user32.GetForegroundWindow() == hwnd}")


def grab(x, y, w, h):
    """BGRA bytes of the screen region, top-down."""
    screen = user32.GetDC(0)
    mem = gdi32.CreateCompatibleDC(screen)
    bmp = gdi32.CreateCompatibleBitmap(screen, w, h)
    old = gdi32.SelectObject(mem, bmp)
    gdi32.BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT)
    bi = BITMAPINFOHEADER(biSize=ctypes.sizeof(BITMAPINFOHEADER), biWidth=w, biHeight=-h, biPlanes=1, biBitCount=32, biCompression=0)
    buf = ctypes.create_string_buffer(w * h * 4)
    gdi32.GetDIBits(mem, bmp, 0, h, buf, ctypes.byref(bi), 0)
    gdi32.SelectObject(mem, old)
    gdi32.DeleteObject(bmp)
    gdi32.DeleteDC(mem)
    user32.ReleaseDC(0, screen)
    return buf.raw


def png(bgra, w, h):
    rows = bytearray()
    for y in range(h):
        rows.append(0)
        row = bgra[y * w * 4:(y + 1) * w * 4]
        rgb = bytearray(w * 3)
        rgb[0::3], rgb[1::3], rgb[2::3] = row[2::4], row[1::4], row[0::4]
        rows += rgb

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(bytes(rows), 9)) + chunk(b"IEND", b"")


def print_png(name, bgra, w, h):
    data = base64.b64encode(png(bgra, w, h)).decode()
    say(f"png {name}: {w}x{h}, {len(data)} base64 chars")
    for i in range(0, len(data), 2000):
        print(f"DIAG-PNG {name} {data[i:i + 2000]}", flush=True)


def black_share(bgra, origin, w, h, window, start):
    """The share of pure-black pixels inside `window` but outside `start` (both screen rects)."""
    ox, oy = origin
    total = black = 0
    for sy in range(max(window[1], oy), min(window[3], oy + h)):
        row = (sy - oy) * w * 4
        for sx in range(max(window[0], ox), min(window[2], ox + w)):
            if start[0] <= sx < start[2] and start[1] <= sy < start[3]:
                continue
            i = row + (sx - ox) * 4
            total += 1
            if bgra[i] <= 4 and bgra[i + 1] <= 4 and bgra[i + 2] <= 4:
                black += 1
    return black, total


def corner_drags(hwnd, tag):
    """#15870: drag the bottom-right corner outward, fast and slow, sampling the screen."""
    place(hwnd, 40, 30, 640, 460)
    start = rect_of(hwnd)
    probe = grab(start[0], start[1], start[2] - start[0], start[3] - start[1])
    if max(probe[i] for i in range(0, len(probe), 4 * 97)) <= 4:
        say(f"resize {tag}: no desktop to sample (the window grabs all black); drags skipped")
        return
    for name, step in (("fast", 12), ("slow", 3)):
        place(hwnd, 40, 30, 640, 460)
        start = rect_of(hwnd)
        cx, cy = start[2] - 3, start[3] - 3
        ox, oy = start[2] - 40, start[3] - 40
        w, h = min(380, SCREEN_W - ox), min(380, SCREEN_H - oy)
        grabs = []
        running = [True]

        def sampler():
            t0 = time.perf_counter()
            while running[0] and len(grabs) < 400:
                grabs.append((time.perf_counter() - t0, rect_of(hwnd), cursor(), grab(ox, oy, w, h)))
                time.sleep(0.004)

        mouse_to(cx, cy)
        time.sleep(0.3)
        thread = threading.Thread(target=sampler)
        thread.start()
        send_mouse(MOUSEEVENTF_LEFTDOWN)
        time.sleep(0.05)
        steps = 300 // step
        for k in range(1, steps + 1):
            mouse_to(cx + k * step * 0.8, cy + k * step * 0.8)
            time.sleep(0.016)
        time.sleep(0.05)
        send_mouse(MOUSEEVENTF_LEFTUP)
        time.sleep(0.6)
        running[0] = False
        thread.join()
        final = rect_of(hwnd)
        say(f"resize {tag} {name}: start={start} end={final} grabs={len(grabs)} region={ox},{oy} {w}x{h}")
        for t, window, pos, bgra in grabs:
            b, total = black_share(bgra, (ox, oy), w, h, window, start)
            share = b / total if total else 0.0
            say(f"resize {tag} {name} t={t * 1000:.0f}ms window={window} cursor={pos} new_px={total} black_px={b} black_share={share:.3f}")
        if grabs:
            picks = sorted({len(grabs) // 4, len(grabs) // 2, max(len(grabs) * 3 // 4, 0)})
            for k in picks:
                print_png(f"{tag}-{name}-{k}", grabs[k][3], w, h)
        mouse_to(SCREEN_W // 2, 10)


# ---- control channel ----


class Conn:
    def __init__(self, token):
        self.sock = socket.create_connection(("127.0.0.1", PORT), timeout=70)
        self.file = self.sock.makefile("rwb")
        self.n = 0
        reply = self.call("auth", {"token": token})
        if not reply.get("ok"):
            raise RuntimeError(f"auth: {reply}")

    def call(self, method, params=None):
        self.n += 1
        self.file.write((json.dumps({"id": self.n, "method": method, "params": params or {}}) + "\n").encode())
        self.file.flush()
        while True:
            line = self.file.readline()
            if not line:
                raise RuntimeError("connection closed")
            reply = json.loads(line)
            if reply.get("id") == self.n:
                return reply

    def close(self):
        try:
            self.file.close()
            self.sock.close()
        except OSError:
            pass


class App:
    def __init__(self, exe, label):
        self.label = label
        self.token = secrets.token_hex(32)
        self.log_path = os.path.join(OUT, f"app-{label}.log")
        self.log = open(self.log_path, "wb")
        env = dict(os.environ, PHOTOCRAFT_DIAG="1", PHOTOCRAFT_CONTROL_TOKEN=self.token, RUST_BACKTRACE="1")
        self.proc = subprocess.Popen([exe, "--control", str(PORT), "--new-instance"], stdout=self.log, stderr=subprocess.STDOUT, env=env)
        self.read_at = 0
        self.conn = None
        end = time.time() + 120
        while time.time() < end and self.proc.poll() is None:
            try:
                self.conn = Conn(self.token)
                break
            except (OSError, RuntimeError):
                time.sleep(1)
        say(f"app {label}: pid={self.proc.pid} connected={self.conn is not None} exit={self.proc.poll()}")
        self.hwnd = window_of(self.proc.pid, 30) if self.conn else None

    def new_lines(self):
        try:
            with open(self.log_path, "rb") as f:
                f.seek(self.read_at)
                data = f.read()
                self.read_at += len(data)
        except OSError:
            return
        for line in data.decode("utf-8", "replace").splitlines():
            if "DIAG" in line or "panicked" in line or "ERROR" in line:
                print(f"  app| {line}", flush=True)

    def call(self, method, params=None):
        try:
            reply = self.conn.call(method, params)
        except Exception as e:  # noqa: BLE001
            reply = {"ok": False, "error": f"client: {e}"}
        say(f"> {method} {json.dumps(params or {})[:300]} -> ok={reply.get('ok')} {'' if reply.get('ok') else reply.get('error')}")
        return reply

    def doc(self):
        return self.conn.call("engine.execute", {"command": "document.inspect", "params": {}}).get("result") or {}

    def history(self):
        try:
            return self.doc().get("history") or []
        except Exception:  # noqa: BLE001
            return []

    def settle(self, before, timeout=6.0):
        """Waits until the history grows past `before` entries, at most `timeout` seconds."""
        t0 = time.time()
        while time.time() - t0 < timeout:
            if len(self.history()) > before:
                break
            time.sleep(0.4)
        return time.time() - t0

    def report(self, case):
        try:
            doc = self.doc()
            ui = self.conn.call("ui.inspect", {}).get("result") or {}
        except Exception as e:  # noqa: BLE001
            say(f"report {case}: {e}")
            self.new_lines()
            return
        layers = [{"name": l.get("name"), "text": (l.get("text") or {}).get("text")} for l in doc.get("layers", [])]
        say(f"after {case}: history={json.dumps(doc.get('history'))[:1500]}")
        say(f"after {case}: layers={json.dumps(layers)[:800]}")
        subset = {k: ui.get(k) for k in ("tool", "window", "textEdit", "typeBoxOverflow", "rects", "dialogs", "agentLink")}
        say(f"after {case}: ui={json.dumps(subset)[:2500]}")
        self.new_lines()

    def canvas_center(self):
        ui = self.conn.call("ui.inspect", {}).get("result") or {}
        r = (ui.get("rects") or {}).get("canvas.0")
        if not r:
            return None
        return ((r[0] + r[2]) / 2, (r[1] + r[3]) / 2)

    def quit(self):
        if self.conn:
            try:
                self.conn.call("app.quit", {})
            except Exception:  # noqa: BLE001
                pass
            self.conn.close()
        try:
            self.proc.wait(20)
        except subprocess.TimeoutExpired:
            subprocess.run(["taskkill", "/F", "/T", "/PID", str(self.proc.pid)], capture_output=True)
        self.new_lines()
        say(f"app {self.label}: exit={self.proc.poll()}")


STROKE = [{"kind": "down", "x": 100, "y": 100}, {"kind": "move", "x": 200, "y": 150}, {"kind": "move", "x": 300, "y": 200}, {"kind": "up", "x": 300, "y": 200}]


def case(app, name, body):
    say(f"case {name} ({app.label}) foreground={app.hwnd is not None and user32.GetForegroundWindow() == app.hwnd}")
    try:
        body()
    except Exception as e:  # noqa: BLE001
        say(f"case {name}: script error {e!r}")
    app.report(name)


def pointer_stroke(app, tool="brush"):
    n = len(app.history())
    app.call("ui.pointer", {"tool": tool, "events": STROKE})
    say(f"history grew after {app.settle(n):.1f}s: {len(app.history())} > {n}")


def closed_connection_stroke(app):
    n = len(app.history())
    c = Conn(app.token)
    reply = c.call("ui.pointer", {"tool": "brush", "events": [{"kind": "down", "x": 120, "y": 300}, {"kind": "move", "x": 250, "y": 320}, {"kind": "up", "x": 400, "y": 340}]})
    c.close()
    say(f"> ui.pointer on a connection closed after the reply -> {reply}")
    say(f"history grew after {app.settle(n):.1f}s: {len(app.history())} > {n}")


def free_transform(app):
    app.call("engine.execute", {"command": "layer.new.layer", "params": {"name": "FT"}})
    app.call("engine.execute", {"command": "select.rect", "params": {"x": 200, "y": 100, "width": 200, "height": 150}})
    app.call("engine.execute", {"command": "edit.fill", "params": {"contents": "color", "color": "#ff0000"}})
    app.call("engine.execute", {"command": "select.deselect", "params": {}})
    app.call("ui.set", {"tool": "move", "moveShowTransform": True})
    time.sleep(0.5)
    n = len(app.history())
    app.call("ui.pointer", {"events": [{"kind": "down", "x": 400, "y": 250}, {"kind": "move", "x": 450, "y": 290}, {"kind": "up", "x": 470, "y": 310}]})
    time.sleep(1.5)
    app.call("ui.key", {"key": "Enter"})
    say(f"history grew after {app.settle(n):.1f}s: {len(app.history())} > {n}")
    app.call("ui.set", {"moveShowTransform": False})


def control_click_stroke(app):
    app.call("ui.set", {"tool": "brush"})
    c = app.canvas_center()
    if not c:
        say("no canvas rect")
        return
    x, y = c
    n = len(app.history())
    app.call("ui.click", {"x": x - 120, "y": y + 60, "hold": True})
    app.call("ui.move", {"x": x - 20, "y": y + 90})
    app.call("ui.move", {"x": x + 100, "y": y + 110})
    app.call("ui.release", {})
    say(f"history grew after {app.settle(n):.1f}s: {len(app.history())} > {n}")


def type_reads(app):
    t0 = time.time()
    for at in (0.3, 1.0, 2.0):
        time.sleep(max(0.0, at - (time.time() - t0)))
        ui = app.conn.call("ui.inspect", {}).get("result") or {}
        layers = [(l.get("name"), (l.get("text") or {}).get("text")) for l in app.doc().get("layers", [])]
        say(f"type read +{at}s: textEdit={json.dumps(ui.get('textEdit'))[:600]} layers={json.dumps(layers)[:400]}")
        app.new_lines()


def type_click(app):
    app.call("ui.set", {"tool": "type"})
    time.sleep(0.3)
    c = app.canvas_center()
    if not c:
        say("no canvas rect")
        return
    app.call("ui.click", {"x": c[0] - 60, "y": c[1] - 40})
    type_reads(app)
    app.call("ui.key", {"key": "Escape"})


def type_pointer(app):
    app.call("ui.pointer", {"tool": "type", "events": [{"kind": "down", "x": 150, "y": 330}, {"kind": "up", "x": 150, "y": 330}]})
    type_reads(app)
    app.call("ui.key", {"key": "Escape"})


def round_(exe, label, scale):
    app = App(exe, label)
    try:
        if not app.conn:
            app.new_lines()
            return
        if scale:
            # Set and saved; the next round starts at this scale from its first frame.
            app.call("engine.execute", {"command": "prefs.set", "params": {"path": "interface.uiScale", "value": scale}})
            time.sleep(1.0)
            return
        if app.hwnd:
            place(app.hwnd, 40, 30, 940, 700)
        else:
            say("no window found for the app")
        ui = app.conn.call("ui.inspect", {}).get("result") or {}
        say(f"{label}: window={ui.get('window')}")
        case(app, "a file.new", lambda: app.call("engine.execute", {"command": "file.new", "params": {"width": 600, "height": 400, "background": "#ffffff"}}))
        time.sleep(1.0)
        case(app, "b ui.pointer brush stroke", lambda: pointer_stroke(app))
        case(app, "c ui.pointer stroke, connection closed after the reply", lambda: closed_connection_stroke(app))

        def move_drag():
            app.call("engine.execute", {"command": "layer.new.layer", "params": {"name": "L"}})
            pointer_stroke(app)
            pointer_stroke(app, "move")

        case(app, "d ui.pointer move drag", move_drag)
        case(app, "e free transform handle drag", lambda: free_transform(app))
        case(app, "f ui.click/ui.move hold brush stroke", lambda: control_click_stroke(app))
        case(app, "g type ui.click", lambda: type_click(app))
        case(app, "h type ui.pointer", lambda: type_pointer(app))
        if app.hwnd:
            focus(app.hwnd)
            case(app, "b2 ui.pointer brush stroke, window focused", lambda: pointer_stroke(app))
            case(app, "g2 type ui.click, window focused", lambda: type_click(app))
            if not scale:
                corner_drags(app.hwnd, "main")
                app.new_lines()
    finally:
        app.quit()


def release_drags():
    zip_path = os.path.join(OUT, "pc-0.23.1.zip")
    t0 = time.time()
    urllib.request.urlretrieve(RELEASE_ZIP, zip_path)
    root = os.path.join(OUT, "pc-0.23.1")
    with zipfile.ZipFile(zip_path) as z:
        z.extractall(root)
    exe = next((os.path.join(d, f) for d, _, fs in os.walk(root) for f in fs if f.lower() == "photocraft.exe"), None)
    say(f"v0.23.1: downloaded and unpacked in {time.time() - t0:.0f}s, exe={exe}")
    if not exe:
        return
    proc = subprocess.Popen([exe], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        hwnd = window_of(proc.pid, 60)
        say(f"v0.23.1: pid={proc.pid} window={hwnd is not None}")
        if hwnd:
            time.sleep(3)
            focus(hwnd)
            corner_drags(hwnd, "v0.23.1")
    finally:
        subprocess.run(["taskkill", "/F", "/T", "/PID", str(proc.pid)], capture_output=True)


def main():
    exe = os.path.abspath(sys.argv[1])
    say(f"screen {SCREEN_W}x{SCREEN_H}, exe {exe} exists={os.path.exists(exe)}")
    for label, scale, body in (
        ("100", None, lambda: round_(exe, "100", None)),
        ("125-set", "125", lambda: round_(exe, "125-set", "125")),
        ("125", None, lambda: round_(exe, "125", None)),
        ("v0.23.1", None, release_drags),
    ):
        say(f"==== round {label} ====")
        try:
            body()
        except Exception as e:  # noqa: BLE001
            say(f"round {label}: script error {e!r}")
    say("done")


if __name__ == "__main__":
    main()
