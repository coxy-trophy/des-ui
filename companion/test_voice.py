import base64
import json
import queue
import os
import subprocess
import sys
from pathlib import Path
import time
import threading
import unittest
from companion.voice import LiveVoice


def until(predicate):
    end = time.monotonic() + 2
    while not predicate():
        if time.monotonic() > end:
            raise AssertionError('condition did not become true')
        time.sleep(.005)


class Socket:
    def __init__(self):
        self.incoming = queue.Queue()
        self.sent = []
        self.closed = False
    def send(self, raw): self.sent.append(json.loads(raw))
    def recv(self):
        try: return json.dumps(self.incoming.get(timeout=.05))
        except queue.Empty: raise TimeoutError()
    def close(self, **kwargs): self.closed = True
    def event(self, kind, **fields): self.incoming.put(dict(type=kind, **fields))


class Audio:
    def __init__(self, capture, failure):
        self.capture = capture
        self.failure = failure
        self.started = False
        self.stopped = False
        self.muted = False
        self.played = []
    def start(self): self.started = True
    def play(self, data): self.played.append(data)
    def mute(self, value): self.muted = value
    def stop(self): self.stopped = True


class VoiceTests(unittest.TestCase):
    def setUp(self):
        self.now = 100.
        self.events, self.words, self.audio = [], [], []
        self.socket = Socket()
        def audio(capture, failure):
            value = Audio(capture, failure)
            self.audio.append(value)
            return value
        self.voice = LiveVoice(self.events.append, lambda role, text: self.words.append((role, text)),
            connect=lambda key: self.socket, audio_factory=audio, clock=lambda: self.now)
    def tearDown(self):
        self.voice.stop('test')
        self.socket.event('session.closed', usage={'seconds': 0})
    def start(self):
        self.voice.start([{'role':'user','text':'Hello'}], 'Be kind', 'secret', 'gpt-5.6-luna')
        until(lambda: self.socket.sent)
    def ready(self):
        self.start()
        self.socket.event('session.started', session={'id':'test'})
        until(lambda: self.audio and self.audio[0].started)
    def test_speaking_tracks_output_playback_and_drains_without_session_stop(self):
        self.ready()
        self.socket.event('session.output_audio.delta', delta=base64.b64encode(b'\x00\x00'*24000).decode())
        until(lambda: any(e['state'] == 'speaking' for e in self.events))
        self.now += 1.1
        until(lambda: self.events[-1]['state'] == 'listening')
        self.assertFalse(self.audio[0].stopped)

    def test_empty_audio_does_not_signal_speaking(self):
        self.ready()
        self.socket.event('session.output_audio.delta', delta='')
        time.sleep(.1)
        self.assertFalse(any(e['state'] == 'speaking' for e in self.events))

    def test_waits_for_ack_and_sends_documented_configuration(self):
        self.start()
        self.assertFalse(self.audio)
        config = self.socket.sent[0]
        self.assertEqual(config['type'], 'session.start')
        self.assertEqual(config['session']['model'], 'gpt-live-1')
        self.assertEqual(config['session']['audio']['format'], {'type':'audio/pcm','rate':24000})
        self.assertEqual(config['session']['delegation']['responses']['tools'], [])
        self.assertEqual(config['session']['input'][0]['content'][0]['text'], 'Hello')
    def test_streams_pcm_and_exact_transcript_fragments(self):
        self.ready()
        self.audio[0].capture(b'\x01\x02\x03')
        self.audio[0].capture(b'\x04')
        until(lambda: len(self.socket.sent) >= 3)
        chunks = [base64.b64decode(e['audio']) for e in self.socket.sent if e['type']=='session.input_audio.append']
        self.assertEqual(b''.join(chunks), b'\x01\x02\x03\x04')
        self.socket.event('session.output_audio.delta', delta=base64.b64encode(b'ab').decode())
        self.socket.event('session.input_transcript.delta', delta=' Hi')
        until(lambda: self.words)
        self.assertEqual(self.words, [('user',' Hi')])
        self.assertEqual(self.audio[0].played, [b'ab'])
    def test_stop_immediately_stops_audio_and_rejects_late_events(self):
        self.ready()
        self.voice.stop('parent_closed')
        self.assertTrue(self.audio[0].stopped)
        self.socket.event('session.output_audio.delta', delta='YWI=')
        self.socket.event('session.input_transcript.delta', delta='late')
        self.socket.event('session.closed', usage={'seconds':15})
        until(lambda: self.socket.closed)
        self.assertFalse(self.audio[0].played)
        self.assertFalse(self.words)
        until(lambda: self.events[-1]['state'] == 'stopped')
        self.assertTrue(self.events[-1]['finalized'])
        self.assertEqual(self.events[-1]['usage_seconds'], 15)
    def test_independent_watchdog_enforces_hard_cap_even_when_kept(self):
        self.ready()
        self.voice.keep()
        self.now += 300
        until(lambda: self.audio[0].stopped)
        until(lambda: self.events[-1]['reason'] == 'deadline')
    def test_followup_waits_for_playback_duration_and_keep_disables_idle(self):
        self.ready()
        self.socket.event('session.output_audio.delta', delta=base64.b64encode(b'\x00\x00'*24000).decode())
        until(lambda: self.audio[0].played)
        self.now += 12
        time.sleep(.15)
        self.assertFalse(self.audio[0].stopped)
        self.now += 2
        until(lambda: self.audio[0].stopped)
    def test_usage_is_cumulative_and_error_redacts_provider_message(self):
        self.ready()
        self.socket.event('session.usage.updated', usage={'seconds':12})
        self.socket.event('session.usage.updated', usage={'seconds':15})
        self.socket.event('error', error={'message':'secret'})
        until(lambda: self.audio[0].stopped)
        until(lambda: self.events[-1]['reason'] == 'provider_error')
        self.assertEqual(self.events[-1]['usage_seconds'], 15)
        self.assertNotIn('secret', json.dumps(self.events))
        self.assertFalse(self.events[-1]['finalized'])
    def test_muting_stops_local_capture_and_resume_is_explicit(self):
        self.ready()
        self.voice.mute(True)
        self.assertTrue(self.audio[0].muted)
        before = len(self.socket.sent)
        self.audio[0].capture(b'ab')
        time.sleep(.05)
        self.assertEqual(len(self.socket.sent), before)
        self.voice.mute(False)
        self.assertFalse(self.audio[0].muted)
    def test_replaced_session_cannot_emit_stale_state_or_transcripts(self):
        self.ready()
        old = self.voice.session
        replacement = Socket()
        self.voice.connect = lambda key: replacement
        self.voice.start([], 'new identity', 'secret', 'gpt-5.6-luna')
        until(lambda: replacement.sent)
        count = len(self.events)
        self.voice._receive(old, {'type':'session.closed', 'usage':{'seconds':99}})
        self.assertEqual(len(self.events), count)
        replacement.event('session.closed', usage={'seconds':0})

    def test_keep_disables_idle_but_does_not_restart_closed_session(self):
        self.ready()
        self.voice.keep()
        self.now += 60
        time.sleep(.15)
        self.assertFalse(self.audio[0].stopped)
        self.voice.stop('user')
        self.voice.keep()
        self.assertTrue(self.audio[0].stopped)

    def test_backend_usage_deduplicates_completions_and_tracks_pending_work(self):
        self.ready()
        self.socket.event('response.event', event={'type':'response.created','response':{'id':'r1'}})
        until(lambda: self.voice.session.pending_responses)
        self.now += 40
        time.sleep(.15)
        self.assertFalse(self.audio[0].stopped)
        event = {'type':'response.completed','response':{'id':'r1','usage':{'total_tokens':12}}}
        self.socket.event('response.event', event=event)
        self.socket.event('response.event', event=event)
        until(lambda: self.voice.session.backend_usage)
        time.sleep(.05)
        self.assertEqual(self.voice.session.backend_usage['total_tokens'],12)

    def test_late_started_after_stop_never_opens_microphone(self):
        self.start()
        self.voice.stop('parent_closed')
        self.socket.event('session.started', session={'id':'late'})
        time.sleep(.1)
        self.assertFalse(self.audio)

    def test_blocked_ui_callback_cannot_prevent_deadline_cleanup(self):
        gate = threading.Event()
        entered = threading.Event()
        def emit(event):
            if event['state'] == 'listening':
                entered.set()
                gate.wait(3)
            self.events.append(event)
        self.voice.emit = emit
        self.ready()
        until(entered.is_set)
        try:
            self.now += 300
            until(lambda: self.audio[0].stopped)
        finally:
            gate.set()

    @unittest.skipUnless(sys.platform == 'linux', 'Linux parent-death lifecycle')
    def test_audio_helper_dies_when_parent_is_killed(self):
        path = str(Path(__file__).with_name('voice.py').resolve())
        code = ("import os, subprocess, sys, time; "
                "p=subprocess.Popen([sys.executable,sys.argv[1],'--audio-child',str(os.getpid()),'/bin/sleep','60']); "
                "print(p.pid,flush=True); time.sleep(60)")
        parent = subprocess.Popen([sys.executable, '-c', code, path], stdout=subprocess.PIPE, text=True)
        pid = int(parent.stdout.readline())
        try:
            until(lambda: Path('/proc', str(pid), 'comm').read_text().strip() == 'sleep')
            parent.kill()
            parent.wait(timeout=1)
            def dead():
                try: return Path('/proc',str(pid),'stat').read_text().split()[2] == 'Z'
                except FileNotFoundError: return True
            until(dead)
        finally:
            parent.kill()
            parent.wait(timeout=1)
            parent.stdout.close()
            try: os.kill(pid, 9)
            except ProcessLookupError: pass

    def test_stalled_audio_sender_cannot_prevent_deadline_cleanup(self):
        self.ready()
        gate = threading.Event()
        entered = threading.Event()
        original = self.socket.send
        def send(raw):
            if json.loads(raw)['type'] == 'session.input_audio.append':
                entered.set()
                gate.wait(3)
            original(raw)
        self.socket.send = send
        self.audio[0].capture(b'ab')
        until(entered.is_set)
        try:
            self.voice.keep()
            self.now += 300
            until(lambda: self.audio[0].stopped)
        finally:
            gate.set()

    def test_audio_backlog_is_bounded_and_overflow_stops_session(self):
        self.ready()
        gate = threading.Event()
        original = self.socket.send
        def send(raw):
            if json.loads(raw)['type'] == 'session.input_audio.append': gate.wait(2)
            original(raw)
        self.socket.send = send
        try:
            for _ in range(100): self.audio[0].capture(b'ab')
            until(lambda: self.audio[0].stopped)
            until(lambda: self.events[-1]['reason'] == 'audio_overflow')
            self.assertLessEqual(self.voice.session.outgoing.qsize(), 40)
        finally:
            gate.set()

    def test_terminal_usage_must_be_valid_before_finalized_accounting(self):
        for usage in (None, {}, {'seconds':-1}, {'seconds':'12'}, {'seconds':True}, {'seconds':float('nan')}):
            with self.subTest(usage=usage):
                self.socket = Socket()
                self.voice.connect = lambda key: self.socket
                self.ready()
                self.socket.event('session.usage.updated', usage={'seconds':12})
                until(lambda: self.voice.session.usage == 12)
                if usage is None:
                    self.socket.event('session.closed', reason='connection_lost')
                else:
                    self.socket.event('session.closed', usage=usage, reason='expired')
                until(lambda: self.voice.session.done.is_set())
                until(lambda: self.events[-1]['state'] == 'stopped')
                self.assertFalse(self.events[-1]['finalized'])
                self.assertEqual(self.events[-1]['usage_seconds'], 12)

    def test_accepted_transcript_drains_before_terminal_event(self):
        gate = threading.Event()
        entered = threading.Event()
        delivered = []
        def emit(event):
            if event['state'] == 'listening':
                entered.set()
                gate.wait(2)
            delivered.append(('event',event['state']))
        self.voice.emit = emit
        self.voice.transcript = lambda role,text: delivered.append((role,text))
        self.ready()
        until(entered.is_set)
        try:
            self.socket.event('session.output_transcript.delta', delta='final utterance')
            self.socket.event('session.closed', usage={'seconds':3})
            until(lambda: self.voice.session.done.is_set())
        finally:
            gate.set()
        until(lambda: ('event','stopped') in delivered)
        self.assertIn(('assistant','final utterance'), delivered)
        self.assertLess(delivered.index(('assistant','final utterance')), delivered.index(('event','stopped')))

    def test_timeout_marks_finalization_unconfirmed(self):
        self.ready()
        self.voice.stop('user')
        self.now += 3
        until(lambda: self.socket.closed)
        until(lambda: self.events[-1]['state'] == 'stopped')
        self.assertFalse(self.events[-1]['finalized'])
        self.assertEqual(self.events[-1]['state'], 'stopped')

