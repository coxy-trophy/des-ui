import os
from pathlib import Path
import subprocess
import tempfile
import unittest


class LoginStateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.mock = self.root / 'mock'
        self.mock.mkdir()
        self.log = self.root / 'commands'
        source = (Path(__file__).parents[1] / 'des-ui-login').read_text()
        for prefix in ['/etc/', '/var/', '/usr/lib/']:
            source = source.replace(prefix, str(self.root) + prefix)
        self.script = self.root / 'des-ui-login'
        self.script.write_text(source)
        self.config = self.root / 'etc/greetd/config.toml'
        self.config.parent.mkdir(parents=True)
        self.config.write_text('[terminal]\nvt = 7\n[default_session]\ncommand = "agreety"\nuser = "greeter"\n')
        alias = self.root / 'etc/systemd/system/display-manager.service'
        alias.parent.mkdir(parents=True)
        alias.symlink_to('/usr/lib/systemd/system/gdm.service')
        known = self.root / 'usr/lib/systemd/system/gdm.service'
        known.parent.mkdir(parents=True)
        known.touch()
        self.legacy = self.root / 'var/lib/des-ui/greeter'
        self.legacy.mkdir(parents=True)
        for directory in [self.root / 'var', self.root / 'var/lib', self.root / 'var/lib/des-ui']:
            directory.chmod(0o755)
        self.state = self.root / 'var/lib/des-ui/login'
        self.write_mock('id', '#!/bin/sh\n[ "$1" != -u ] || { echo 0; exit; }\nexit 0\n')
        self.write_mock('install', '#!/usr/bin/python3\nimport subprocess,sys\na=sys.argv[1:]\nwhile "-o" in a:\n i=a.index("-o"); del a[i:i+2]\nsys.exit(subprocess.call(["/usr/bin/install",*a]))\n')
        self.write_mock('stat', '#!/bin/sh\nif [ "$1" = -c ] && [ "$2" = %u ]; then echo 0; else /usr/bin/stat "$@"; fi\n')
        self.write_mock('systemctl', '#!/bin/sh\nprintf "%s\\n" "$*" >> "$TEST_COMMANDS"\n')
        self.env = dict(os.environ, PATH=str(self.mock) + ':' + os.environ['PATH'], TEST_COMMANDS=str(self.log))

    def write_mock(self, name, source):
        path = self.mock / name
        path.write_text(source)
        path.chmod(0o755)

    def run_login(self, action):
        return subprocess.run(['sh', str(self.script), action], env=self.env, capture_output=True, text=True)

    def commands(self):
        return self.log.read_text() if self.log.exists() else ''

    def test_enable_never_writes_legacy_symlink_and_records_private_state(self):
        victim = self.root / 'victim'
        victim.write_text('preserved')
        (self.legacy / 'previous-login-manager').symlink_to(victim)
        result = self.run_login('enable')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(victim.read_text(), 'preserved')
        self.assertEqual((self.state / 'previous-login-manager').read_text(), 'gdm.service\n')
        self.assertEqual(self.state.stat().st_mode & 0o777, 0o700)
        self.assertEqual(self.run_login('disable').returncode, 0)
        self.assertIn('enable --force gdm.service', self.commands())

    def test_old_install_never_enables_greeter_supplied_service(self):
        self.config.write_text('command = "/usr/bin/des-ui-greeter"\nuser = "greeter"\n')
        (self.legacy / 'previous-login-manager').write_text('/tmp/evil.service\n')
        result = self.run_login('disable')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn('evil.service', self.commands())
        self.assertIn('enable --force gdm.service', self.commands())

    def test_rejects_symlinked_private_directory(self):
        self.state.symlink_to(self.legacy, target_is_directory=True)
        self.assertNotEqual(self.run_login('enable').returncode, 0)
        self.assertEqual(self.commands(), '')

    def test_repeated_enable_preserves_original_manager(self):
        self.assertEqual(self.run_login('enable').returncode, 0)
        alias = self.root / 'etc/systemd/system/display-manager.service'
        alias.unlink()
        alias.symlink_to('/usr/lib/systemd/system/greetd.service')
        self.assertEqual(self.run_login('enable').returncode, 0)
        self.assertEqual((self.state / 'previous-login-manager').read_text(), 'gdm.service\n')
        self.assertEqual(self.run_login('disable').returncode, 0)
        self.assertIn('enable --force gdm.service', self.commands())

    def test_rejects_writable_private_parent(self):
        self.state.parent.chmod(0o777)
        self.assertNotEqual(self.run_login('enable').returncode, 0)
        self.assertEqual(self.commands(), '')

    def test_rejects_private_state_symlink_and_absolute_unit(self):
        for symlink in [False, True]:
            with self.subTest(symlink=symlink):
                self.state.mkdir(mode=0o700, exist_ok=True)
                saved = self.state / 'previous-login-manager'
                saved.unlink(missing_ok=True)
                victim = self.root / 'evil'
                victim.write_text('/tmp/evil.service\n')
                if symlink:
                    saved.symlink_to(victim)
                else:
                    saved.write_text('/tmp/evil.service\n')
                self.config.write_text('command = "/usr/bin/des-ui-greeter"\nuser = "greeter"\n')
                self.assertNotEqual(self.run_login('disable').returncode, 0)
                self.assertEqual(self.commands(), '')


if __name__ == '__main__':
    unittest.main()
