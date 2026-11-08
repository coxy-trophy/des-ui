package io.github.coxytrophy.desconnect.companion;

import java.io.BufferedInputStream;
import java.io.ByteArrayOutputStream;
import java.io.EOFException;
import java.io.IOException;
import java.io.InputStream;
import java.net.ProtocolException;
import java.net.SocketTimeoutException;
import java.util.function.LongSupplier;

final class Frames {
    static final int MAX = 64 * 1024;
    private final InputStream input;
    private final LongSupplier clock;
    private final ByteArrayOutputStream partial = new ByteArrayOutputStream();
    private long started;

    Frames(InputStream input, LongSupplier clock) {
        this.input = new BufferedInputStream(input, 4096);
        this.clock = clock;
    }

    // null means an idle read timeout, not EOF or a frame boundary.
    byte[] read() throws IOException {
        try {
            while (true) {
                int b = input.read();
                if (b < 0) {
                    if (partial.size() != 0) throw new ProtocolException("Incomplete packet");
                    throw new EOFException();
                }
                checkDeadline();
                if (b == '\n') {
                    byte[] frame = partial.toByteArray();
                    partial.reset();
                    return frame;
                }
                if (partial.size() == MAX - 1) throw new ProtocolException("Packet exceeds 64 KiB");
                if (partial.size() == 0) started = clock.getAsLong();
                partial.write(b);
            }
        } catch (SocketTimeoutException idle) {
            checkDeadline();
            return null;
        }
    }

    private void checkDeadline() throws ProtocolException {
        if (partial.size() != 0 && clock.getAsLong() - started >= 5_000)
            throw new ProtocolException("Incomplete packet timed out");
    }
}
