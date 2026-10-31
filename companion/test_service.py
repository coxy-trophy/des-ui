import json
import os
from pathlib import Path
import tempfile
import threading
import sys
import time
import unittest

from service import Store, Service, Responses, validate_reply


class StoreTests(unittest.TestCase):
    def test_private_persistent_bounded_history_and_identity_rollback(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp) / 'state'
            store = Store(root)
            for i in range(250):
                store.message('user', str(i))
            store.identity('Chosen by me')
            store.identity('Changed my mind')
            store.remember('User-approved fact')
            store.close()
            store = Store(root)
            self.assertEqual(len(store.messages()), 200)
            self.assertEqual(store.messages()[-1]['text'], '249')
            self.assertEqual(store.get_identity(), 'Changed my mind')
            versions = store.identities()
            store.restore_identity(versions[-2]['id'])
            self.assertEqual(store.get_identity(), 'Chosen by me')
            self.assertEqual(store.memories()[0]['text'], 'User-approved fact')
            self.assertEqual(root.stat().st_mode & 0o777, 0o700)
            for file in root.iterdir():
                self.assertEqual(file.stat().st_mode & 0o777, 0o600)
            store.close()

    def test_symlink_store_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            target = Path(temp) / 'target'
            target.mkdir()
            link = Path(temp) / 'link'
            link.symlink_to(target)
            with self.assertRaises(ValueError):
                Store(link)

    def test_edit_memory_preserves_identity_and_validates_limits(self):
        with tempfile.TemporaryDirectory() as temp:
            store = Store(Path(temp) / 'state')
            store.remember('Old preference')
            ident = store.memories()[0]['id']
            store.edit_memory(ident, '  New preference  ')
            self.assertEqual(store.memories(), [{'id': ident, 'text': 'New preference'}])
            with self.assertRaises(ValueError): store.edit_memory(ident, ' ')
            with self.assertRaises(ValueError): store.edit_memory(ident + 100, 'Missing')
            with self.assertRaises(ValueError): store.edit_memory(ident, 'x' * 1001)
            self.assertEqual(store.memories()[0]['text'], 'New preference')
            store.close()

    def test_edit_memory_preserves_global_size_limit_atomically(self):
        with tempfile.TemporaryDirectory() as temp:
            store = Store(Path(temp) / 'state')
            for _ in range(31): store.remember('x' * 1000)
            store.remember('a' * 999)
            store.remember('z')
            ident = store.memories()[-2]['id']
            with self.assertRaises(ValueError): store.edit_memory(ident, 'b' * 1000)
            self.assertEqual(store.memories()[-2]['text'], 'a' * 999)
            store.edit_memory(ident, 'b' * 998)
            self.assertEqual(store.memories()[-2]['text'], 'b' * 998)
            store.close()

    def test_forget_and_clear_are_distinct_from_identity(self):
        with tempfile.TemporaryDirectory() as temp:
            store = Store(Path(temp) / 'state')
            store.identity('I choose my name')
            store.message('user', 'Private')
            store.remember('Fact')
            store.forget(store.memories()[0]['id'])
            store.clear()
            self.assertEqual(store.messages(), [])
            self.assertEqual(store.memories(), [])
            self.assertEqual(store.get_identity(), 'I choose my name')
            store.close()


class ValidationTests(unittest.TestCase):
    def test_model_cannot_change_permissions_or_write_unapproved_memories(self):
        with self.assertRaises(ValueError):
            validate_reply({'reply':'hi', 'identity':'', 'enabled':True})
        with self.assertRaises(ValueError):
            validate_reply({'reply':'hi', 'identity':'', 'memories':['secret']})
        self.assertEqual(validate_reply({'reply':'hi', 'identity':'I pick a name'}),
                         ('hi', 'I pick a name'))

    def test_provider_body_has_no_tools_or_stored_session(self):
        body = Responses.body('gpt-5.6-luna', 'instructions', [{'role':'user','text':'hello'}])
        self.assertFalse(body['store'])
        self.assertNotIn('tools', body)
        self.assertEqual(body['input'][0]['content'], 'hello')
        self.assertTrue(body['text']['format']['strict'])

    def test_image_is_attached_only_to_the_explicit_message(self):
        body = Responses.body('model', 'instructions', [
            {'role':'user', 'text':'earlier'},
            {'role':'user', 'text':'What is this?', 'image':'/9j/AA=='},
        ])
        self.assertEqual(body['input'][0]['content'], 'earlier')
        self.assertEqual(body['input'][1]['content'][0], {'type':'input_text','text':'What is this?'})
        self.assertEqual(body['input'][1]['content'][1]['image_url'], 'data:image/jpeg;base64,/9j/AA==')
        self.assertFalse(body['store'])

    def test_incomplete_and_refusal_are_not_identity_updates(self):
        for response in [{'status':'incomplete'}, {'status':'completed','output':[{'type':'message','content':[{'type':'refusal','refusal':'no'}]}]}]:
            with self.assertRaises(ValueError):
                Responses.extract(response)


