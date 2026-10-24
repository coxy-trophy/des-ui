import json
from pathlib import Path
import tempfile
import threading
import time
import unittest
from service import Service, Store

class FakeKeyring:
    def __init__(self, key=''):
        self.key = key
        self.gate = None
        self.failure = False
    def perform(self, operation, key=''):
        if self.gate: self.gate.wait(2)
        if self.failure: raise RuntimeError('secret-provider-payload')
        if operation == 'load': return self.key
        self.key = key if operation == 'save' else ''
        return ''

class CredentialTests(unittest.TestCase):
    def settle(self, service):
        deadline = time.monotonic()+3
        while service.credential_loading and time.monotonic()<deadline:
            service.pump()
            time.sleep(.005)
        self.assertFalse(service.credential_loading)
    def test_restart_save_remove_and_no_disk_or_state_secret(self):
        with tempfile.TemporaryDirectory() as root:
            backend = FakeKeyring()
            events = []
            service = Service(Store(root), events.append, environment_key='', credential_store=backend)
            self.settle(service)
            service.command({'type':'configure','api_key':'sk-test-private','persist_key':True})
            self.settle(service)
            self.assertEqual(backend.key, 'sk-test-private')
            service.close()
            service = Service(Store(root), events.append, environment_key='', credential_store=backend)
            self.settle(service)
            self.assertEqual(service.key, 'sk-test-private')
            service.snapshot()
            self.assertNotIn('sk-test-private', json.dumps(events))
            self.assertNotIn(b'sk-test-private', Path(root,'companion.sqlite3').read_bytes())
            service.command({'type':'remove_saved_key'})
            self.settle(service)
            self.assertEqual(backend.key, '')
            self.assertEqual(service.key, '')
            service.close()
    def test_stale_load_cannot_replace_new_key_and_failure_is_safe(self):
        with tempfile.TemporaryDirectory() as root:
            backend = FakeKeyring('old-key')
            backend.gate = threading.Event()
            events = []
            service = Service(Store(root), events.append, environment_key='', credential_store=backend)
            service.command({'type':'configure','api_key':'new-key','persist_key':True})
            backend.gate.set()
            self.settle(service)
            self.assertEqual(service.key, 'new-key')
            self.assertEqual(backend.key, 'new-key')
            backend.failure = True
            service.command({'type':'configure','persist_key':True})
            self.settle(service)
            self.assertEqual(service.credential_status, 'unavailable')
            self.assertNotIn('secret-provider-payload', json.dumps(events))
            service.close()

    def test_remove_failure_does_not_reload_saved_key_on_restart(self):
        with tempfile.TemporaryDirectory() as root:
            backend = FakeKeyring('old-key')
            service = Service(Store(root), lambda event:None, environment_key='', credential_store=backend)
            self.settle(service)
            backend.failure = True
            service.command({'type':'remove_saved_key'})
            self.settle(service)
            service.close()
            backend.failure = False
            service = Service(Store(root), lambda event:None, environment_key='', credential_store=backend)
            self.settle(service)
            self.assertEqual(service.key, '')
            service.close()

    def test_environment_precedence_and_nonblocking_load_timeout(self):
        with tempfile.TemporaryDirectory() as root:
            backend = FakeKeyring('saved-key')
            backend.gate = threading.Event()
            now = [0]
            service = Service(Store(root), lambda event:None, environment_key='environment-key',
                              credential_store=backend, clock=lambda:now[0])
            self.assertTrue(service.credential_loading)
            self.assertEqual(service.key, 'environment-key')
            service.command({'type':'status'})
            now[0] = 11
            service.pump()
            self.assertFalse(service.credential_loading)
            self.assertEqual(service.credential_status, 'unavailable')
            backend.gate.set()
            time.sleep(.02)
            service.pump()
            self.assertEqual(service.key, 'environment-key')
            self.assertEqual(service.credential_status, 'unavailable')
            service.close()
