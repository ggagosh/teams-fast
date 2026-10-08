#!/usr/bin/env python3
"""Check a deployed relay with synthetic metadata only; never sends Teams messages."""
import concurrent.futures
import getpass
import json
import secrets
import sys
import time
import urllib.error
import urllib.request

base = sys.argv[1].rstrip("/")
key = getpass.getpass("Relay access key: ")


def request(method, path, data=None, authenticated=True):
    headers = {"Content-Type": "application/json", "User-Agent": "TeamsFast/0.1"}
    if authenticated:
        headers["Authorization"] = f"Bearer {key}"
    req = urllib.request.Request(
        base + path, method=method, headers=headers,
        data=None if data is None else json.dumps(data).encode(),
    )
    try:
        with urllib.request.urlopen(req, timeout=35) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as error:
        return error.code, error.read()


health = request("GET", "/health", authenticated=False)
assert health == (200, b"ok"), f"Health check returned HTTP {health[0]}"
identity = secrets.token_hex(32)
path = f"/v1/clients/{identity}"
assert request("POST", "/v1/clients", {"installation_id": identity}, False)[0] == 401
status, body = request("POST", "/v1/clients", {"installation_id": identity})
assert status == 200
registration = json.loads(body)
try:
    assert registration["notification_url"] == base + f"/graph/{identity}"
    assert request("POST", f"/graph/{identity}?validationToken=check%20echo", authenticated=False) == (200, b"check echo")
    notice = {"value": [{"clientState": "invalid", "changeType": "created", "resource": "chats/synthetic/messages/check"}]}
    request("POST", f"/graph/{identity}", notice, False)
    with concurrent.futures.ThreadPoolExecutor() as pool:
        waiting = pool.submit(request, "GET", path + "/events?after=0")
        time.sleep(0.3)
        assert not waiting.done(), "Invalid webhook must not emit an event"
        notice["value"][0]["clientState"] = registration["client_state"]
        started = time.monotonic()
        assert request("POST", f"/graph/{identity}", notice, False)[0] == 202
        status, body = waiting.result()
        latency = time.monotonic() - started
        batch = json.loads(body)
        assert status == 200 and batch["cursor"] == 1 and not batch["reset"]
        assert batch["changes"] == [{"chat_id": "synthetic", "message_id": "check", "kind": "created"}]
        assert latency < 5, "Webhook should immediately wake the waiting client"
        print(f"PASS: HTTPS health, authentication, validation, rejected invalid state, push delivery ({latency:.2f}s)")
finally:
    assert request("DELETE", path)[0] == 204
