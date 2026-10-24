"""System Secret Service credentials; never create an application key file."""
import queue
import threading

class CredentialError(Exception):
    pass

class SecretService:
    def perform(self, operation, key=''):
        try:
            import gi
            gi.require_version('Secret', '1')
            from gi.repository import Gio, Secret
        except (ImportError, ValueError):
            raise CredentialError('unavailable') from None
        schema = Secret.Schema.new('org.des-ui.Companion', Secret.SchemaFlags.NONE,
                                   {'service': Secret.SchemaAttributeType.STRING})
        attributes = {'service': 'openai-api-key'}
        cancel = Gio.Cancellable()
        timeout = threading.Timer(8, cancel.cancel)
        timeout.daemon = True
        timeout.start()
        try:
            if operation == 'load':
                service = Secret.Service.get_sync(Secret.ServiceFlags.OPEN_SESSION, cancel)
                items = service.search_sync(schema, attributes, Secret.SearchFlags.ALL, cancel)
                if not items:
                    return ''
                item = items[0]
                if item.get_locked():
                    raise CredentialError('locked')
                item.load_secret_sync(cancel)
                value = item.get_secret()
                return value.get_text() if value else ''
            if operation == 'save':
                if not Secret.password_store_sync(schema, attributes, Secret.COLLECTION_DEFAULT,
                                                  'DEs-UI companion OpenAI API key', key, cancel):
                    raise CredentialError('unavailable')
            else:
                Secret.password_clear_sync(schema, attributes, cancel)
            return ''
        except CredentialError:
            raise
        except Exception as error:
            # Never forward arbitrary provider messages or D-Bus payloads.
            if hasattr(error, 'matches') and error.matches(Secret.error_get_quark(), Secret.Error.IS_LOCKED):
                raise CredentialError('locked') from None
            raise CredentialError('unavailable') from None
        finally:
            timeout.cancel()

class CredentialWorker:
    """One daemon and bounded pending commands; all results applied on main loop."""
    def __init__(self, backend, results):
        self.backend, self.results = backend, results
        self.jobs = queue.Queue(maxsize=8)
        self.closed = threading.Event()
        threading.Thread(target=self.run, daemon=True, name='companion-credentials').start()

    def submit(self, generation, operation, key=''):
        try:
            self.jobs.put_nowait((generation, operation, key))
            return True
        except queue.Full:
            return False

    def run(self):
        while not self.closed.is_set():
            try:
                generation, operation, key = self.jobs.get(timeout=.2)
            except queue.Empty:
                continue
            try:
                result = self.backend.perform(operation, key)
                status = 'saved' if operation == 'save' or result else 'not_saved'
            except Exception as error:
                result = ''
                status = str(error) if type(error) is CredentialError else 'unavailable'
            key = ''
            if not self.closed.is_set():
                try:
                    self.results.put_nowait(('credential', (generation, operation, result, status)))
                except queue.Full:
                    pass
            result = ''

    def close(self):
        self.closed.set()
        while True:
            try: self.jobs.get_nowait()
            except queue.Empty: break
