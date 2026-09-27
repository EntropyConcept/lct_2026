#!/usr/bin/env python3
"""Exercise a running dispatcher through its published Docker port (stdlib only)."""
import json
import sys
from urllib.error import HTTPError
from urllib.request import Request, urlopen

base = sys.argv[1].rstrip("/")


def request(path, *, body=None, headers=None, status=200):
    req = Request(
        base + path,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Content-Type": "application/json", **(headers or {})},
    )
    try:
        response = urlopen(req, timeout=30)
    except HTTPError as error:
        response = error
    with response:
        assert response.status == status, (path, response.status, status)
        return response.read()


assert b"<html" in request("/").lower()
assert request("/vendor/leaflet.js")
assert request("/vendor/leaflet.css")
assert len(json.loads(request("/api/norms"))) == 4
assert len(json.loads(request("/api/datasets"))) == 3
assert json.loads(request("/api/dataset/0"))["jobs"]
scenario = json.loads(request("/api/demo"))
plan = json.loads(request("/api/plan", body={"scenario": scenario, "seconds": 0.1},
                          headers={"Origin": base}))
assert plan["plan"]["routes"]
request("/api/demo", headers={"Origin": "https://untrusted.example"}, status=403)
request("/api/demo", headers={"Host": "untrusted.example"}, status=403)
print("Container smoke checks passed: UI, datasets, norms, solve, Host/Origin restrictions")
