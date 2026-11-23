"""postman-echo-like local test server for postmini examples (HTTP 8765, optional HTTPS 8766)."""
import json, gzip, zlib, hashlib, ssl, sys, threading, time, base64, re, os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs, parse_qsl
NONCE = "abc123nonce"
class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, *a): pass
    def body(self):
        n = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n) if n else b""
    def send_json(self, obj, code=200, extra=None, enc=None):
        data = json.dumps(obj, indent=2).encode()
        if enc == "gzip": data = gzip.compress(data)
        if enc == "deflate": data = zlib.compress(data)
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        if enc: self.send_header("Content-Encoding", enc)
        for k, v in (extra or []): self.send_header(k, v)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers(); self.wfile.write(data)
    def echo(self):
        u = urlparse(self.path); raw = self.body(); ct = self.headers.get("Content-Type", "")
        out = {"args": dict(parse_qsl(u.query)), "headers": {k.lower(): v for k, v in self.headers.items()}, "url": "http://%s%s" % (self.headers.get("Host"), self.path), "method": self.command}
        out["data"] = raw.decode("utf-8", "replace"); out["json"] = None; out["form"] = {}; out["files"] = {}
        if "json" in ct:
            try: out["json"] = json.loads(raw)
            except Exception: pass
        if "x-www-form-urlencoded" in ct: out["form"] = dict(parse_qsl(raw.decode()))
        if "multipart/form-data" in ct:
            b = ct.split("boundary=")[1].encode()
            for part in raw.split(b"--" + b)[1:-1]:
                head, _, val = part.partition(b"\r\n\r\n"); val = val[:-2]
                name = re.search(rb'name="([^"]*)"', head).group(1).decode()
                fn = re.search(rb'filename="([^"]*)"', head)
                if fn: out["files"][fn.group(1).decode()] = "data:application/octet-stream;base64," + base64.b64encode(val).decode()
                else: out["form"][name] = val.decode()
        self.send_json(out)
    def route(self):
        p = urlparse(self.path); path = p.path; q = dict(parse_qsl(p.query))
        if path in ("/get", "/post", "/put", "/patch", "/delete", "/anything") or path.startswith("/anything"): return self.echo()
        m = re.match(r"/status/(\d+)", path)
        if m:
            c = int(m.group(1)); self.send_response(c); self.send_header("Content-Length", "0"); self.end_headers(); return
        m = re.match(r"/redirect/(\d+)", path)
        if m:
            n = int(m.group(1)); self.body()
            self.send_response(302); self.send_header("Location", "/redirect/%d" % (n-1) if n > 1 else "/get"); self.send_header("Content-Length", "0"); self.end_headers(); return
        if path == "/cookies/set":
            self.send_response(302)
            for k, v in q.items(): self.send_header("Set-Cookie", "%s=%s; Path=/" % (k, v))
            self.send_header("Location", "/cookies"); self.send_header("Content-Length", "0"); self.end_headers(); return
        if path == "/cookies":
            c = {}
            for part in (self.headers.get("Cookie") or "").split(";"):
                if "=" in part: k, v = part.strip().split("=", 1); c[k] = v
            return self.send_json({"cookies": c})
        if path == "/gzip": return self.send_json({"gzipped": True, "headers": dict(self.headers)}, enc="gzip")
        if path == "/deflate": return self.send_json({"deflated": True}, enc="deflate")
        m = re.match(r"/stream/(\d+)", path)
        if m:
            self.send_response(200); self.send_header("Content-Type", "application/json"); self.send_header("Transfer-Encoding", "chunked"); self.end_headers()
            for i in range(int(m.group(1))):
                d = (json.dumps({"id": i}) + "\n").encode(); self.wfile.write(b"%x\r\n%s\r\n" % (len(d), d))
            self.wfile.write(b"0\r\n\r\n"); return
        m = re.match(r"/basic-auth/([^/]+)/([^/]+)", path)
        if m:
            exp = "Basic " + base64.b64encode(("%s:%s" % m.groups()).encode()).decode()
            if self.headers.get("Authorization") == exp: return self.send_json({"authenticated": True})
            return self.send_json({"authenticated": False}, 401)
        m = re.match(r"/digest-auth/auth/([^/]+)/([^/]+)", path)
        if m:
            user, pw = m.groups(); a = self.headers.get("Authorization", "")
            if a.startswith("Digest "):
                f = dict(re.findall(r'(\w+)="?([^",]+)"?', a))
                ha1 = hashlib.md5(("%s:%s:%s" % (user, "me@example.com", pw)).encode()).hexdigest()
                ha2 = hashlib.md5(("%s:%s" % (self.command, f.get("uri"))).encode()).hexdigest()
                exp = hashlib.md5(("%s:%s:%s:%s:%s:%s" % (ha1, NONCE, f.get("nc"), f.get("cnonce"), f.get("qop"), ha2)).encode()).hexdigest()
                if f.get("response") == exp: return self.send_json({"authenticated": True, "user": user})
            return self.send_json({}, 401, [("WWW-Authenticate", 'Digest realm="me@example.com", nonce="%s", qop="auth", opaque="xyz", algorithm=MD5' % NONCE)])
        m = re.match(r"/delay/(\d+)", path)
        if m: time.sleep(int(m.group(1))); return self.send_json({"delay": m.group(1)})
        if path == "/xml":
            d = b'<?xml version="1.0"?><root a="1"><item id="1">one</item><item id="2">two</item><name>x &amp; y</name></root>'
            self.send_response(200); self.send_header("Content-Type", "application/xml"); self.send_header("Content-Length", str(len(d))); self.end_headers(); self.wfile.write(d); return
        if path == "/headers": return self.send_json({"headers": dict(self.headers)})
        if path == "/close":
            d = b"until-close body"; self.send_response(200); self.send_header("Connection", "close"); self.end_headers(); self.wfile.write(d); self.close_connection = True; return
        self.send_json({"error": "not found", "path": path}, 404)
    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = do_HEAD = do_OPTIONS = route
def serve(port, tls):
    s = ThreadingHTTPServer(("127.0.0.1", port), H)
    if tls:
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); ctx.load_cert_chain(os.path.join(os.path.dirname(__file__), "cert.pem"), os.path.join(os.path.dirname(__file__), "key.pem"))
        s.socket = ctx.wrap_socket(s.socket, server_side=True)
    s.serve_forever()
if __name__ == "__main__":
    here = os.path.dirname(os.path.abspath(__file__))
    if os.path.exists(os.path.join(here, "cert.pem")) and os.path.exists(os.path.join(here, "key.pem")):
        threading.Thread(target=serve, args=(8766, True), daemon=True).start()
        print("HTTPS on https://localhost:8766")
    else:
        print("No cert.pem/key.pem next to this script - HTTPS test endpoint disabled.")
        print("  (create with: openssl req -x509 -newkey rsa:2048 -nodes -keyout key.pem -out cert.pem -days 365 -subj /CN=localhost)")
    print("HTTP on http://127.0.0.1:8765  (Ctrl+C to stop)")
    serve(8765, False)
