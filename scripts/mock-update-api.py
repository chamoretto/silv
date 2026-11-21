#!/usr/bin/env python3
"""Mock of the github releases api, for manually testing the silv self-update flow.

Answers the two routes the updater uses:
  GET /repos/{owner}/{repo}/releases  -> a github-shaped release listing (json)
  GET /files/{asset}                  -> the asset bytes (the "new binary")

Point silv at it with --repo-url:

  python scripts/mock-update-api.py --port 8080
  silv update --check --repo-url http://127.0.0.1:8080/testowner/testrepo

The asset defaults to target/debug/silv[.exe] so that a manual self-update lands
on a runnable binary. See docs/manual-update-test.md for the full walkthrough.
"""

import argparse
import json
import platform
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

REPO_OWNER = "testowner"
REPO_NAME = "testrepo"

# Release asset names are silv-<tag>-<target>[.exe], see .github/workflows/release.yml
TRIPLES = {
    ("win32", "AMD64"): "x86_64-pc-windows-msvc",
    ("linux", "x86_64"): "x86_64-unknown-linux-musl",
    ("linux", "aarch64"): "aarch64-unknown-linux-musl",
    ("darwin", "arm64"): "aarch64-apple-darwin",
    ("darwin", "x86_64"): "x86_64-apple-darwin",
}


def detect_triple():
    key = (sys.platform, platform.machine())
    if key in TRIPLES:
        return TRIPLES[key]
    print(f"unknown platform {key}, pass --triple explicitly", file=sys.stderr)
    raise SystemExit(2)


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        print(f"{self.address_string()} {fmt % args}")

    def send(self, status, body, content_type):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        path = self.path.split("?", 1)[0]
        server = self.server
        releases = f"/repos/{REPO_OWNER}/{REPO_NAME}/releases"
        if path == releases:
            self.send(200, json.dumps(server.listing).encode(), "application/json")
        elif path.startswith("/files/"):
            name = path.removeprefix("/files/")
            if name in server.assets:
                self.send(200, server.assets[name], "application/octet-stream")
            else:
                self.send(404, b'{"message":"Not Found"}', "application/json")
        else:
            print(f"  note: silv asked an unexpected route {path!r}")
            self.send(404, b'{"message":"Not Found"}', "application/json")


def build_listing(port, versions, body, triple, exe_suffix):
    listing = []
    assets = {}
    for index, version in enumerate(versions):
        asset = f"silv-v{version}-{triple}{exe_suffix}"
        assets[asset] = body
        listing.append(
            {
                "tag_name": f"v{version}",
                "name": f"v{version}",
                # github publishes newest first; the updater walks that order
                "body": f"## v{version}\n\n{body_note(body, version)}",
                "created_at": f"2026-11-{20 - index:02d}T10:00:00Z",
                "html_url": f"http://127.0.0.1:{port}/releases/tag/v{version}",
                "assets": [
                    {"id": index + 1, "name": asset, "url": f"http://127.0.0.1:{port}/files/{asset}"}
                ],
            }
        )
    return listing, assets


def body_note(payload, version):
    if version.endswith(".1"):
        return "- mock release: the changelog button shows every release newer than you"
    return "- mock release: click the update button on the bottom line to install it"


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--port", type=int, default=8080)
    parser.add_argument("--version", default="9.9.9", help="newest mocked release version")
    parser.add_argument("--extra-version", default="0.4.0", help="an older mocked release, for the changelog")
    parser.add_argument("--asset", help="file served as the new binary (default: target/debug/silv)")
    parser.add_argument("--triple", default=None, help="target triple of the asset name")
    parser.add_argument("--host", default="127.0.0.1")
    args = parser.parse_args()

    exe_suffix = ".exe" if sys.platform == "win32" else ""
    triple = args.triple or detect_triple()
    asset_path = args.asset or f"target/debug/silv{exe_suffix}"
    try:
        with open(asset_path, "rb") as handle:
            payload = handle.read()
    except OSError as error:
        print(f"cannot read the asset {asset_path!r}: {error}", file=sys.stderr)
        raise SystemExit(2)

    versions = sorted({args.version, args.extra_version}, key=lambda v: [int(p) for p in v.split(".")], reverse=True)
    listing, assets = build_listing(args.port, versions, payload, triple, exe_suffix)

    server = ThreadingHTTPServer((args.host, args.port), Handler)
    server.listing = listing
    server.assets = assets

    print(f"mock github api on http://{args.host}:{args.port}")
    print(f"  repo url for silv: http://{args.host}:{args.port}/{REPO_OWNER}/{REPO_NAME}")
    print(f"  releases: {', '.join('v' + v for v in versions)}, asset from {asset_path} ({len(payload)} bytes)")
    print("  Ctrl-C stops it")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
