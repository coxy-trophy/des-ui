#!/usr/bin/env python3
"""Record exact source identity without committing or copying user configuration.

Usage: python3 scripts/validation-snapshot.py > /tmp/des-ui-source.json
Attach this JSON and verify.sh's output to a hardware validation record.
"""
import hashlib
import json
import platform
import subprocess
from pathlib import Path

root = Path(__file__).resolve().parents[1]


def git(*args):
    return subprocess.check_output(["git", "-C", str(root), *args])


paths = git("ls-files", "-z", "--cached", "--others", "--exclude-standard").split(b"\0")
files = {}
for raw in sorted(set(paths)):
    if not raw:
        continue
    name = raw.decode("utf-8", errors="surrogateescape")
    path = root / name
    if path.is_symlink():
        files[name] = {"symlink": str(path.readlink())}
    elif path.is_file():
        files[name] = {"sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                       "executable": bool(path.stat().st_mode & 0o111)}
    else:
        files[name] = {"missing": True}
identity = hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest()
print(json.dumps({"head": git("rev-parse", "HEAD").decode().strip(),
                  "working_tree": git("status", "--short").decode(),
                  "source_sha256": identity, "kernel": platform.release(),
                  "machine": platform.machine(), "files": files}, indent=2))