class FakeProvider:
    def __init__(self):
        self.started = threading.Event()
        self.release = threading.Event()
        self.calls = []
    def request(self, key, model, instructions, context):
        self.calls.append((key, model, instructions, context))
        self.started.set()
        self.release.wait(2)
        return {'reply':'Reply', 'identity':'A self-chosen style'}


class ServiceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.events = []
        self.provider = FakeProvider()
        self.service = Service(Store(Path(self.temp.name)/'state'), self.events.append,
                               provider=self.provider, environment_key='test-only-key')
    def tearDown(self):
        self.provider.release.set()
        self.service.close()
        self.temp.cleanup()
    def command(self, **command):
        self.service.command(command)
    def pump_until(self, condition):
        deadline = time.monotonic()+2
        while not condition() and time.monotonic()<deadline:
            self.service.pump()
            time.sleep(.005)
        self.assertTrue(condition())
    def test_disabled_never_calls_api_and_keys_are_not_persisted_or_emitted(self):
        self.command(type='send',text='hello')
        self.assertEqual(self.provider.calls, [])
        self.command(type='configure',enabled=True,api_key='private-value')
        self.service.snapshot()
        self.assertNotIn('private-value', json.dumps(self.events))
        self.assertNotIn(b'private-value', self.service.store.path.read_bytes())
    def test_attached_image_is_not_saved_in_conversation_history(self):
        self.command(type='configure',enabled=True)
        self.command(type='send',text='What is this?', image='/9j/AA==')
        self.assertTrue(self.provider.started.wait(1))
        self.assertEqual(self.provider.calls[0][3][-1]['image'], '/9j/AA==')
        self.assertNotIn('/9j/AA==', json.dumps(self.service.store.messages()))
        self.assertNotIn(b'/9j/AA==', self.service.store.path.read_bytes())

    def test_invalid_attachment_never_starts_provider_or_saves_message(self):
        self.command(type='configure',enabled=True)
        self.command(type='send',text='Look',image='not-base64')
        self.assertEqual(self.provider.calls, [])
        self.assertEqual(self.service.store.messages(), [])

    def test_edit_memory_command_requires_an_existing_memory(self):
        self.command(type='remember',text='Old')
        ident = self.service.store.memories()[0]['id']
        self.command(type='edit_memory',memory_id=ident,text='New')
        self.assertEqual(self.service.store.memories(), [{'id':ident,'text':'New'}])
        self.command(type='edit_memory',memory_id=ident+1,text='Other')
        self.assertEqual(self.service.store.memories(), [{'id':ident,'text':'New'}])
        self.assertTrue(any(event.get('type')=='error' for event in self.events))

    def test_voice_screen_rejects_invalid_or_inactive_requests(self):
        self.command(type='voice_share_screen',image='bad')
        self.command(type='voice_share_screen',image='/9j/AA==')
        self.assertFalse(any(e.get('type') == 'voice_screen_queued' for e in self.events))
        self.assertEqual(self.service.store.messages(), [])
        self.assertNotIn(b'/9j/AA==', self.service.store.path.read_bytes())

    def test_cancel_discards_late_reply_and_identity(self):
        self.command(type='configure',enabled=True)
        self.command(type='send',text='hello')
        self.assertTrue(self.provider.started.wait(1))
        self.command(type='cancel')
        self.provider.release.set()
        self.pump_until(lambda:not self.service.worker_running)
        self.assertEqual(self.service.store.get_identity(), '')
        self.assertFalse(any(x.get('type')=='reply' for x in self.events))
    def test_success_preserves_shared_context_and_self_authored_identity(self):
        self.command(type='configure',enabled=True)
        self.command(type='remember',text='I prefer short replies')
        self.command(type='send',text='hello')
        self.provider.release.set()
        self.pump_until(lambda:not self.service.worker_running)
        self.assertEqual(self.service.store.messages()[-1]['text'], 'Reply')
        self.assertEqual(self.service.store.get_identity(), 'A self-chosen style')
        self.assertIn('I prefer short replies', self.provider.calls[0][2])
    def test_suppressed_social_and_repeated_ignored_approaches_are_bounded(self):
        self.command(type='configure',enabled=True,social=True)
        self.command(type='social',suppressed=True)
        self.assertEqual(self.provider.calls, [])
        self.command(type='social',suppressed=False)
        self.provider.release.set()
        self.pump_until(lambda:not self.service.worker_running)
        self.command(type='social',suppressed=False)
        self.assertEqual(len(self.provider.calls), 1)
    def test_daily_request_cap_applies_to_manual_and_social(self):
        self.command(type='configure',enabled=True,daily_requests=1,social=True)
        self.command(type='send',text='hello')
        self.provider.release.set()
        self.pump_until(lambda:not self.service.worker_running)
        self.command(type='send',text='again')
        self.assertEqual(len(self.provider.calls), 1)
        self.assertTrue(any('limit' in x.get('message','').lower() for x in self.events))

