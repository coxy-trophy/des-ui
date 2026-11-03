#!/usr/bin/env python3
"""Open a disposable nested desktop with real grouped app windows for dock review."""
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile
import threading
import time

ROOT = Path(__file__).resolve().parents[1]

def empty_music(listener, stop):
    def client(stream):
        state = {'type': 'state', 'track': None, 'playing': False, 'position_ms': 0,
                 'volume': 50, 'shuffle': False, 'repeat': 'off'}
        reply = (json.dumps(state) + '\n').encode()
        try:
            stream.settimeout(0.2); stream.sendall(reply)
            while not stop.is_set():
                try:
                    data = stream.recv(4096)
                    if not data:
                        break
                    stream.sendall(reply)
                except socket.timeout:
                    pass
        except OSError:
            pass
        finally:
            stream.close()
    listener.settimeout(0.2)
    while not stop.is_set():
        try:
            stream, _ = listener.accept()
            threading.Thread(target=client, args=(stream,), daemon=True).start()
        except socket.timeout:
            pass
        except OSError:
            break

def stop_group(process):
    if process is not None and process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()

def main():
    for name in ('des-comp', 'des-ui'):
        if not (ROOT / 'target/debug' / name).is_file():
            raise SystemExit('Build the preview first: cargo build -p des-comp -p des-ui')
    original_runtime = Path(os.environ.get('XDG_RUNTIME_DIR', '/run/user/' + str(os.getuid())))
    original_display = os.environ.get('WAYLAND_DISPLAY', 'wayland-1')
    parent_display = Path(original_display)
    if not parent_display.is_absolute():
        parent_display = original_runtime / original_display
    with tempfile.TemporaryDirectory(prefix='des-left-dock-', dir=original_runtime) as temporary:
        runtime = Path(temporary)
        (runtime / 'config/des-ui').mkdir(parents=True)
        (runtime / 'config/des-ui/config').write_text('geo = off\nprocessor_temperature_warnings = off\n')
        env = os.environ.copy()
        for key in ('DES_COMP_SOCKET', 'DES_UI_TERMINAL_DIR', 'DES_VIRTUAL_SOCKET'):
            env.pop(key, None)
        env.update(XDG_RUNTIME_DIR=str(runtime), WAYLAND_DISPLAY=str(parent_display),
                   XDG_CONFIG_HOME=str(runtime / 'config'), XDG_STATE_HOME=str(runtime / 'state'),
                   XDG_CACHE_HOME=str(runtime / 'cache'), DES_MUSIC_SOCKET=str(runtime / 'music.sock'),
                   DES_UI_CONTROL_SOCKET=str(runtime / 'control.sock'))
        listener = socket.socket(socket.AF_UNIX)
        listener.bind(env['DES_MUSIC_SOCKET']); Path(env['DES_MUSIC_SOCKET']).chmod(0o600); listener.listen(4)
        stop = threading.Event()
        threading.Thread(target=empty_music, args=(listener, stop), daemon=True).start()
        compositor = files = None
        try:
            compositor = subprocess.Popen([str(ROOT / 'target/debug/des-comp'), '--backend', 'winit', '--no-xwayland'], env=env, start_new_session=True)
            deadline = time.monotonic() + 20
            while not (runtime / 'control.sock').exists():
                if compositor.poll() is not None:
                    raise SystemExit('Preview compositor exited before its shell started.')
                if time.monotonic() > deadline:
                    raise SystemExit('Preview shell startup timed out.')
                time.sleep(0.1)
            display = next(p for p in runtime.glob('wayland-*') if p.is_socket())
            app_env = env.copy(); app_env.update(WAYLAND_DISPLAY=str(display), GDK_BACKEND='wayland')
            # A separate bus keeps these file-manager windows separate from the
            # owner's existing Thunar process and real desktop session.
            files = subprocess.Popen(['dbus-run-session', '--', 'sh', '-c',
                'thunar -w "$1" & first=$!; sleep 1; thunar -w "$2"; wait "$first"',
                'dock-demo', str(ROOT), str(ROOT / 'shell')], env=app_env, start_new_session=True)
            print('Preview ready: click the clock or top button to expand; click the grouped file-manager icon.', flush=True)
            print('Close the preview desktop to end its temporary windows and services.', flush=True)
            return compositor.wait()
        finally:
            stop_group(files); stop_group(compositor)
            stop.set(); listener.close()

if __name__ == '__main__':
    raise SystemExit(main())
