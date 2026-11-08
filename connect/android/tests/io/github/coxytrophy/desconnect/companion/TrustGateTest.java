package io.github.coxytrophy.desconnect.companion;

public final class TrustGateTest {
    public static void main(String[] args) throws Exception {
        closedLinkCannotChangeTrust();
        newerLinkDisplacesTheOldLink();
        System.out.println("Trust lifecycle regression checks passed");
    }

    private static void closedLinkCannotChangeTrust() throws Exception {
        TrustGate link = new TrustGate();
        StringBuilder store = new StringBuilder("current pin");
        link.close();
        if (link.mutate(() -> store.setLength(0)) || !"current pin".contentEquals(store))
            throw new AssertionError("a closed link must not clear the pin");
    }

    private static void newerLinkDisplacesTheOldLink() throws Exception {
        TrustGate old = new TrustGate();
        TrustGate current = new TrustGate();
        StringBuilder store = new StringBuilder();
        if (!current.mutate(() -> store.append("new pin")))
            throw new AssertionError("the current link must be allowed to persist trust");
        if (old.mutate(() -> store.append("old pin")) || !"new pin".contentEquals(store))
            throw new AssertionError("a superseded link must not change trust");
    }
}
