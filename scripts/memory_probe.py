#!/usr/bin/env python3
"""Measure actual agent --once processes over loopback HTTPS (macOS/Linux).

Uses fresh generated fixtures only. It never reads or publishes user files.
This is a sequential-workload baseline, NOT the complete daemon memory gate.
"""
import argparse
import hashlib
import json
import os
import platform
from pathlib import Path
import secrets
import socket
import ssl
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/filesync"))
    parser.add_argument("--files", type=int, default=20000)
    parser.add_argument("--large-bytes", type=int, default=99_000_000)
    parser.add_argument("--output", type=Path, default=Path(".local/memory-report.json"))
    args = parser.parse_args()
    if not hasattr(os, "wait4"):
        parser.error("This baseline uses wait4 peak RSS; Windows measurement is not implemented.")
    if args.files < 1 or not 0 < args.large_bytes < 100_000_000:
        parser.error("files >= 1 and 0 < large-bytes < 100000000 required")
    binary = args.binary.resolve(strict=True)
    admin_token = secrets.token_hex(32)
    env = dict(os.environ, FILESYNC_PROBE_ADMIN=admin_token)
    report = {
        "platform": platform.platform(),
        "machine": platform.machine(),
        "binary_sha256": hashlib.file_digest(binary.open("rb"), "sha256").hexdigest(),
        "workload": {"small_files": args.files, "large_file_bytes": args.large_bytes,
                     "small_file_bytes": 64, "transport": "HTTPS loopback with certificate verification",
                     "transfer_concurrency": 1, "claimed_files": 1,
                     "unclaimed_small_files": args.files},
        "limit_bytes": 20_000_000,
        "complete_memory_acceptance": False,
        "limitations": ["agent --once, not long-lived daemon", "sequential scan and transfer",
                        "single OS", "no 1-hour idle or long-running growth test"],
        "runs": [],
    }
    with tempfile.TemporaryDirectory(prefix="filesync-probe-") as work:
        root = Path(work).resolve()
        cert, key = root / "cert.pem", root / "key.pem"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                        "-keyout", str(key), "-out", str(cert), "-days", "1", "-subj", "/CN=localhost",
                        "-addext", "subjectAltName=DNS:localhost,IP:127.0.0.1",
                        "-addext", "basicConstraints=critical,CA:FALSE"],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        url = f"https://127.0.0.1:{port}"
        hub_config = root / "hub.toml"
        hub_config.write_text(f'listen="127.0.0.1:{port}"\ndata_dir={json.dumps(str(root / "hub"))}\n'
                              f'admin_token_env="FILESYNC_PROBE_ADMIN"\ntls_cert={json.dumps(str(cert))}\ntls_key={json.dumps(str(key))}\n')
        ctx = ssl.create_default_context(cafile=str(cert))
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), urllib.request.HTTPSHandler(context=ctx))

        def request(path, body=None, token=admin_token):
            data = None if body is None else json.dumps(body).encode()
            req = urllib.request.Request(url + path, data=data, headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"})
            with opener.open(req, timeout=10) as response:
                content = response.read()
                return json.loads(content) if content else None

        with (root / "hub.log").open("wb") as hub_log:
            hub = subprocess.Popen([str(binary), "--config", str(hub_config), "hub"], env=env, stdout=hub_log, stderr=hub_log)
            try:
                for _ in range(100):
                    if hub.poll() is not None:
                        raise RuntimeError("hub exited: " + (root / "hub.log").read_text())
                    try:
                        request("/health")
                        break
                    except (urllib.error.URLError, TimeoutError):
                        time.sleep(0.1)
                else:
                    raise RuntimeError("hub did not become ready")
                upstream = request("/v1/nodes", {"name": "probe-upstream"})
                downstream = request("/v1/nodes", {"name": "probe-downstream"})
                request("/v1/edges", {"upstream": upstream["id"], "downstream": downstream["id"]})
                source = root / "source"
                source.mkdir()
                for index in range(args.files):
                    (source / f"small-{index:05}.txt").write_bytes(b"s" * 64)
                # Fixture generation is outside the measured agent.
                block = b"large-file-fixture-" * 4096
                with (source / "large.bin").open("wb") as output:
                    remaining = args.large_bytes
                    while remaining:
                        chunk = block[:min(remaining, len(block))]
                        output.write(chunk)
                        remaining -= len(chunk)
                dest = root / "received"

                def agent_config(node, name, extra=""):
                    path = root / f"{name}.toml"
                    path.write_text(f'hub={json.dumps(url)}\nnode_id={json.dumps(node["id"])}\ntoken_env="FILESYNC_PROBE_NODE"\n'
                                    f'data_dir={json.dumps(str(root / name))}\nca_cert={json.dumps(str(cert))}\n' + extra)
                    return path

                a_config = agent_config(upstream, "upstream", f'[[source]]\npath={json.dumps(str(source))}\nprefix="probe"\n')
                b_config = agent_config(downstream, "downstream")

                def measure(name, config, credential):
                    with (root / "agent.out").open("wb") as stdout, (root / "agent.err").open("wb") as stderr:
                        started = time.monotonic()
                        process = subprocess.Popen([str(binary), "--config", str(config), "agent", "--once"],
                                                   env=dict(env, FILESYNC_PROBE_NODE=credential["token"]), stdout=stdout, stderr=stderr)
                        _, status, usage = os.wait4(process.pid, 0)
                        process.returncode = os.waitstatus_to_exitcode(status)
                    if process.returncode:
                        raise RuntimeError((root / "agent.err").read_text())
                    peak = usage.ru_maxrss * (1 if platform.system() == "Darwin" else 1024)
                    row = {"name": name, "peak_rss_bytes": peak, "peak_mb": round(peak / 1_000_000, 3),
                           "peak_mib": round(peak / 1048576, 3), "under_limit": peak < 20_000_000,
                           "elapsed_seconds": round(time.monotonic() - started, 3),
                           "summary": json.loads((root / "agent.out").read_text())}
                    report["runs"].append(row)
                    print(json.dumps(row), flush=True)

                measure("initial_publish", a_config, upstream)
                measure("pending_no_download", b_config, downstream)
                assert not dest.exists(), "unclaimed files created a target directory"
                b_config = agent_config(downstream, "downstream",
                                        f'[[map]]\nupstream={json.dumps(upstream["id"])}\nfrom="probe/large.bin"\nto={json.dumps(str(dest / "large.bin"))}\n')
                measure("claimed_large_download", b_config, downstream)
                assert report["runs"][-1]["summary"]["downloaded"] == 1
                assert report["runs"][-1]["summary"]["pending"] == args.files
                with (source / "large.bin").open("rb") as a, (dest / "large.bin").open("rb") as b:
                    assert hashlib.file_digest(a, "sha256").digest() == hashlib.file_digest(b, "sha256").digest()
                measure("unchanged_publish", a_config, upstream)
                assert report["runs"][-1]["summary"]["published"] == 0
                report["hash_verified"] = True
            finally:
                hub.terminate()
                try:
                    hub.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    hub.kill()
                    hub.wait()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(f"Report: {args.output}; complete acceptance remains pending.")


if __name__ == "__main__":
    main()
