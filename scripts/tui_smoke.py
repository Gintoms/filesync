#!/usr/bin/env python3
"""Unix PTY smoke test of the TUI against a local control-API fixture."""
import argparse
import errno
import fcntl
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import tempfile
import termios
import threading
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/filesync"))
    parser.add_argument("--output", type=Path, default=Path(".local/tui-smoke.ansi"))
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    token = "local-tui-fixture-only-" * 3

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            if self.headers.get("Authorization") != "Bearer " + token:
                self.send_error(401)
                return
            data = [{"upstream": "fixture-upstream", "path": "quotes/customer.txt",
                     "sha256": "0" * 64, "size": 100, "target": None, "state": "pending", "error": None}]
            body = json.dumps(data).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *args):
            pass

    with tempfile.TemporaryDirectory(prefix="filesync-tui-") as work:
        root = Path(work).resolve()
        server = HTTPServer(("127.0.0.1", 0), Handler)
        (root / "control.addr").write_text(f"127.0.0.1:{server.server_port}")
        config = root / "agent.toml"
        config.write_text(f'hub="http://127.0.0.1:1"\nnode_id="fixture"\ntoken_env="FILESYNC_TUI_TEST"\ndata_dir={json.dumps(str(root))}\n')
        pid, fd = pty.fork()
        if pid == 0:
            os.environ.update(TERM="xterm-256color", FILESYNC_TUI_TEST=token)
            os.execv(str(binary), [str(binary), "--config", str(config)])
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
        content = bytearray()
        exited = False
        quit_sent = False
        try:
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                if select.select([fd], [], [], 0.1)[0]:
                    try:
                        content.extend(os.read(fd, 65536))
                    except OSError as error:
                        if error.errno != errno.EIO:
                            raise
                if b"quotes/customer.txt" in content and not quit_sent:
                    os.write(fd, b"q")
                    quit_sent = True
                result, status = os.waitpid(pid, os.WNOHANG)
                if result:
                    exited = True
                    assert os.waitstatus_to_exitcode(status) == 0, content.decode(errors="replace")
                    break
            assert exited and quit_sent, "TUI failed to render and exit on q"
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_bytes(content)
            print(f"TUI render / authenticated pending request / q exit: PASS ({args.output})")
        finally:
            if not exited:
                os.kill(pid, signal.SIGKILL)
                os.waitpid(pid, 0)
            os.close(fd)
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    main()
