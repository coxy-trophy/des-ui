#!/usr/bin/env python3
"""Native overview regression in a private Xvfb desktop; never uses the live session.

Requires built target/debug binaries, Xvfb, xdotool, Chromium and Pillow.
Run with --x11 to exercise Xwayland instead of native Wayland. Raw captures
are kept only in the printed private temporary directory, never in the repo.
"""
import argparse
import json
import os
from pathlib import Path
import select
import shutil
import signal
import subprocess
import tempfile
import time
from urllib.parse import unquote, urlparse

from PIL import Image


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--x11', action='store_true')
    parser.add_argument('--current', action='store_true', help='keep the app on the current workspace')
    parser.add_argument('--width', type=int, choices=[1280, 1920], default=1920)
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    binaries = repo / 'target/debug'
    for name in ['Xvfb', 'xdotool', 'chromium']:
        if not shutil.which(name):
            raise RuntimeError(f'{name} is required')
    root = Path(tempfile.mkdtemp(prefix='desui-overview-check-'))
    env = {'PATH': os.environ.get('PATH', '/usr/bin:/bin'), 'LANG': 'C.UTF-8',
           'LIBGL_ALWAYS_SOFTWARE': '1', 'SHELL': '/bin/sh',
           'DBUS_SESSION_BUS_ADDRESS': f'unix:path={root}/no-dbus'}
    for key in ['HOME', 'XDG_CONFIG_HOME', 'XDG_CONFIG_DIRS', 'XDG_STATE_HOME', 'XDG_CACHE_HOME', 'XDG_RUNTIME_DIR']:
        folder = root / key.lower()
        folder.mkdir(mode=0o700)
        env[key] = str(folder)
    config = Path(env['XDG_CONFIG_HOME']) / 'des-ui'
    config.mkdir()
    (config / 'config').write_text('panels =\npet = off\nlock_after = never\nkeep_terminal_sessions = off\ndo_not_disturb = on\ngeo = off\nprocessor_temperature_warnings = off\n')
    client = root / 'client.py'
    browser_args = [shutil.which('chromium'), f'--user-data-dir={root}/browser',
                    '--ozone-platform=' + ('x11' if args.x11 else 'wayland'),
                    '--no-first-run', '--disable-background-networking',
                    '--app=data:text/html,<body style="background:lightgreen"><h1>Overview regression</h1><p>Native application pixels</p></body>']
    client.write_text('import os,time\ntime.sleep(1)\nos.execv(' + repr(browser_args[0]) + ', ' + repr(browser_args) + ')\n')
    wrapper = root / 'shell'
    wrapper.write_text('#!/usr/bin/python3\nimport os,subprocess\nsubprocess.Popen(["/usr/bin/python3",' + repr(str(client)) + '])\nos.execv(' + repr(str(binaries / 'des-ui')) + ',["des-ui"])\n')
    wrapper.chmod(0o700)
    height = 1080 if args.width == 1920 else 900
    log = (root / 'check.log').open('w')
    read_fd, write_fd = os.pipe()
    xserver = subprocess.Popen(['Xvfb', '-displayfd', str(write_fd), '-screen', '0', f'{args.width}x{height}x24', '-nolisten', 'tcp'], pass_fds=[write_fd], stdout=log, stderr=log)
    os.close(write_fd)
    compositor = None
    try:
        if not select.select([read_fd], [], [], 10)[0]:
            raise RuntimeError('Xvfb did not start')
        env['DISPLAY'] = ':' + os.read(read_fd, 32).decode().strip()
        command = [str(binaries / 'des-comp'), '--backend', 'winit', '--shell', str(wrapper)]
        if not args.x11:
            command.append('--no-xwayland')
        compositor = subprocess.Popen(command, env=env, stdout=log, stderr=log, start_new_session=True)

        def ctl(*words):
            result = subprocess.run([str(binaries / 'des-ui-ctl'), *words], env=env, capture_output=True, text=True, timeout=15)
            if result.returncode:
                raise RuntimeError(result.stdout + result.stderr)
            return result.stdout.strip()

        # Wait for our shell and native client, rather than opening any live apps.
        for _ in range(100):
            if compositor.poll() is not None:
                raise RuntimeError('nested compositor stopped')
            try:
                state = json.loads(ctl('state').removeprefix('ok '))
                if state.get('apps'):
                    break
            except (RuntimeError, json.JSONDecodeError):
                pass
            time.sleep(.1)
        else:
            raise RuntimeError('native client did not appear')
        found = subprocess.run(['xdotool', 'search', '--pid', str(compositor.pid)], env=env, capture_output=True, text=True, check=True)
        window = found.stdout.splitlines()[0]
        subprocess.run(['xdotool', 'windowsize', window, str(args.width), str(height)], env=env, check=True)
        time.sleep(1)
        if not args.current:
            ctl('workspace', '1')  # Hide the app while viewing workspace 2.
        ctl('overview')
        # Control replies acknowledge enqueueing, not a Wayland buffer commit.
        # Wait boundedly for the shell's presentation as well as native pixels.
        for attempt in range(10):
            time.sleep(.3)
            answer = json.loads(ctl('screenshot-file', 'screen').removeprefix('ok '))
            capture = Path(unquote(urlparse(answer['uri']).path))
            im = Image.open(capture).convert('RGB')
            pixels = im.load()
            green = [(x, y) for y in range(im.height) for x in range(im.width)
                     if (lambda rgb: rgb[1] > 180 and 90 < rgb[0] < 180 and 90 < rgb[2] < 180)(pixels[x, y])]
            if len(green) > 2000 and pixels[8, 200] == pixels[4, 200]:
                break
        assert len(green) > 2000, 'capture omitted native app content from hidden workspace'
        x0, x1 = min(x for x, _ in green), max(x for x, _ in green)
        y0, y1 = min(y for _, y in green), max(y for _, y in green)
        right, bottom = (486, 1032) if args.width == 1920 else (628, 450)
        assert x0 >= 64 and x1 <= right - 16 and y0 >= 128 and y1 <= bottom - 24, (x0, y0, x1, y1)
        assert pixels[8, 200] == pixels[4, 200], 'underlying collapsed rail outline leaked into overview'
        print(json.dumps({'result': 'passed', 'backend': 'Xwayland' if args.x11 else 'Wayland', 'workspace': 'current' if args.current else 'hidden', 'size': im.size, 'native_pixels': len(green), 'capture_attempts': attempt + 1, 'capture': str(capture), 'evidence': str(root)}))
    finally:
        os.close(read_fd)
        if compositor is not None:
            try:
                os.killpg(compositor.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            compositor.wait(timeout=10)
        xserver.terminate()
        xserver.wait(timeout=10)
        log.close()


if __name__ == '__main__':
    main()
