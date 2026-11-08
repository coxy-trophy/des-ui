// SPDX-License-Identifier: MIT
package io.github.coxytrophy.desconnect.companion;

import android.os.SystemClock;
import org.json.JSONObject;
import java.net.DatagramPacket;
import java.net.DatagramSocket;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.SocketTimeoutException;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Consumer;

/** LAN announcements are address hints only; Link verifies the actual peer. */
final class DesktopDiscovery implements AutoCloseable {
    static final class Desktop {
        final String name, host;
        final int port;
        Desktop(String name, String host, int port) {
            this.name = name; this.host = host; this.port = port;
        }
    }
    private static final int LIMIT = 8192;
    private static final long TTL_MS = 65_000;
    private static final class Entry {
        final Desktop desktop;
        final long seen;
        Entry(Desktop desktop, long seen) { this.desktop = desktop; this.seen = seen; }
    }
    private final Consumer<List<Desktop>> listener;
    private volatile boolean closed;
    private DatagramSocket socket;
    private boolean started;

    DesktopDiscovery(Consumer<List<Desktop>> listener) { this.listener = listener; }

    synchronized void start() {
        if (started || closed) return;
        started = true;
        new Thread(this::receive, "des-discovery").start();
    }

    @Override public synchronized void close() {
        closed = true;
        if (socket != null) socket.close();
    }

    private void receive() {
        Map<String, Entry> found = new LinkedHashMap<>();
        try (DatagramSocket datagrams = new DatagramSocket(null)) {
            synchronized (this) {
                if (closed) return;
                socket = datagrams;
            }
            datagrams.setReuseAddress(true);
            datagrams.bind(new InetSocketAddress(1716));
            datagrams.setSoTimeout(3000);
            byte[] buffer = new byte[LIMIT + 1];
            DatagramPacket packet = new DatagramPacket(buffer, buffer.length);
            long window = SystemClock.elapsedRealtime();
            int burst = 0;
            while (!closed) {
                boolean changed = false;
                try {
                    packet.setLength(buffer.length);
                    datagrams.receive(packet);
                    long at = SystemClock.elapsedRealtime();
                    if (at - window >= 1000) { window = at; burst = 0; }
                    if (++burst > 24) {
                        // Keep an untrusted broadcast flood off the UI and CPU hot path.
                        SystemClock.sleep(40);
                        continue;
                    }
                    Desktop desktop = parse(packet);
                    if (desktop != null) {
                        String key = desktop.host + "/" + desktop.port;
                        if (found.containsKey(key) || found.size() < 16) {
                            Entry prior = found.put(key, new Entry(desktop, SystemClock.elapsedRealtime()));
                            changed = prior == null || !prior.desktop.name.equals(desktop.name);
                        }
                    }
                } catch (SocketTimeoutException ignored) { /* Age out disappeared desktops. */ }
                long now = SystemClock.elapsedRealtime();
                changed |= found.values().removeIf(entry -> now - entry.seen > TTL_MS);
                if (changed && !closed) {
                    List<Desktop> result = new ArrayList<>();
                    for (Entry entry : found.values()) result.add(entry.desktop);
                    listener.accept(Collections.unmodifiableList(result));
                }
            }
        } catch (java.io.IOException ignored) {
            // The address field is the fallback if Android or another companion owns the port.
            if (!closed) listener.accept(Collections.emptyList());
        }
    }

    private static Desktop parse(DatagramPacket packet) {
        InetAddress address = packet.getAddress();
        byte[] ip = address.getAddress();
        if (!(address.isSiteLocalAddress() || address.isLinkLocalAddress() || address.isLoopbackAddress()
                || (ip.length == 16 && (ip[0] & 0xfe) == 0xfc))) return null;
        if (packet.getLength() > LIMIT) return null;
        try {
            WirePacket hint = WirePacket.parse(Arrays.copyOfRange(packet.getData(), packet.getOffset(),
                    packet.getOffset() + packet.getLength()));
            if (!WirePacket.IDENTITY.equals(hint.type)) return null;
            JSONObject body = hint.body;
            Object deviceId = body.opt("deviceId");
            if (!(deviceId instanceof String) || !PeerIdentity.validId((String) deviceId)) return null;
            String name = hint.desktopName((String) deviceId, "");
            Object field = body.opt("tcpPort");
            if (!(field instanceof Long)) return null;
            long port = (Long) field;
            return port >= 1716 && port <= 1764 ? new Desktop(name, address.getHostAddress(), (int) port) : null;
        } catch (Exception malformed) { return null; }
    }
}
