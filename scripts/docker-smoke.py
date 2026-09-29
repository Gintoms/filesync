#!/usr/bin/env python3
"""Verify a built hub image using an isolated container and disposable volume."""
import hashlib
import json
import os
import re
import secrets
import subprocess
import sys
import time
import urllib.error
import urllib.request


def docker(*args):
    return subprocess.check_output(["docker", *args], text=True).strip()


image = sys.argv[1] if len(sys.argv) > 1 else "filesync:0.1.0"
name = "filesync-smoke-" + secrets.token_hex(5)
volume = name + "-data"
token = secrets.token_hex(32)
env = dict(os.environ, FILESYNC_ADMIN_TOKEN=token)
url = ""


def request(path, method="GET", body=None, auth=True, credential=None):
    headers = {"Authorization": "Bearer " + (credential or token)} if auth else {}
    if body is not None:
        headers["Content-Type"] = "application/octet-stream" if isinstance(body, bytes) else "application/json"
    req = urllib.request.Request(url + path, method=method, headers=headers,
                                 data=body if isinstance(body, bytes) else None if body is None else json.dumps(body).encode())
    with urllib.request.urlopen(req, timeout=5) as res:
        return res.status, res.read()


def start():
    global url
    subprocess.check_output([
        "docker", "run", "-d", "--name", name,
        "-p", "127.0.0.1::8787", "-e", "FILESYNC_ADMIN_TOKEN",
        "-v", volume + ":/data", image,
    ], env=env, text=True)
    port = docker("port", name, "8787/tcp").rsplit(":", 1)[1]
    url = "http://127.0.0.1:" + port
    for _ in range(100):
        try:
            if request("/health")[0] == 200:
                return
        except (OSError, urllib.error.URLError):
            pass
        time.sleep(0.1)
    raise RuntimeError("container did not become healthy")


try:
    docker("volume", "create", volume)
    start()
    assert docker("exec", name, "id", "-u") == "10001"
    html = request("/")[1].decode()
    assert 'id="app"' in html
    assets = re.findall(r'(?:src|href)="(/assets/[^\"]+)"', html)
    assert assets
    for path in assets:
        assert request(path)[0] == 200
    try:
        request("/v1/admin/nodes", auth=False)
        raise AssertionError("anonymous administration allowed")
    except urllib.error.HTTPError as error:
        assert error.code == 401
    node = json.loads(request("/v1/nodes", "POST", {"name": "Container persistence test"})[1])
    receiver = json.loads(request("/v1/nodes", "POST", {"name": "Container receiver"})[1])
    request("/v1/edges", "POST", {"upstream": node["id"], "downstream": receiver["id"]})
    hashes = []
    for revision in range(3):
        content = ("FileSync container revision " + str(revision) + "\n").encode()
        sha = hashlib.sha256(content).hexdigest()
        hashes.append(sha)
        request("/v1/blobs/" + sha, "PUT", content, credential=node["token"])
        manifest = (json.dumps({"type": "file", "data": {"path": "test.txt", "sha256": sha,
                     "size": len(content)}}) + '\n{"type":"complete"}\n').encode()
        request("/v1/manifest", "PUT", manifest, credential=node["token"])
    # Repeating the same manifest must preserve the previous content too.
    request("/v1/manifest", "PUT", manifest, credential=node["token"])
    assert subprocess.run(["docker", "exec", name, "test", "-e", "/data/blobs/" + hashes[0]]).returncode == 1
    for retained in hashes[1:]:
        docker("exec", name, "test", "-f", "/data/blobs/" + retained)
    docker("stop", "--time", "30", name)
    state = json.loads(docker("inspect", "--format", "{{json .State}}", name))
    assert state["ExitCode"] == 0 and not state["OOMKilled"]
    docker("rm", name)
    start()
    nodes = json.loads(request("/v1/admin/nodes")[1])
    assert any(item["id"] == node["id"] for item in nodes)
    for retained in hashes[1:]:
        docker("exec", name, "test", "-f", "/data/blobs/" + retained)
    assert request("/v1/content/" + node["id"] + "/" + sha, credential=receiver["token"])[1] == content
    assert request("/v1/admin/nodes/" + node["id"], "DELETE")[0] == 204
    print(json.dumps({"image": image, "http": "passed", "web_assets": len(assets),
                      "authentication": "passed", "non_root": "passed",
                      "volume_persistence": "passed", "graceful_stop": "passed",
                      "file_upload_download_after_recreate": "passed",
                      "two_version_retention": "passed",
                      "node_deletion": "passed"}))
finally:
    subprocess.run(["docker", "rm", "-f", name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    subprocess.run(["docker", "volume", "rm", volume], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
