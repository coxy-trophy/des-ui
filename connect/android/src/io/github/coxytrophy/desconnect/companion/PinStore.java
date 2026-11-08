package io.github.coxytrophy.desconnect.companion;

import android.content.Context;
import android.util.AtomicFile;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileNotFoundException;
import java.io.FileOutputStream;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.cert.CertificateException;
import java.security.cert.X509Certificate;

final class PinStore {
    static final class Pin {
        final String id;
        final String fingerprint;

        Pin(String id, String fingerprint) {
            this.id = id;
            this.fingerprint = fingerprint;
        }

        void verify(X509Certificate cert) throws GeneralSecurityException {
            if (!id.equals(PeerIdentity.certificateId(cert))
                    || !fingerprint.equals(PeerIdentity.fingerprint(cert)))
                throw new CertificateException("Desktop identity changed. Forget the saved desktop before pairing again.");
        }

        boolean matches(Pin other) {
            return other != null && id.equals(other.id) && fingerprint.equals(other.fingerprint);
        }
    }

    private static final Object LOCK = new Object();
    private final AtomicFile file;

    PinStore(Context context) {
        file = new AtomicFile(new File(context.getNoBackupFilesDir(), "desktop-pin"));
    }

    Pin read() throws IOException {
        synchronized (LOCK) {
            return readLocked();
        }
    }

    void remember(Pin pin) throws IOException {
        synchronized (LOCK) {
            Pin stored = readLocked();
            if (stored != null && !stored.matches(pin))
                throw new IOException("Forget the saved desktop before pairing another.");
            if (stored == null) writeLocked(pin.id + "\n" + pin.fingerprint);
        }
    }

    void forget(Pin expected) throws IOException {
        synchronized (LOCK) {
            if (expected != null && !expected.matches(readLocked()))
                throw new IOException("The saved desktop changed.");
            writeLocked("");
        }
    }

    private Pin readLocked() throws IOException {
        try (FileInputStream stream = file.openRead()) {
            byte[] data = new byte[104];
            int size = 0;
            while (size < data.length) {
                int read = stream.read(data, size, data.length - size);
                if (read < 0) break;
                size += read;
            }
            if (size == 0) return null;
            String[] parts = new String(data, 0, size, StandardCharsets.US_ASCII).split("\n", -1);
            if (size == data.length || parts.length != 2 || !PeerIdentity.validId(parts[0])
                    || !parts[1].matches("[0-9A-F]{64}"))
                throw new IOException("Saved desktop pin is unreadable. Forget it before pairing.");
            return new Pin(parts[0], parts[1]);
        } catch (FileNotFoundException missing) {
            if (file.getBaseFile().exists()) throw missing;
            return null;
        }
    }

    private void writeLocked(String contents) throws IOException {
        FileOutputStream stream = file.startWrite();
        try {
            stream.write(contents.getBytes(StandardCharsets.US_ASCII));
            stream.getFD().sync();
            file.finishWrite(stream);
        } catch (IOException | RuntimeException error) {
            file.failWrite(stream);
            throw error;
        }
        // AtomicFile logs rename failures instead of propagating them.
        Pin persisted = readLocked();
        String actual = persisted == null ? "" : persisted.id + "\n" + persisted.fingerprint;
        if (!contents.equals(actual)) throw new IOException("Could not update the saved desktop.");
    }
}