if __name__ == '__main__': unittest.main()

class ConnectionDiagnosticTests(unittest.TestCase):
    def failed_connection(self, error):
        events, devices = [], []
        def connect(key):
            raise error
        voice = LiveVoice(events.append, lambda *args: None, connect=connect,
            audio_factory=lambda *args: devices.append(args))
        voice.start([], '', 'private-api-key')
        until(lambda: events and events[-1]['state'] == 'stopped')
        self.assertFalse(devices)
        encoded = json.dumps(events)
        self.assertNotIn('private-api-key', encoded)
        self.assertNotIn('private-provider-body', encoded)
        self.assertTrue(events[-1]['finalized'])
        self.assertEqual(events[-1]['usage_seconds'], 0)
        return events[-1]

    def test_handshake_status_gives_safe_action_without_headers_or_body(self):
        import websocket
        for status, action in [(401, 'API key'), (403, 'access'), (404, 'endpoint'),
                               (429, 'billing'), (503, 'try again'), (418, 'HTTP 418')]:
            with self.subTest(status=status):
                error = websocket.WebSocketBadStatusException('private-api-key', status,
                    resp_headers={'Authorization':'private-api-key'}, resp_body='private-provider-body')
                event = self.failed_connection(error)
                self.assertIn(action, event['message'])
                self.assertIn(str(status), event['message'])

    def test_local_dependency_network_tls_and_timeout_have_distinct_actions(self):
        import socket
        import ssl
        import websocket
        for error, action in [
                (ModuleNotFoundError('private-api-key'), 'python3-websocket'),
                (ssl.SSLError('private-api-key'), 'certificate'),
                (socket.gaierror('private-api-key'), 'DNS'),
                (websocket.WebSocketAddressException('private-api-key'), 'DNS'),
                (TimeoutError('private-api-key'), 'timed out'),
                (websocket.WebSocketTimeoutException('private-api-key'), 'timed out'),
                (OSError('private-api-key'), 'connection'),
                (ValueError('private-provider-body'), 'connection')]:
            with self.subTest(error=type(error).__name__):
                self.assertIn(action, self.failed_connection(error)['message'])

    def test_uncertain_start_send_failure_keeps_reservation_unconfirmed(self):
        events = []
        class FailingSend(Socket):
            def send(self, raw):
                raise OSError('private-api-key')
        voice = LiveVoice(events.append, lambda *args: None, connect=lambda key: FailingSend())
        voice.start([], '', 'private-api-key')
        until(lambda: events and events[-1]['state'] == 'stopped')
        self.assertFalse(events[-1]['finalized'])
        self.assertNotIn('private-api-key', json.dumps(events))

    def test_connection_drop_after_start_keeps_reservation_unconfirmed(self):
        events = []
        class DropAfterStart(Socket):
            def recv(self):
                raise OSError('private-api-key')
        voice = LiveVoice(events.append, lambda *args: None, connect=lambda key: DropAfterStart())
        voice.start([], '', 'private-api-key')
        until(lambda: events and events[-1]['state'] == 'stopped')
        self.assertFalse(events[-1]['finalized'])
        self.assertNotIn('private-api-key', json.dumps(events))

