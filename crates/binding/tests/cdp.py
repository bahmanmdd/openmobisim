"""A headless Chrome or Chromium driven over its DevTools protocol on a pipe (fds 3 and 4,
`--remote-debugging-pipe`), with the standard library alone: open a page at a viewport size, run
script in it, and press and release the mouse at a point (real pointer events). For the pages'
tests; no third-party package."""

import json
import os
import subprocess
import time


class Page:
    """One page in a fresh browser; `close()` ends the browser."""

    def __init__(self, binary, profile, url, width=1500, height=950):
        to_r, self._to = os.pipe()
        self._from, from_w = os.pipe()

        def child():
            os.dup2(to_r, 3)
            os.dup2(from_w, 4)

        self._browser = subprocess.Popen(
            [binary, "--headless=new", "--disable-gpu", "--no-sandbox", "--remote-debugging-pipe",
             f"--user-data-dir={profile}", "about:blank"],
            preexec_fn=child, pass_fds=(3, 4), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        os.close(to_r)
        os.close(from_w)
        self._buffer, self._next, self._session = b"", 0, None
        target = self.call("Target.createTarget", {"url": "about:blank"})["targetId"]
        self._session = self.call("Target.attachToTarget", {"targetId": target, "flatten": True})["sessionId"]
        self.call("Emulation.setDeviceMetricsOverride",
                  {"width": width, "height": height, "deviceScaleFactor": 1, "mobile": False})
        self.call("Page.navigate", {"url": url})

    def call(self, method, params=None):
        """Send one command and return its result; an error is raised."""
        self._next += 1
        message = {"id": self._next, "method": method, "params": params or {}}
        if self._session and not method.startswith(("Target.", "Browser.")):
            message["sessionId"] = self._session
        os.write(self._to, json.dumps(message).encode() + b"\0")
        while True:
            while b"\0" in self._buffer:
                raw, self._buffer = self._buffer.split(b"\0", 1)
                reply = json.loads(raw)
                if reply.get("id") == self._next:
                    if "error" in reply:
                        raise RuntimeError(f"{method}: {reply['error']}")
                    return reply["result"]
            chunk = os.read(self._from, 1 << 16)
            if not chunk:
                raise RuntimeError("the browser closed its pipe")
            self._buffer += chunk

    def run(self, expression):
        """The value of a JavaScript expression in the page."""
        result = self.call("Runtime.evaluate", {"expression": expression, "returnByValue": True})
        if "exceptionDetails" in result:
            raise RuntimeError(result["exceptionDetails"])
        return result["result"].get("value")

    def wait(self, expression, seconds=30):
        """Wait until the expression is true in the page."""
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if self.run(expression):
                return
            time.sleep(0.05)
        raise TimeoutError(expression)

    def click(self, x, y):
        """Press and release the left button at (x, y), CSS px from the viewport's corner."""
        for kind in ("mouseMoved", "mousePressed", "mouseReleased"):
            self.call("Input.dispatchMouseEvent",
                      {"type": kind, "x": x, "y": y, "button": "left", "buttons": 1 if kind == "mousePressed" else 0,
                       "clickCount": 1})
        time.sleep(0.1)

    def move(self, x, y):
        """Move the mouse to (x, y): a hover."""
        self.call("Input.dispatchMouseEvent", {"type": "mouseMoved", "x": x, "y": y, "buttons": 0})
        time.sleep(0.1)

    def key(self, name):
        """Press and release a key by its DOM name (`Escape`)."""
        for kind in ("keyDown", "keyUp"):
            self.call("Input.dispatchKeyEvent",
                      {"type": kind, "key": name, "code": name, "windowsVirtualKeyCode": {"Escape": 27}.get(name, 0)})
        time.sleep(0.1)

    def close(self):
        try:
            self.call("Browser.close")
        finally:
            self._browser.wait(timeout=30)
