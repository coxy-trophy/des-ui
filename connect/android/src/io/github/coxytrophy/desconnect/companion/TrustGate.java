package io.github.coxytrophy.desconnect.companion;

import java.io.IOException;

/** Serializes durable pin changes with closing and replacing the foreground link. */
final class TrustGate {
    interface Write { void run() throws IOException; }

    private static final Object LOCK = new Object();
    private static volatile TrustGate current;

    TrustGate() {
        synchronized (LOCK) {
            current = this;
        }
    }

    boolean isCurrent() { return current == this; }

    boolean mutate(Write operation) throws IOException {
        synchronized (LOCK) {
            if (!isCurrent()) return false;
            operation.run();
            return true;
        }
    }

    void close() {
        synchronized (LOCK) {
            if (isCurrent()) current = null;
        }
    }
}