if __name__ == '__main__':
    unittest.main()

class FakeVoice:
    def __init__(self, emit, transcript):
        self.emit, self.transcript = emit, transcript
        self.stops = []
        self.shared_images = []
    def share_image(self, image):
        self.shared_images.append(image)
    def start(self,*args):
        self.args = args
    def stop(self,reason):
        self.stops.append(reason)
    def keep(self):
        pass
    def mute(self,value):
        pass


class VoiceIntegrationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.events = []
        self.service = Service(Store(Path(self.temp.name)/'state'),self.events.append,
                               environment_key='fake',voice_factory=FakeVoice)
        self.service.command({'type':'configure','enabled':True})
    def tearDown(self):
        self.service.close()
        self.temp.cleanup()
    def test_explicit_voice_screen_is_queued_without_being_saved(self):
        self.service.command({'type':'voice_start'})
        self.assertEqual(self.service.voice.shared_images, [])
        self.service.command({'type':'voice_share_screen', 'image':'/9j/AA=='})
        self.assertEqual(self.service.voice.shared_images, ['/9j/AA=='])
        self.assertTrue(any(event.get('type') == 'voice_screen_queued' for event in self.events))
        self.assertNotIn(b'/9j/AA==', self.service.store.path.read_bytes())
        self.service.command({'type':'voice_share_screen', 'image':'invalid'})
        self.assertEqual(self.service.voice.shared_images, ['/9j/AA=='])

    def test_transcript_persisted_before_connection_ends_and_clear_rejects_late_deltas(self):
        self.service.command({'type':'voice_start'})
        voice = self.service.voice
        voice.transcript('user','Hel')
        voice.transcript('user','lo')
        self.service.pump()
        self.assertEqual(self.service.store.messages()[-1]['text'],'Hello')
        self.service.command({'type':'clear'})
        voice.transcript('assistant','late')
        self.service.pump()
        self.assertEqual(self.service.store.messages(),[])
        self.assertIn('cancelled', voice.stops)
    def test_daily_allowance_reserved_and_final_usage_is_cumulative(self):
        self.service.command({'type':'voice_start'})
        self.assertEqual(self.service.store.usage(),(1,900))
        self.assertEqual(self.service.voice.args[-1], 900)
        self.service.voice.emit({'state':'listening','seconds':10})
        self.service.voice.emit({'state':'listening','seconds':15})
        self.service.voice.emit({'state':'stopped','finalized':True,'seconds':17})
        self.service.pump()
        self.assertEqual(self.service.store.usage(),(1,17))
    def test_unfinalized_session_keeps_conservative_reservation_and_no_auto_restart(self):
        self.service.command({'type':'voice_start'})
        old = self.service.voice
        old.emit({'state':'expired','finalized':False})
        self.service.pump()
        self.assertFalse(self.service.voice_active)
        self.assertEqual(self.service.store.usage(),(1,900))
        self.assertIs(self.service.voice,old)

    def test_remaining_allowance_below_five_minutes_can_be_used(self):
        self.service.store.charge(voice=840)
        self.service.command({'type':'voice_start'})
        self.assertTrue(self.service.voice_active)
        self.assertEqual(self.service.voice.args[-1], 60)
        self.assertEqual(self.service.store.usage(), (1, 900))

class ErrorBoundaryTests(unittest.TestCase):
    def test_arbitrary_provider_exception_cannot_echo_key(self):
        class BadProvider:
            def request(self,*args):
                raise ValueError('secret-key-from-provider-error')
        with tempfile.TemporaryDirectory() as temp:
            events=[]
            service=Service(Store(Path(temp)/'state'),events.append,provider=BadProvider(),environment_key='secret-key-from-provider-error')
            service.command({'type':'configure','enabled':True})
            service.command({'type':'send','text':'hello'})
            deadline=time.monotonic()+1
            while service.worker_running and time.monotonic()<deadline:
                service.pump()
                time.sleep(.005)
            self.assertNotIn('secret-key-from-provider-error',json.dumps(events))
            service.close()

