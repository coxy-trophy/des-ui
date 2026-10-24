"""Build package staging trees in a temporary repo, without installing anything."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class PackagingTests(unittest.TestCase):
    def test_desktop_package_contains_hand_control_and_its_dependencies(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for name in ("packaging", "session", "themes", "companion"):
                shutil.copytree(ROOT / name, root / name,
                                ignore=shutil.ignore_patterns("tests", "__pycache__"))
            (root / "shell").mkdir()
            shutil.copyfile(ROOT / "shell/Cargo.toml", root / "shell/Cargo.toml")
            for name in ("README.md", "LICENSE"):
                shutil.copyfile(ROOT / name, root / name)
            release = root / "target/release"
            release.mkdir(parents=True)
            for name in ("des-ui", "des-ui-ctl", "des-comp", "des-portal", "des-hands", "des-music"):
                path = release / name
                path.write_text("#!/bin/sh\nexit 0\n")
                path.chmod(0o755)
            (release / "des-comp").write_text('''#!/bin/sh
set -eu
if [ "${1:-}" = --write-cursor-theme ]; then
    mkdir -p "$2/cursors"
    printf '[Icon Theme]\\nName=DEs-UI\\n' >"$2/index.theme"
    printf Xcur >"$2/cursors/default"
fi
''')
            tools = root / "tools"
            tools.mkdir()
            mocks = {
                "dpkg": "echo amd64",
                "dpkg-shlibdeps": 'case "$*" in *des-hands*) echo "shlibs:Depends=libhand-test1";; *) echo "shlibs:Depends=libbase-test1";; esac',
                "dpkg-deb": '''
set -eu
for arg; do
    case "$arg" in
        */des-ui)
            test -s "$arg/usr/share/des-ui/companion/service.py" || { echo 'companion service missing' >&2; exit 1; }
            test -s "$arg/usr/share/des-ui/companion/voice.py" || { echo 'companion voice missing' >&2; exit 1; }
            test ! -e "$arg/usr/share/des-ui/companion/test_service.py" || { echo 'companion tests packaged' >&2; exit 1; }
            grep -q '^Depends: .*python3-websocket' "$arg/DEBIAN/control" || { echo 'companion websocket dependency missing' >&2; exit 1; }
            test -x "$arg/usr/bin/des-hands" || { echo 'hand control missing from package' >&2; exit 1; }
            test -s "$arg/usr/share/icons/DEs-UI/cursors/default" || { echo 'cursor theme missing from package' >&2; exit 1; }
            grep -q '^Name=DEs-UI' "$arg/usr/share/icons/DEs-UI/index.theme" || { echo 'cursor theme metadata missing' >&2; exit 1; }
            grep -q '^Depends: .*libhand-test1' "$arg/DEBIAN/control" || { echo 'hand dependencies missing' >&2; exit 1; }
            ;;
    esac
done
for last; do :; done
touch "$last"
''',
            }
            for name, script in mocks.items():
                path = tools / name
                path.write_text("#!/bin/sh\n" + script + "\n")
                path.chmod(0o755)
            env = dict(os.environ, PATH=str(tools) + ":" + os.environ["PATH"],
                       DEB_MAINTAINER="Test <test@example.invalid>")
            result = subprocess.run(["sh", "packaging/build-deb.sh", "--no-build"],
                                    cwd=root, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_uninstall_removes_hand_control(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            mock = root / "sudo"
            mock.write_text('#!/bin/sh\nprintf "%s\\n" "$@" >> "$REMOVAL_LOG"\n')
            mock.chmod(0o755)
            log = root / "removals"
            env = dict(os.environ, PATH=str(root) + ":" + os.environ["PATH"], REMOVAL_LOG=str(log))
            subprocess.run(["sh", str(ROOT / "uninstall.sh")], env=env, check=True, capture_output=True)
            self.assertIn("/usr/local/bin/des-hands", log.read_text().splitlines())
            self.assertIn("/usr/local/share/icons/DEs-UI", log.read_text().splitlines())
