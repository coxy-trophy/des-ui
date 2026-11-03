#!/usr/bin/env python3
"""Open an isolated, silent native lyric-motion preview. No Spotify account needed."""
import argparse
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time

LINES = [
    'Let the words arrive with the music.',
    'Keep the current phrase in focus.',
    'Each line moves into place.',
    'Without pulling the page from your hands.',
    'Scroll back and take your time.',
    'Return when you want to follow.',
    'A longer phrase should wrap once and keep its shape while the highlight moves through the song.',
    '',
    'Quiet motion, bright words.',
    'Then let the next phrase take the lead.',
    'Pause, and let the scene rest.',
    'This is a silent lyric-motion study.',
]
URI = 'spotify:track:des-lyrics-motion-demo'
LENGTH = len(LINES) * 3000

def serve_client(client, stop):
    playing, base, began, volume = True, 0, time.monotonic(), 50
    pending = b''
    def position():
        return int(base + ((time.monotonic() - began) * 1000 if playing else 0)) % LENGTH
    def send(value):
        client.sendall((json.dumps(value) + '\n').encode())
    def state():
        send({'type': 'state', 'playing': playing, 'position_ms': position(), 'volume': volume,
              'track': {'uri': URI, 'name': 'Lyric motion study — silent preview',
                        'artists': 'DEs-UI', 'album': 'Animation prototype',
                        'duration_ms': LENGTH}, 'repeat': 'all', 'shuffle': False})
    def lyrics():
        send({'type': 'lyrics', 'track': URI, 'synced': True, 'error': None,
              'lines': [{'ms': i * 3000, 'text': text} for i, text in enumerate(LINES)]})
    try:
        client.settimeout(0.1)
        state(); lyrics()
        send({'type': 'list', 'kind': 'playlists', 'key': '', 'items': [], 'error': None, 'offset': 0, 'more': False})
        tick = time.monotonic()
        while not stop.is_set():
            try:
                data = client.recv(4096)
                if not data:
                    break
                pending += data
                if len(pending) > 65536:
                    break
                while b'\n' in pending:
                    line, pending = pending.split(b'\n', 1)
                    request = json.loads(line)
                    command = request.get('cmd')
                    if command == 'lyrics':
                        lyrics()
                    elif command == 'playlists':
                        send({'type': 'list', 'kind': 'playlists', 'key': '', 'items': [], 'error': None, 'offset': 0, 'more': False})
                    elif command == 'seek':
                        base = max(0, min(int(request.get('ms', 0)), LENGTH - 1)); began = time.monotonic()
                    elif command == 'toggle':
                        base = position(); began = time.monotonic(); playing = not playing
                    elif command in ('next', 'prev'):
                        base = 0; began = time.monotonic()
                    elif command == 'volume':
                        volume = max(0, min(int(request.get('percent', 50)), 100))
                    state()
            except socket.timeout:
                pass
            if playing and time.monotonic() - tick > 0.5:
                state(); tick = time.monotonic()
    except (OSError, ValueError):
        pass
    finally:
        client.close()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path(__file__).resolve().parents[1] / 'target/debug/des-ui')
    parser.add_argument('--no-animation', action='store_true')
    parser.add_argument('--screenshot', type=Path)
    args = parser.parse_args()
    if not args.binary.is_file():
        parser.error('Build the shell first: cargo build -p des-ui')
    with tempfile.TemporaryDirectory(prefix='des-lyrics-demo-') as temporary:
        root = Path(temporary)
        (root / 'config/des-ui').mkdir(parents=True)
        (root / 'config/des-ui/config').write_text('panels = none\ngeo = off\nprocessor_temperature_warnings = off\nanimate = ' + ('off' if args.no_animation else 'on') + '\n')
        listener = socket.socket(socket.AF_UNIX)
        address = root / 'music.sock'
        listener.bind(str(address)); address.chmod(0o600); listener.listen(4); listener.settimeout(0.2)
        stop = threading.Event()
        def accept():
            while not stop.is_set():
                try:
                    client, _ = listener.accept()
                    threading.Thread(target=serve_client, args=(client, stop), daemon=True).start()
                except socket.timeout:
                    pass
                except OSError:
                    break
        worker = threading.Thread(target=accept, daemon=True); worker.start()
        env = os.environ.copy()
        for key in ('DES_COMP_SOCKET', 'DES_UI_TERMINAL_DIR', 'DES_VIRTUAL_SOCKET'):
            env.pop(key, None)
        env.update(DES_MUSIC_SOCKET=str(address), DES_UI_CONTROL_SOCKET=str(root / 'control.sock'),
                   XDG_CONFIG_HOME=str(root / 'config'), XDG_STATE_HOME=str(root / 'state'),
                   XDG_CACHE_HOME=str(root / 'cache'))
        command = [str(args.binary), '--windowed', '--music=lyrics', '--theme', 'blade']
        if args.screenshot:
            command += ['--screenshot', str(args.screenshot.resolve())]
        print('Opening a silent preview. Playback controls affect only this demo.', flush=True)
        try:
            return subprocess.call(command, env=env)
        finally:
            stop.set(); listener.close(); worker.join(timeout=1)

if __name__ == '__main__':
    raise SystemExit(main())