class ProtocolTests(unittest.TestCase):
    def test_social_probe_uses_saved_opt_in_without_starting_service(self):
        import subprocess
        with tempfile.TemporaryDirectory() as temp:
            env = dict(os.environ, XDG_STATE_HOME=temp, DES_UI_COMPANION_DISABLE_KEYRING='1')
            def probe():
                return subprocess.run([sys.executable, str(Path(__file__).with_name('service.py')), '--social-probe'], env=env, capture_output=True, text=True, timeout=5)
            self.assertEqual(probe().stdout, 'no\n')
            store = Store(Path(temp)/'des-ui-companion')
            store.save_settings({'enabled': True, 'social': True})
            store.close()
            result = probe()
            self.assertEqual((result.returncode, result.stdout), (0, 'yes\n'))
            store = Store(Path(temp)/'des-ui-companion')
            store.save_settings({'enabled': False})
            store.close()
            self.assertEqual(probe().stdout, 'no\n')

    def test_worker_persists_user_state_and_never_emits_runtime_key(self):
        import subprocess
        with tempfile.TemporaryDirectory() as temp:
            environment=dict(os.environ,XDG_STATE_HOME=temp)
            environment.pop('OPENAI_API_KEY',None)
            environment['DES_UI_COMPANION_DISABLE_KEYRING'] = '1'
            commands=[{'type':'configure','api_key':'runtime-secret'},
                      {'type':'remember','text':'A user-approved preference'},
                      {'type':'identity','text':'A chosen identity'},
                      {'type':'shutdown'}]
            result=subprocess.run([sys.executable,str(Path(__file__).with_name('service.py'))],
                input='\n'.join(map(json.dumps,commands))+'\n',capture_output=True,text=True,env=environment,timeout=5)
            self.assertEqual(result.returncode,0,result.stderr)
            self.assertNotIn('runtime-secret',result.stdout+result.stderr)
            result=subprocess.run([sys.executable,str(Path(__file__).with_name('service.py'))],
                input='{"type":"status"}\n{"type":"shutdown"}\n',capture_output=True,text=True,env=environment,timeout=5)
            states=[json.loads(line) for line in result.stdout.splitlines() if json.loads(line).get('type')=='state']
            self.assertEqual(states[-1]['identity'],'A chosen identity')
            self.assertEqual(states[-1]['memories'][0]['text'],'A user-approved preference')
            self.assertFalse(states[-1]['configured'])

class SocialPersistenceTests(unittest.TestCase):
    def test_restart_does_not_repeat_an_unanswered_approach(self):
        with tempfile.TemporaryDirectory() as temp:
            root=Path(temp)/'state'
            provider=FakeProvider()
            provider.release.set()
            service=Service(Store(root),lambda e:None,provider=provider,environment_key='fake')
            service.command({'type':'configure','enabled':True,'social':True})
            service.command({'type':'social','suppressed':False})
            deadline=time.monotonic()+1
            while service.worker_running and time.monotonic()<deadline:
                service.pump()
                time.sleep(.005)
            self.assertEqual(len(provider.calls),1)
            service.close()
            provider=FakeProvider()
            service=Service(Store(root),lambda e:None,provider=provider,environment_key='fake')
            service.command({'type':'social','suppressed':False})
            self.assertEqual(provider.calls,[])
            service.close()

class CredentialChangeTests(unittest.TestCase):
    def test_replacing_key_cancels_old_request_without_writing_it(self):
        with tempfile.TemporaryDirectory() as temp:
            provider=FakeProvider()
            events=[]
            service=Service(Store(Path(temp)/'state'),events.append,provider=provider,environment_key='old-key')
            service.command({'type':'configure','enabled':True})
            service.command({'type':'send','text':'hello'})
            self.assertTrue(provider.started.wait(1))
            service.command({'type':'configure','api_key':'new-key'})
            provider.release.set()
            deadline=time.monotonic()+1
            while service.worker_running and time.monotonic()<deadline:
                service.pump()
                time.sleep(.005)
            self.assertFalse(any(e.get('type')=='reply' for e in events))
            self.assertEqual(service.store.get_identity(),'')
            service.close()

class TranscriptSpacingTests(unittest.TestCase):
    def test_exact_fragment_spacing_is_preserved_in_shared_history(self):
        with tempfile.TemporaryDirectory() as temp:
            service=Service(Store(Path(temp)/'state'),lambda e:None,environment_key='fake',voice_factory=FakeVoice)
            service.command({'type':'configure','enabled':True})
            service.command({'type':'voice_start'})
            for chunk in [' ', 'Hello ', 'world']:
                service.voice.transcript('user',chunk)
            service.pump()
            self.assertEqual(service.store.messages()[-1]['text'],' Hello world')
            service.close()

