"""Inspect real deb archives built from disposable program fixtures."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class ConnectPackagingTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        for name in ("packaging", "session", "themes", "companion"):
            shutil.copytree(ROOT / name, self.root / name,
                            ignore=shutil.ignore_patterns("tests", "__pycache__"))
        (self.root / "scripts").mkdir()
        shutil.copyfile(ROOT / "scripts/virtual-display-viewer.py",
                        self.root / "scripts/virtual-display-viewer.py")
        (self.root / "shell").mkdir()
        shutil.copyfile(ROOT / "shell/Cargo.toml", self.root / "shell/Cargo.toml")
        for name in ("README.md", "LICENSE"):
            shutil.copyfile(ROOT / name, self.root / name)
        release = self.root / "target/release"
        release.mkdir(parents=True)
        for name in ("des-ui", "des-ui-ctl", "des-comp", "des-portal", "des-hands",
                     "des-music", "des-connectd", "des-connect-ctl"):
            self.executable(release / name, "exit 0")
        self.executable(release / "des-comp", '''
if [ "${1:-}" = --write-cursor-theme ]; then
    mkdir -p "$2/cursors"
    printf '[Icon Theme]\\nName=DEs-UI\\n' >"$2/index.theme"
    printf Xcur >"$2/cursors/default"
fi
''')
        tools = self.root / "tools"
        tools.mkdir()
        self.executable(tools / "dpkg", "echo amd64")
        self.executable(tools / "dpkg-shlibdeps", '''
case "${FAIL_DEPENDENCIES:-}" in
    all) echo 'shlibs:Depends=libpartial-test1'; echo 'fixture: dependency discovery failed' >&2; exit 1;;
    music) case "$*" in *des-music*) echo 'shlibs:Depends=libpartial-test1'; exit 1;; esac;;
esac
case "$*" in
    *des-connectd*) echo 'shlibs:Depends=libconnect-test1, libbase-test1';;
    *) echo 'shlibs:Depends=libbase-test1';;
esac
''')
        self.env = dict(os.environ, PATH=str(tools) + ":" + os.environ["PATH"],
                        DEB_MAINTAINER="Test <test@example.invalid>",
                        FAIL_DEPENDENCIES="")

    def executable(self, path, script):
        path.write_text("#!/bin/sh\nset -eu\n" + script + "\n")
        path.chmod(0o755)

    def build(self):
        return subprocess.run(["sh", "packaging/build-deb.sh", "--no-build"],
                              cwd=self.root, env=self.env, capture_output=True, text=True)

    def test_connect_binaries_helper_and_native_dependencies_are_in_desktop_archive(self):
        result = self.build()
        self.assertEqual(result.returncode, 0, result.stderr)
        archive, = (self.root / "target/deb").glob("des-ui_[0-9]*_amd64.deb")
        extracted = self.root / "extracted"
        subprocess.run(["dpkg-deb", "-x", str(archive), str(extracted)], check=True)
        for name in ("des-connectd", "des-connect-ctl"):
            with self.subTest(program=name):
                binary = extracted / "usr/bin" / name
                self.assertTrue(binary.is_file(), f"{name} is missing from the deb")
                self.assertTrue(os.access(binary, os.X_OK))
        helper = extracted / "usr/share/des-ui/virtual-display-viewer.py"
        self.assertTrue(helper.is_file(), "virtual-display viewer is missing from the deb")
        self.assertEqual(helper.read_bytes(), (ROOT / "scripts/virtual-display-viewer.py").read_bytes())
        dependencies = subprocess.check_output(
            ["dpkg-deb", "-f", str(archive), "Depends"], text=True)
        self.assertIn("libconnect-test1", dependencies)
        self.assertIn("python3", dependencies)
        self.assertFalse(list(extracted.rglob("*des-connect*.service")), "Connect must remain opt-in")

    def test_dependency_discovery_failure_stops_desktop_package(self):
        self.env["FAIL_DEPENDENCIES"] = "all"
        result = self.build()
        self.assertNotEqual(result.returncode, 0, "dependency errors must fail packaging")
        self.assertIn("fixture: dependency discovery failed", result.stderr)
        self.assertFalse(list((self.root / "target/deb").glob("*.deb")))

    def test_music_dependency_failure_does_not_produce_music_archive(self):
        self.env["FAIL_DEPENDENCIES"] = "music"
        result = self.build()
        self.assertNotEqual(result.returncode, 0, "music dependency errors must fail packaging")
        self.assertFalse(list((self.root / "target/deb").glob("des-ui-music_*.deb")))

    def test_missing_connect_binary_fails_packaging(self):
        (self.root / "target/release/des-connectd").unlink()
        self.assertNotEqual(self.build().returncode, 0)

    def test_uninstall_removes_connect_programs_without_removing_user_state(self):
        tools = self.root / "tools"
        log = self.root / "removals"
        self.executable(tools / "sudo", 'printf "%s\\n" "$@" >> "$REMOVAL_LOG"')
        env = dict(self.env, REMOVAL_LOG=str(log))
        subprocess.run(["sh", str(ROOT / "uninstall.sh")], env=env, check=True, capture_output=True)
        removed = log.read_text().splitlines()
        self.assertIn("/usr/local/bin/des-connectd", removed)
        self.assertIn("/usr/local/bin/des-connect-ctl", removed)
        self.assertFalse(any(".config" in path or ".local/state" in path for path in removed))
