import importlib.machinery
import importlib.util
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest

loader = importlib.machinery.SourceFileLoader('startup', str(Path(__file__).parents[1] / 'des-ui-startup'))
spec = importlib.util.spec_from_loader(loader.name, loader)
startup = importlib.util.module_from_spec(spec)
loader.exec_module(startup)


class StartupTests(unittest.TestCase):
    def run_child(self, source, timeout=0.15):
        return startup.run([sys.executable, '-c', source], timeout=timeout)

    def test_clean_logout_before_readiness(self):
        self.assertEqual(self.run_child('pass'), 0)

    def test_early_failure(self):
        self.assertEqual(self.run_child('raise SystemExit(7)'), 7)

    def test_ready_session_outlives_startup_deadline(self):
        start = time.monotonic()
        self.assertEqual(self.run_child('''
import os, time
from pathlib import Path
ready = Path(os.environ['DES_UI_SESSION_READY'])
assert ready.parent.stat().st_mode & 0o777 == 0o700
fd = os.open(ready, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
os.write(fd, b'ready\\n')
os.close(fd)
time.sleep(0.35)
raise SystemExit(3)
'''), 3)
        self.assertGreater(time.monotonic() - start, 0.3)

    def test_hang_is_bounded(self):
        start = time.monotonic()
        self.assertEqual(self.run_child('import time; time.sleep(60)'), 124)
        self.assertLess(time.monotonic() - start, 2)

    def test_timeout_kills_term_resistant_descendant(self):
        with tempfile.TemporaryDirectory() as directory:
            pidfile = Path(directory) / 'pid'
            source = '''
import os, signal, time
signal.signal(signal.SIGTERM, signal.SIG_IGN)
pid = os.fork()
if pid == 0:
    open(%r, 'w').write(str(os.getpid()))
while True:
    time.sleep(60)
''' % str(pidfile)
            self.assertEqual(self.run_child(source, timeout=0.3), 124)
            pid = int(pidfile.read_text())
            # A killed grandchild may remain a zombie until the host init reaps it;
            # either absence or Z proves it cannot keep the display/session alive.
            stat = Path('/proc') / str(pid) / 'stat'
            deadline = time.monotonic() + 1
            while stat.exists() and stat.read_text().split()[2] != 'Z' and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(not stat.exists() or stat.read_text().split()[2] == 'Z')

    def test_rejects_unsafe_or_incomplete_acknowledgements(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'ready'
            path.write_bytes(b'ready\n')
            path.chmod(0o644)
            self.assertFalse(startup.acknowledged(path))
            path.chmod(0o600)
            self.assertTrue(startup.acknowledged(path))
            link = Path(directory) / 'link'
            link.symlink_to(path)
            self.assertFalse(startup.acknowledged(link))
            path.write_bytes(b'read')
            self.assertFalse(startup.acknowledged(path))

    def test_attempts_use_fresh_readiness_paths(self):
        paths = []
        original = startup.subprocess.Popen
        def capture(*args, **kwargs):
            paths.append(kwargs['env']['DES_UI_SESSION_READY'])
            return original(*args, **kwargs)
        from unittest.mock import patch
        with patch.object(startup.subprocess, 'Popen', capture):
            self.run_child('pass')
            self.run_child('pass')
        self.assertNotEqual(*paths)
        self.assertTrue(all(not Path(p).parent.exists() for p in paths))

class LauncherTests(unittest.TestCase):
    def attempt(self, compositor, previous=None, cursor_env=None):
        import shutil
        import subprocess
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bin_dir = root / 'bin'
            bin_dir.mkdir()
            def executable(path, source):
                path.write_text(source)
                path.chmod(0o755)
            source_dir = Path(__file__).parents[1]
            shutil.copy(source_dir / 'des-ui-session', bin_dir)
            executable(bin_dir / 'des-ui-startup', (source_dir / 'des-ui-startup').read_text().replace(
                'sys.exit(run(sys.argv[1:]))', 'sys.exit(run(sys.argv[1:], timeout=0.15))'))
            executable(bin_dir / 'des-comp', '#!/bin/sh\n' + compositor)
            for stub in ('systemctl', 'pkill'):
                executable(bin_dir / stub, '#!/bin/sh\nexit 0\n')
            executable(bin_dir / 'cage', '#!/bin/sh\ntouch "$HOME/cage-used"\nexit 0\n')
            if previous is not None:
                old = root / 'lib/des-ui/previous'
                old.mkdir(parents=True)
                executable(old / 'des-comp', '#!/bin/sh\ntouch "$HOME/previous-used"\nexit 0\n')
                executable(old / 'des-ui', '#!/bin/sh\n# ' + previous + '\n')
            env = dict(os.environ, HOME=directory, PATH=str(bin_dir) + ':/usr/bin:/bin',
                       XDG_STATE_HOME=str(root / 'state'), XDG_CONFIG_HOME=str(root / 'config'),
                       XDG_CACHE_HOME=str(root / 'cache'))
            for name in ('XCURSOR_THEME', 'XCURSOR_SIZE', 'XCURSOR_PATH'):
                env.pop(name, None)
            env.update(cursor_env or {})
            completed = subprocess.run([str(bin_dir / 'des-ui-session')], env=env, timeout=4,
                                       capture_output=True)
            return completed.returncode, (root / 'previous-used').exists(), (root / 'cage-used').exists()

    def test_clean_logout_does_not_fallback(self):
        self.assertEqual(self.attempt('exit 0\n'), (0, False, False))

    def test_cursor_defaults_reach_the_desktop_including_local_theme_path(self):
        source = '''
[ "$XCURSOR_THEME" = DEs-UI ] || exit 9
[ "$XCURSOR_SIZE" = 24 ] || exit 9
case "$XCURSOR_PATH" in *:/usr/local/share/icons:*) ;; *) exit 9;; esac
case "$XCURSOR_PATH" in "$XDG_CACHE_HOME/des-ui/cursor-theme:"*) ;; *) exit 9;; esac
exit 0
'''
        self.assertEqual(self.attempt(source), (0, False, False))

    def test_explicit_cursor_preferences_are_preserved(self):
        source = '''
[ "$XCURSOR_THEME" = UserTheme ] || exit 9
[ "$XCURSOR_SIZE" = 48 ] || exit 9
[ "$XCURSOR_PATH" = /custom/icons ] || exit 9
exit 0
'''
        self.assertEqual(self.attempt(source, cursor_env={
            'XCURSOR_THEME': 'UserTheme', 'XCURSOR_SIZE': '48', 'XCURSOR_PATH': '/custom/icons',
        }), (0, False, False))

    def test_hang_skips_unsupported_previous_and_uses_cage(self):
        self.assertEqual(self.attempt('sleep 60\n', previous='legacy'), (0, False, True))

    def test_capable_previous_build_can_logout(self):
        self.assertEqual(self.attempt('exit 9\n', previous='DES_UI_SESSION_READY'), (0, True, False))


if __name__ == '__main__':
    unittest.main()