class ConnectionAllowanceTests(unittest.TestCase):
    def test_prestart_failure_releases_allowance_through_service(self):
        import tempfile
        from service import Service, Store
        with tempfile.TemporaryDirectory() as temp:
            events = []
            def connect(key):
                raise TimeoutError('private-api-key')
            def factory(emit, transcript):
                return LiveVoice(emit, transcript, connect=connect)
            service = Service(Store(Path(temp)/'state'), events.append,
                              environment_key='private-api-key', voice_factory=factory)
            try:
                service.command({'type':'configure', 'enabled':True})
                service.command({'type':'voice_start'})
                def stopped():
                    service.pump()
                    return not service.voice_active
                until(stopped)
                self.assertEqual(service.store.usage(), (1, 0))
                terminal = [event for event in events if event.get('state') == 'stopped'][-1]
                self.assertTrue(terminal['finalized'])
                self.assertIn('timed out', terminal['message'])
                self.assertNotIn('private-api-key', json.dumps(events))
            finally:
                service.close()

class TransportTimeoutTests(unittest.TestCase):
    def test_handshake_gets_ten_seconds_then_receive_uses_one_second(self):
        from types import SimpleNamespace
        from unittest.mock import patch
        from companion.voice import _connect
        class Transport:
            def settimeout(self, timeout):
                self.timeout = timeout
        transport = Transport()
        calls = []
        def create_connection(url, **kwargs):
            calls.append((url, kwargs))
            return transport
        with patch.dict(sys.modules, {'websocket': SimpleNamespace(create_connection=create_connection)}):
            result = _connect('test-key')
        self.assertIs(result, transport)
        self.assertEqual(calls[0][1]['timeout'], 10)
        self.assertEqual(transport.timeout, 1)
        self.assertEqual(calls[0][1]['redirect_limit'], 0)
