#!/usr/bin/env python3
"""Run the real loopback UCTP profile without providers or fixed ports."""
import os
from pathlib import Path
import queue
import signal
import subprocess
import threading
import uuid


def main():
    root = Path(__file__).resolve().parents[4]
    subprocess.run(
        ["cargo", "build", "--locked", "-p", "rvoip-websocket", "--example", "application_profile"],
        cwd=root, check=True,
    )
    target = Path(os.environ.get("CARGO_TARGET_DIR", root / "target"))
    if not target.is_absolute():
        target = root / target
    binary = target / "debug" / "examples" / ("application_profile.exe" if os.name == "nt" else "application_profile")
    env = dict(os.environ, RVOIP_EXAMPLE_TOKEN=uuid.uuid4().hex)
    server = subprocess.Popen(
        [str(binary), "--server"], cwd=root, env=env,
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
    )
    try:
        lines = queue.Queue()
        reader = threading.Thread(target=lambda: lines.put(server.stdout.readline()), daemon=True)
        reader.start()
        address = lines.get(timeout=10).strip()
        if not address.startswith("ws://127.0.0.1:"):
            raise RuntimeError("example host did not report a loopback address")
        good = subprocess.run(
            [str(binary), "--client", address], cwd=root, env=env,
            timeout=15, capture_output=True, text=True, check=True,
        )
        if "Authenticated, correlated echo:" not in good.stdout or "Duplicate ID refused with 409" not in good.stdout:
            raise RuntimeError("example client did not verify echo and duplicate refusal")
        print(good.stdout, end="")
        bad = subprocess.run(
            [str(binary), "--client", address], cwd=root,
            env=dict(env, RVOIP_EXAMPLE_TOKEN=uuid.uuid4().hex),
            timeout=15, capture_output=True, text=True,
        )
        if bad.returncode == 0 or "example authentication failed" not in bad.stderr:
            raise RuntimeError("example did not explicitly refuse the wrong credential")
        print("Wrong credential refused; loopback application profile verified")
    finally:
        if server.poll() is None:
            server.send_signal(signal.SIGINT if os.name != "nt" else signal.SIGTERM)
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                server.kill()
                server.wait(timeout=5)
        server.stdout.close()


if __name__ == "__main__":
    main()
