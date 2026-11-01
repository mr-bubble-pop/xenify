#!/usr/bin/env python3
"""Checks the built site (target/site) the way people will meet it, before it
goes to xenify.net: it's served as Cloudflare Pages serves it (an address
ending in index.html or .html is sent on to the one without, with a 308),
and a headless Chromium opens each app with an offline copy (Write, Draw)
three times, then once more with no connection, plus each other app once.

Why: an offline copy that kept an answer that came through such a redirect
made Chrome show "This site can't be reached" from the second visit on (the
first, for Draw), and only on the real host. Run after scripts/build-site.sh:

    scripts/check-site.py [site folder]   (exits 1, saying what failed)

Needs chromium. Uses a fresh browser profile each run (in a temporary folder).
"""
import http.server, json, os, shutil, socket, subprocess, sys, tempfile, threading, time, urllib.parse

# The site to check: target/site, or the folder given.
ROOT = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "target", "site"))
OFFLINE_APPS = ["write", "draw"]
OTHER_APPS = ["calc", "plan"]


class CloudflareLike(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **k):
        super().__init__(*a, directory=ROOT, **k)

    def redirect_target(self):
        u = urllib.parse.urlsplit(self.path)
        if u.path.endswith("/index.html"):
            to = u.path[: -len("index.html")]
        elif u.path.endswith(".html") and u.path != "/404.html":
            to = u.path[: -len(".html")]
        else:
            return None
        return to + (("?" + u.query) if u.query else "")

    def do_GET(self):
        to = self.redirect_target()
        if to:
            self.send_response(308)
            self.send_header("Location", to)
            self.end_headers()
            return
        # And serves x.html at x (Draw's present.html at /draw/present).
        u = urllib.parse.urlsplit(self.path)
        if not os.path.exists(self.translate_path(u.path)) and os.path.isfile(self.translate_path(u.path + ".html")):
            self.path = u.path + ".html" + (("?" + u.query) if u.query else "")
        return super().do_GET()

    def end_headers(self):
        # Like Pages: revalidated by content, never "not modified" by date alone.
        self.send_header("Cache-Control", "no-cache")
        super().end_headers()

    def log_message(self, *a):
        pass


class Browser:
    """Headless Chromium over --remote-debugging-pipe (standard library only)."""

    def __init__(self, profile):
        r1, w1 = os.pipe()
        r2, w2 = os.pipe()

        def child():
            os.dup2(r1, 3)
            os.dup2(w2, 4)

        env = dict(os.environ)
        env.pop("DISPLAY", None)
        exe = shutil.which("chromium") or shutil.which("chromium-browser") or shutil.which("google-chrome")
        if not exe:
            sys.exit("check-site: needs chromium")
        self.proc = subprocess.Popen(
            [exe, "--headless=new", "--no-sandbox", "--remote-debugging-pipe", "--no-first-run",
             "--use-angle=swiftshader", "--enable-unsafe-swiftshader", f"--user-data-dir={profile}", "about:blank"],
            env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            preexec_fn=child, pass_fds=(3, 4), close_fds=False)
        os.close(r1)
        os.close(w2)
        self.w, self.r, self.buf, self.n = w1, r2, b"", 0

    def send(self, method, params=None, session=None):
        self.n += 1
        msg = {"id": self.n, "method": method, "params": params or {}}
        if session:
            msg["sessionId"] = session
        os.write(self.w, json.dumps(msg).encode() + b"\0")
        while True:
            while b"\0" not in self.buf:
                chunk = os.read(self.r, 1 << 20)
                if not chunk:
                    raise RuntimeError("chromium closed")
                self.buf += chunk
            raw, self.buf = self.buf.split(b"\0", 1)
            m = json.loads(raw)
            if m.get("id") == self.n:
                if "error" in m:
                    raise RuntimeError(f"{method}: {m['error']}")
                return m.get("result", {})

    def page(self):
        t = self.send("Target.createTarget", {"url": "about:blank"})["targetId"]
        s = self.send("Target.attachToTarget", {"targetId": t, "flatten": True})["sessionId"]
        self.send("Page.enable", session=s)
        self.send("Runtime.enable", session=s)
        return s

    def where(self, s):
        r = self.send("Runtime.evaluate", {"expression": "location.href + '\\n' + document.title", "returnByValue": True}, s)
        return r.get("result", {}).get("value", "")

    def close(self):
        try:
            self.send("Browser.close")
        except Exception:
            pass
        self.proc.wait(timeout=10)


def main():
    if not os.path.isfile(os.path.join(ROOT, "write", "index.html")):
        sys.exit("check-site: build the site first (scripts/build-site.sh)")
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), CloudflareLike)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{port}"
    profile = tempfile.mkdtemp(prefix="check-site-")
    failed = []
    b = Browser(profile)
    try:
        page = b.page()

        def visit(app, label, wait=10):
            b.send("Page.navigate", {"url": f"{base}/{app}/"}, page)
            time.sleep(wait)
            href, _, title = b.where(page).partition("\n")
            ok = href.startswith(f"{base}/{app}/") and "Xenify" in title
            print(f"  {app:6} {label:16} {'ok' if ok else 'FAILED'}  ({title or href})")
            if not ok:
                failed.append(f"{app} {label}")

        for app in OFFLINE_APPS:
            for n in ("first visit", "second visit", "third visit"):
                visit(app, n, 14 if n == "first visit" else 8)
            # Offline once its copy is kept (the first visit downloads the
            # engine and the checker, tens of MB: up to a minute here).
            ready = "(async () => { const r = await navigator.serviceWorker.getRegistration(); return !!(r && r.active && !r.installing && !r.waiting && navigator.serviceWorker.controller); })()"
            for _ in range(60):
                r = b.send("Runtime.evaluate", {"expression": ready, "awaitPromise": True, "returnByValue": True}, page)
                if r.get("result", {}).get("value"):
                    break
                time.sleep(1)
            b.send("Network.enable", session=page)
            b.send("Network.emulateNetworkConditions", {"offline": True, "latency": 0, "downloadThroughput": -1, "uploadThroughput": -1}, page)
            visit(app, "offline")
            b.send("Network.emulateNetworkConditions", {"offline": False, "latency": 0, "downloadThroughput": -1, "uploadThroughput": -1}, page)
        for app in OTHER_APPS:
            visit(app, "visit")
    finally:
        b.close()
        server.shutdown()
        shutil.rmtree(profile, ignore_errors=True)
    if failed:
        print("check-site: FAILED: " + ", ".join(failed))
        sys.exit(1)
    print("check-site: every app opens, again and again, and offline")


if __name__ == "__main__":
    main()