class ShutdownFailureTests(unittest.TestCase):
    def test_storage_failure_cannot_prevent_microphone_stop(self):
        with tempfile.TemporaryDirectory() as temp:
            service=Service(Store(Path(temp)/'state'),lambda e:None,environment_key='fake',voice_factory=FakeVoice)
            service.command({'type':'configure','enabled':True})
            service.command({'type':'voice_start'})
            voice=service.voice
            voice.transcript('user','Accepted before shutdown')
            def broken_store(*args,**kwargs):
                import sqlite3
                raise sqlite3.OperationalError('disk full')
            service.store.message=broken_store
            try:
                service.close()
            except Exception:
                pass
            self.assertIn('shutdown',voice.stops)
            self.assertTrue(service.closed)

class CloseTranscriptTests(unittest.TestCase):
    def test_ordinary_close_preserves_already_accepted_voice_fragments(self):
        with tempfile.TemporaryDirectory() as temp:
            service=Service(Store(Path(temp)/'state'),lambda e:None,environment_key='fake',voice_factory=FakeVoice)
            service.command({'type':'configure','enabled':True})
            service.command({'type':'voice_start'})
            service.voice.transcript('user','Last words before closing')
            service.command({'type':'cancel'})
            service.voice.emit({'state':'stopped','finalized':True,'usage_seconds':4})
            service.pump()
            self.assertEqual(service.store.messages()[-1]['text'],'Last words before closing')
            service.close()

class ConfirmedUsageTests(unittest.TestCase):
    def test_provider_final_usage_can_exceed_local_deadline(self):
        with tempfile.TemporaryDirectory() as temp:
            service=Service(Store(Path(temp)/'state'),lambda e:None,environment_key='fake',voice_factory=FakeVoice)
            service.command({'type':'configure','enabled':True})
            service.command({'type':'voice_start'})
            service.voice.emit({'state':'stopped','finalized':True,'usage_seconds':302})
            service.pump()
            self.assertEqual(service.store.usage(),(1,302))
            service.close()

class ProviderDiagnosticTests(unittest.TestCase):
    def request_error(self, failure):
        from unittest.mock import patch
        from service import PublicError
        with patch('service.urllib.request.OpenerDirector.open', side_effect=failure):
            with self.assertRaises(PublicError) as raised:
                Responses().request('diagnostic-secret', 'example-model', '', [])
        message = str(raised.exception)
        self.assertNotIn('diagnostic-secret', message)
        self.assertNotIn('untrusted-provider-message', message)
        return message

    def test_http_errors_offer_specific_actions_without_echoing_body(self):
        import io
        import urllib.error
        cases = [
            (401, None, 'API key'),
            (403, None, 'permission'),
            (404, 'model_not_found', 'model'),
            (429, 'insufficient_quota', 'billing'),
            (429, 'rate_limit_exceeded', 'Wait'),
            (400, 'model_not_found', 'model'),
            (400, None, 'settings'),
            (503, None, 'try again'),
            (418, 'diagnostic-secret', 'HTTP 418'),
        ]
        for status, code, expected in cases:
            with self.subTest(status=status, code=code):
                payload = json.dumps({'error': {'code':code, 'type':code,
                    'message':'untrusted-provider-message diagnostic-secret'}}).encode()
                failure = urllib.error.HTTPError('https://api.openai.com/v1/responses', status,
                    'diagnostic-secret', {}, io.BytesIO(payload))
                self.assertIn(expected, self.request_error(failure))

    def test_malformed_error_body_cannot_replace_safe_http_diagnostic(self):
        import io
        import urllib.error
        for payload in [b'not json diagnostic-secret', b'[]', b'{"error":"diagnostic-secret"}',
                        b'{"error":{"code":["diagnostic-secret"]}}', b'x'*70000]:
            failure = urllib.error.HTTPError('https://api.openai.com/v1/responses', 401,
                'diagnostic-secret', {}, io.BytesIO(payload))
            self.assertIn('API key', self.request_error(failure))

    def test_timeout_has_distinct_retry_guidance(self):
        import urllib.error
        for failure in [TimeoutError('diagnostic-secret'),
                        urllib.error.URLError(TimeoutError('diagnostic-secret'))]:
            self.assertIn('timed out', self.request_error(failure))

    def test_network_errors_remain_private(self):
        import urllib.error
        self.assertIn('connection', self.request_error(urllib.error.URLError('diagnostic-secret')))
