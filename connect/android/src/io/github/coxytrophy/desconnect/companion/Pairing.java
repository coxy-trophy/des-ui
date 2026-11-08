package io.github.coxytrophy.desconnect.companion;

import java.util.concurrent.atomic.AtomicLong;

/** Pairing decisions for one already authenticated TLS connection. */
final class Pairing {
    enum State { UNPAIRED, INCOMING, OUTGOING, PAIRED }

    private static final AtomicLong NEXT_ATTEMPT = new AtomicLong();
    private State state;
    private long attempt;
    private long timestamp;
    private long deadline;
    private boolean mustReconnect;
    private boolean localConfirmed;
    private boolean peerConfirmed;

    Pairing(boolean pinned) {
        state = pinned ? State.PAIRED : State.UNPAIRED;
    }

    State state() { return state; }
    long attempt() { return attempt; }
    long timestamp() { return timestamp; }
    boolean mustReconnect() { return mustReconnect; }
    boolean localConfirmed() { return localConfirmed; }
    boolean peerConfirmed() { return peerConfirmed; }

    boolean incoming(long timestamp, long wall, long now) {
        expire(now);
        if (state != State.UNPAIRED || mustReconnect || timestamp < 0 || wall < 0
                || Math.abs(timestamp - wall) > 1800) return false;
        begin(State.INCOMING, timestamp, now + 25_000);
        return true;
    }

    boolean outgoing(long timestamp, long now) {
        expire(now);
        if (state != State.UNPAIRED || mustReconnect) return false;
        begin(State.OUTGOING, timestamp, now + 30_000);
        return true;
    }

    private void begin(State next, long timestamp, long deadline) {
        state = next;
        attempt = NEXT_ATTEMPT.incrementAndGet();
        this.timestamp = timestamp;
        this.deadline = deadline;
    }

    boolean canAccept(long attempt, long now) {
        expire(now);
        return state == State.INCOMING && this.attempt == attempt;
    }

    boolean acceptSent(long attempt, long now) {
        if (!canAccept(attempt, now)) return false;
        clear(State.PAIRED);
        return true;
    }

    boolean peerAccepts(long now) {
        expire(now);
        if (state != State.OUTGOING) return false;
        peerConfirmed = true;
        return completeOutgoing();
    }

    boolean confirmOutgoing(long attempt, long now) {
        expire(now);
        if (state != State.OUTGOING || this.attempt != attempt) return false;
        localConfirmed = true;
        return completeOutgoing();
    }

    private boolean completeOutgoing() {
        if (!peerConfirmed || !localConfirmed) return false;
        clear(State.PAIRED);
        return true;
    }

    boolean cancel(long attempt, long now) {
        expire(now);
        if (this.attempt == 0 || this.attempt != attempt) return false;
        unpair();
        return true;
    }

    boolean expire(long now) {
        if (attempt == 0 || now < deadline) return false;
        unpair();
        return true;
    }

    void unpair() {
        // A v8 acceptance has no attempt ID. After cancelling an outgoing
        // request or ending a trusted session, a late reply could answer a retry.
        if (state == State.OUTGOING || state == State.PAIRED) mustReconnect = true;
        clear(State.UNPAIRED);
    }

    private void clear(State next) {
        state = next;
        attempt = 0;
        timestamp = 0;
        deadline = 0;
        localConfirmed = false;
        peerConfirmed = false;
    }
}
