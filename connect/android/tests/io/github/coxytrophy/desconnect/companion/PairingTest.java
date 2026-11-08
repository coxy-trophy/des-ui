package io.github.coxytrophy.desconnect.companion;

public final class PairingTest {
    private static final long WALL = 1_800_000_000L;

    public static void main(String[] args) {
        unsolicitedAcceptanceDoesNotPair();
        staleApprovalCannotAcceptReplacement();
        expiredRequestCannotBeApproved();
        concurrentRequestCannotAuthorizeOutgoingPairing();
        delayedReplyCannotAuthorizeARetryOnTheSameSocket();
        skewedIncomingRequestCannotBeApproved();
        outgoingRequiresLocalCodeConfirmation();
        unpairCannotReuseAConnectionForAnotherPairing();
        System.out.println("Pairing regression checks passed");
    }

    private static void unsolicitedAcceptanceDoesNotPair() {
        Pairing pair = new Pairing(false);
        expect(!pair.peerAccepts(100), "unsolicited acceptance must not create trust");
        expect(pair.state() == Pairing.State.UNPAIRED, "unsolicited acceptance must leave the device unpaired");
    }

    private static void staleApprovalCannotAcceptReplacement() {
        Pairing pair = new Pairing(false);
        expect(pair.incoming(WALL, WALL, 100), "first request must become pending");
        long stale = pair.attempt();
        expect(pair.cancel(stale, 101), "current request must be cancellable");
        expect(pair.incoming(WALL + 1, WALL, 102), "replacement request must become pending");
        expect(!pair.canAccept(stale, 103), "approval of the first request must not authorize its replacement");
        expect(pair.canAccept(pair.attempt(), 103), "approval of the replacement must be valid");
        Pairing other = new Pairing(false);
        expect(other.incoming(WALL, WALL, 103), "request after reconnect must become pending");
        expect(!other.canAccept(pair.attempt(), 104), "an approval must not survive reconnect");
    }

    private static void expiredRequestCannotBeApproved() {
        Pairing pair = new Pairing(false);
        expect(pair.incoming(WALL, WALL, 100), "request must become pending");
        long attempt = pair.attempt();
        expect(!pair.canAccept(attempt, 25_100), "an approval cannot be used after the deadline");
    }

    private static void concurrentRequestCannotAuthorizeOutgoingPairing() {
        Pairing pair = new Pairing(false);
        expect(pair.outgoing(WALL, 100), "local request must become pending");
        expect(!pair.incoming(WALL, WALL, 101), "a second request must not replace a pending outgoing request");
        expect(pair.state() == Pairing.State.OUTGOING, "concurrent request must not create trust");
        expect(!pair.peerAccepts(102), "a fresh timestamp-free acceptance alone cannot create trust");
    }

    private static void delayedReplyCannotAuthorizeARetryOnTheSameSocket() {
        Pairing pair = new Pairing(false);
        expect(pair.outgoing(WALL, 100), "local request must become pending");
        expect(pair.cancel(pair.attempt(), 101), "current outgoing request must be cancellable");
        expect(pair.mustReconnect(), "cancelling an outgoing request must require a fresh socket");
        expect(!pair.outgoing(WALL, 102), "an outgoing attempt must not be restarted on this socket");
        expect(!pair.peerAccepts(103), "delayed acceptance must not be valid");
        Pairing timed = new Pairing(false);
        expect(timed.outgoing(WALL, 100), "local request must become pending");
        expect(!timed.peerAccepts(30_100), "acceptance at the deadline must fail");
        expect(timed.mustReconnect(), "an expired outgoing request must require a fresh socket");
    }

    private static void skewedIncomingRequestCannotBeApproved() {
        Pairing pair = new Pairing(false);
        expect(!pair.incoming(Long.MAX_VALUE, WALL, 100), "extreme timestamps must fail");
        expect(!pair.incoming(WALL - 1801, WALL, 101), "request outside the clock window must fail");
        expect(pair.incoming(WALL - 1800, WALL, 102), "the endpoint of the allowed clock window must work");
    }

    private static void outgoingRequiresLocalCodeConfirmation() {
        Pairing pair = new Pairing(false);
        expect(pair.outgoing(WALL, 100), "local request must become pending");
        expect(!pair.peerAccepts(101), "desktop acceptance alone must not pair");
        expect(pair.state() == Pairing.State.OUTGOING, "desktop acceptance must leave the code available");
        expect(!pair.confirmOutgoing(pair.attempt() + 1, 102), "confirmation of another attempt must fail");
        expect(pair.confirmOutgoing(pair.attempt(), 103), "current confirmation after acknowledgment must pair");
        expect(pair.state() == Pairing.State.PAIRED, "both confirmations must create trust");

        Pairing reverse = new Pairing(false);
        expect(reverse.outgoing(WALL, 100), "local request must become pending");
        expect(!reverse.confirmOutgoing(reverse.attempt(), 101), "local confirmation alone must not pair");
        expect(reverse.peerAccepts(102), "desktop acknowledgment after local confirmation must pair");
    }

    private static void unpairCannotReuseAConnectionForAnotherPairing() {
        Pairing pair = new Pairing(true);
        pair.unpair();
        expect(pair.mustReconnect(), "unpairing must end the previously authorized session");
        expect(!pair.outgoing(WALL, 101), "old socket must not accept a fresh outgoing pairing attempt");
        expect(!pair.peerAccepts(102), "old socket must not accept a delayed reply");
    }

    private static void expect(boolean value, String message) {
        if (!value) throw new AssertionError(message);
    }
}
