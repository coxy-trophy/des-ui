package io.github.coxytrophy.desconnect.companion;

import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.os.BatteryManager;
import android.os.Build;
import android.os.SystemClock;
import android.util.Log;
import java.io.EOFException;
import java.io.IOException;
import java.net.InetAddress;
import java.net.InetSocketAddress;
import java.net.ProtocolException;
import java.net.Socket;
import java.net.SocketTimeoutException;
import java.security.cert.CertificateException;
import java.security.cert.X509Certificate;
import java.util.concurrent.ArrayBlockingQueue;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.ScheduledThreadPoolExecutor;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Consumer;
import javax.net.ssl.SSLSocket;
import org.json.JSONArray;
import org.json.JSONObject;

final class Link implements AutoCloseable {
    enum Phase { CONNECTING, UNPAIRED, INCOMING, OUTGOING, PAIRED, DISCONNECTED, ERROR }

    static final class View {
        final Phase phase;
        final String desktop, detail, code;
        final long attempt;
        final boolean savedDesktop, retryAction;

        View(Phase phase, String desktop, String detail, String code, long attempt) {
            this(phase, desktop, detail, code, attempt, false);
        }

        View(Phase phase, String desktop, String detail, String code, long attempt, boolean savedDesktop) {
            this(phase, desktop, detail, code, attempt, savedDesktop, false);
        }

        private View(Phase phase, String desktop, String detail, String code, long attempt, boolean savedDesktop,
                     boolean retryAction) {
            this.phase = phase;
            this.desktop = desktop;
            this.detail = detail;
            this.code = code;
            this.attempt = attempt;
            this.savedDesktop = savedDesktop;
            this.retryAction = retryAction;
        }
    }

    private enum Action { CONNECT, PAIR, ACCEPT, REJECT, PING, UNPAIR }

    private static final class Command {
        final Action action;
        final String host;
        final int port;
        final long attempt;
        final long created = now();

        Command(Action action, long attempt, String host, int port) {
            this.action = action;
            this.attempt = attempt;
            this.host = host;
            this.port = port;
        }

        boolean fresh() { return now() - created <= 4_000; }
    }

    private static final ScheduledThreadPoolExecutor DEADLINES = new ScheduledThreadPoolExecutor(1, task -> {
        Thread thread = new Thread(task, "DES Connect I/O deadline");
        thread.setDaemon(true);
        return thread;
    });

    static { DEADLINES.setRemoveOnCancelPolicy(true); }

    private final Context context;
    private final Consumer<View> listener;
    private final PinStore pins;
    final CastLink cast;
    private final Thread worker;
    private final ArrayBlockingQueue<Command> commands = new ArrayBlockingQueue<>(16);
    private final AtomicBoolean started = new AtomicBoolean();
    private final AtomicBoolean queueFull = new AtomicBoolean();
    private final Object lifecycle = new Object();
    private final TrustGate trust = new TrustGate();
    private Socket activeSocket;
    private boolean savedDesktop; // Link thread only

    // The listener is called on the link thread; the Activity owns UI dispatch.
    Link(Context context, Consumer<View> listener, Runnable castChanged) {
        this.context = context.getApplicationContext();
        this.listener = listener;
        this.pins = new PinStore(this.context);
        this.cast = new CastLink(this.context, castChanged);
        worker = new Thread(this::run, "DES Connect foreground link");
        worker.setDaemon(true);
        worker.start();
    }

    // One connection per Link. Reconnect with a fresh Link and TLS session.
    void connect(String host, int port) {
        if (!isClosed() && started.compareAndSet(false, true))
            enqueue(new Command(Action.CONNECT, 0, host, port));
    }

    void pair() { offer(Action.PAIR, 0); }
    void accept(long attempt) { offer(Action.ACCEPT, attempt); }
    void reject(long attempt) { offer(Action.REJECT, attempt); }
    void ping() { offer(Action.PING, 0); }
    void unpair() { offer(Action.UNPAIR, 0); }

    private void offer(Action action, long attempt) {
        if (!isClosed()) enqueue(new Command(action, attempt, null, 0));
    }

    private void enqueue(Command command) {
        if (!commands.offer(command)) {
            if (command.action == Action.CONNECT) started.set(false);
            queueFull.set(true);
        }
    }

    @Override public void close() {
        trust.close();
        cast.stop();
        Socket socket;
        synchronized (lifecycle) {
            socket = activeSocket;
            activeSocket = null;
        }
        abort(socket);
        commands.clear();
        worker.interrupt();
    }

    private void run() {
        try {
            savedDesktop = pins.read() != null;
            if (savedDesktop && !started.get()) publish(Phase.DISCONNECTED, "", "", "", 0);
        } catch (IOException corrupt) {
            savedDesktop = true;
            publish(Phase.ERROR, "", explain(corrupt), "", 0);
        }
        while (!isClosed()) {
            try {
                Command command = commands.take();
                if (isClosed()) return;
                if (queueFull.getAndSet(false) || !command.fresh()) {
                    publish(Phase.DISCONNECTED, "", "An action expired. Try again.", "", 0, true);
                    if (!command.fresh()) {
                        if (command.action == Action.CONNECT) started.set(false);
                        continue;
                    }
                }
                if (command.action == Action.CONNECT) {
                    connectSession(command.host, command.port);
                } else if (command.action == Action.UNPAIR) {
                    if (!trust.mutate(() -> pins.forget(null))) return;
                    savedDesktop = false;
                    publish(Phase.DISCONNECTED, "", "Saved desktop forgotten.", "", 0);
                }
            } catch (InterruptedException stopped) {
                return;
            } catch (Exception failure) {
                if (!isClosed()) Log.w("DESConnectPreview", "Foreground connection failed", failure);
                publish(Phase.ERROR, "", explain(failure), "", 0);
            }
        }
    }

    private void connectSession(String host, int port) throws Exception {
        publish(Phase.CONNECTING, "", "Verifying the desktop connection…", "", 0);
        InetAddress address = LocalAddress.parse(host);
        if (port < 1 || port > 65535) throw new IOException("Enter a port between 1 and 65535.");
        PhoneIdentity phone = PhoneIdentity.load();
        PinStore.Pin saved = pins.read();
        savedDesktop = saved != null;
        Socket raw = new Socket();
        synchronized (lifecycle) {
            if (isClosed()) {
                abort(raw);
                return;
            }
            activeSocket = raw;
        }
        SSLSocket tls = null;
        try {
            ScheduledFuture<?> deadline = abortAfter(raw, 8_000);
            Frames frames;
            X509Certificate cert;
            String peerId;
            String name;
            boolean canCast;
            try {
                raw.connect(new InetSocketAddress(address, port), 4_000);
                raw.setTcpNoDelay(true);
                raw.setSoTimeout(5_000);
                WirePacket.write(raw.getOutputStream(), WirePacket.IDENTITY, localIdentity(phone.id, false));

                // The TCP initiator is the TLS server in this protocol.
                tls = (SSLSocket) phone.tls(saved).getSocketFactory().createSocket(raw, host, port, false);
                tls.setUseClientMode(false);
                tls.setEnabledProtocols(new String[]{"TLSv1.2"});
                tls.setEnabledCipherSuites(new String[]{
                        "TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256", "TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384"
                });
                tls.setNeedClientAuth(true);
                tls.startHandshake();
                cert = (X509Certificate) tls.getSession().getPeerCertificates()[0];
                PeerIdentity.validate(cert);
                peerId = PeerIdentity.certificateId(cert);
                if (saved != null) saved.verify(cert);

                WirePacket.write(tls.getOutputStream(), WirePacket.IDENTITY, localIdentity(phone.id, true));
                tls.setSoTimeout(250);
                frames = new Frames(tls.getInputStream(), Link::now);
                byte[] first = null;
                while (!isClosed() && first == null) first = frames.read();
                if (isClosed()) return;
                WirePacket remote = WirePacket.parse(first);
                name = remote.desktopName(peerId, phone.id);
                canCast = remote.supports(CastLink.KIND);
                PinStore.Pin current = pins.read();
                if (current != null) current.verify(cert);
                if (saved != null && current == null)
                    throw new CertificateException("Desktop was forgotten during the connection.");
                saved = current;
                savedDesktop = saved != null;
            } finally {
                deadline.cancel(false);
            }
            PinStore.Pin candidate = new PinStore.Pin(peerId, PeerIdentity.fingerprint(cert));
            new Session(raw, tls, frames, phone.cert, cert, candidate, saved != null, name, canCast).run();
        } catch (EOFException disconnected) {
            publish(Phase.DISCONNECTED, "", "Desktop disconnected.", "", 0);
        } finally {
            cast.stop();
            abort(raw);
            if (tls != null) abort(tls);
            synchronized (lifecycle) {
                if (activeSocket == raw) activeSocket = null;
            }
        }
    }

    private JSONObject localIdentity(String id, boolean secure) throws Exception {
        String model = Build.MODEL == null ? "" : Build.MODEL.replaceAll("[\\p{Cntrl}]", "").trim();
        if (model.length() > 80) model = model.substring(0, 80);
        String name = model.isEmpty() ? "Android phone" : model;
        JSONArray incoming = new JSONArray().put(WirePacket.PING).put(WirePacket.BATTERY_REQUEST);
        JSONArray outgoing = new JSONArray().put(WirePacket.PING).put(WirePacket.BATTERY);
        if (secure) {
            incoming.put(CastLink.KIND);
            outgoing.put(CastLink.KIND);
        }
        return new JSONObject().put("deviceId", id).put("deviceName", name)
                .put("deviceType", "phone").put("protocolVersion", 8)
                .put("incomingCapabilities", incoming).put("outgoingCapabilities", outgoing);
    }

    private final class Session {
        private final Socket raw;
        private final SSLSocket tls;
        private final Frames frames;
        private final byte[] localKey, peerKey;
        private final PinStore.Pin candidate;
        private final Pairing pairing;
        private final String name;
        private final boolean canCast;
        private long unpairedUntil = now() + 90_000;
        private long lastBattery = -1_000;
        private long lastPing = -1_000;
        private String code = "";
        private boolean finished;

        Session(Socket raw, SSLSocket tls, Frames frames, X509Certificate phone, X509Certificate peer,
                PinStore.Pin candidate, boolean pinned, String name, boolean canCast) {
            this.raw = raw;
            this.tls = tls;
            this.frames = frames;
            this.localKey = phone.getPublicKey().getEncoded();
            this.peerKey = peer.getPublicKey().getEncoded();
            this.candidate = candidate;
            this.pairing = new Pairing(pinned);
            this.name = name;
            this.canCast = canCast;
        }

        void run() throws Exception {
            state(paired() ? "Secure connection restored." : "Desktop verified. Pair to connect.");
            if (paired()) battery();
            boolean castActive = false;
            while (!isClosed()) {
                if (expirePairing(now())) return;
                if (!paired() && pairing.state() == Pairing.State.UNPAIRED && now() >= unpairedUntil) {
                    publish(Phase.DISCONNECTED, name, "Pairing session expired. Connect again to retry.", "", 0);
                    return;
                }
                for (int i = 0; i < 4 && !isClosed(); i++) {
                    Command command = commands.poll();
                    if (command == null) break;
                    long tick = now();
                    if (expirePairing(tick)) return;
                    if (command.fresh()) command(command, tick);
                    else retryAction();
                    if (finished) return;
                    if (pairing.mustReconnect()) {
                        endOutgoing("Pairing cancelled. Connect again to retry.");
                        return;
                    }
                }
                if (isClosed()) return;
                if (queueFull.getAndSet(false)) retryAction();
                if (canCast && paired()) {
                    JSONObject castMessage = cast.poll();
                    if (castMessage != null) send(CastLink.KIND, castMessage);
                    boolean active = cast.view() != null;
                    if (active != castActive) {
                        tls.setSoTimeout(active ? 100 : 250);
                        castActive = active;
                    }
                }
                byte[] frame = frames.read();
                if (frame == null || isClosed()) continue;
                WirePacket packet = WirePacket.parse(frame);
                long tick = now();
                if (expirePairing(tick)) return;
                received(packet, tick);
                if (finished) return;
                if (pairing.mustReconnect()) {
                    endOutgoing("Pairing ended. Connect again to retry.");
                    return;
                }
            }
        }

        private boolean expirePairing(long tick) throws Exception {
            if (!pairing.expire(tick)) return false;
            if (pairing.mustReconnect()) {
                endOutgoing("Pairing expired. Connect again to retry.");
                return true;
            }
            code = "";
            sendPair(false, null);
            state("Pairing request expired.");
            return false;
        }

        private void command(Command command, long tick) throws Exception {
            switch (command.action) {
                case PAIR:
                    long wall = wall();
                    if (pairing.outgoing(wall, tick)) {
                        code = PeerIdentity.code(localKey, peerKey, wall);
                        sendPair(true, wall);
                        showOutgoing();
                    }
                    break;
                case ACCEPT:
                    if (pairing.canAccept(command.attempt, tick)) {
                        sendPair(true, null);
                        if (pairing.acceptSent(command.attempt, now())) remember();
                        else {
                            sendPair(false, null);
                            code = "";
                            state("Pairing request expired.");
                        }
                    } else if (pairing.state() == Pairing.State.OUTGOING && pairing.attempt() == command.attempt) {
                        if (pairing.confirmOutgoing(command.attempt, tick)) remember();
                        else if (!pairing.mustReconnect()) showOutgoing();
                    }
                    break;
                case REJECT:
                    if (pairing.cancel(command.attempt, tick) && !pairing.mustReconnect()) {
                        code = "";
                        sendPair(false, null);
                        state("Pairing request declined.");
                    }
                    break;
                case UNPAIR:
                    if (paired()) {
                        if (!forget()) return;
                        sendPair(false, null);
                        publish(Phase.DISCONNECTED, name, "Desktop forgotten.", "", 0);
                    }
                    break;
                case PING:
                    if (paired()) {
                        send(WirePacket.PING, new JSONObject().put("message", "DES Connect"));
                        state("Ping sent.");
                    }
                    break;
                default:
                    break;
            }
        }

        private void received(WirePacket packet, long tick) throws Exception {
            if (WirePacket.PAIR.equals(packet.type)) {
                Object wants = packet.body.opt("pair");
                if (!(wants instanceof Boolean)) throw new ProtocolException("Invalid pairing message");
                if (!((Boolean) wants)) {
                    boolean paired = paired();
                    if (paired) {
                        if (forget()) publish(Phase.DISCONNECTED, name, "Desktop unpaired.", "", 0);
                        return;
                    }
                    if (pairing.attempt() != 0) {
                        pairing.unpair();
                        unpairedUntil = now() + 90_000;
                        code = "";
                        if (!pairing.mustReconnect())
                            state("Desktop cancelled pairing.");
                    }
                    return;
                }
                Object stamp = packet.body.opt("timestamp");
                if (!packet.body.has("timestamp")) {
                    if (pairing.state() == Pairing.State.OUTGOING) {
                        boolean wasAcknowledged = pairing.peerConfirmed();
                        if (pairing.peerAccepts(tick)) remember();
                        else if (!wasAcknowledged && !pairing.mustReconnect()) showOutgoing();
                    }
                    // A timestamp-free acknowledgement cannot initiate pairing.
                } else if (!(stamp instanceof Long)) {
                    throw new ProtocolException("Invalid pairing timestamp");
                } else if (pairing.incoming((Long) stamp, wall(), tick)) {
                    code = PeerIdentity.code(localKey, peerKey, pairing.timestamp());
                    state("Compare this code on both devices before approving.");
                }
                return;
            }
            if (!paired()) return;
            if (canCast && CastLink.KIND.equals(packet.type)) {
                cast.received(packet.body);
                return;
            }
            if (WirePacket.PING.equals(packet.type) && now() - lastPing >= 1_000) {
                lastPing = now();
                state("Ping received.");
            } else if (WirePacket.BATTERY_REQUEST.equals(packet.type)
                    && Boolean.TRUE.equals(packet.body.opt("request"))
                    && now() - lastBattery >= 1_000) {
                battery();
            }
        }

        private void endOutgoing(String detail) throws Exception {
            sendPair(false, null);
            publish(Phase.DISCONNECTED, name, detail, "", 0);
        }

        private void showOutgoing() {
            if (pairing.localConfirmed()) state("Code confirmed. Waiting for desktop approval.");
            else if (pairing.peerConfirmed()) state("Desktop approved. Confirm that the codes match.");
            else state("Compare this code with the desktop before approving.");
        }

        private boolean forget() throws IOException {
            cast.stop();
            if (!trust.mutate(() -> pins.forget(candidate))) return false;
            savedDesktop = false;
            pairing.unpair();
            code = "";
            finished = true;
            return true;
        }

        private void remember() throws Exception {
            try {
                if (!trust.mutate(() -> pins.remember(candidate))) return;
                savedDesktop = true;
            } catch (IOException failure) {
                pairing.unpair();
                sendPair(false, null);
                throw failure;
            }
            code = "";
            state("Pairing complete.");
            battery();
        }

        private void battery() throws Exception {
            lastBattery = now();
            Intent state = context.registerReceiver(null, new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
            if (state == null) return;
            int level = state.getIntExtra(BatteryManager.EXTRA_LEVEL, -1);
            int scale = state.getIntExtra(BatteryManager.EXTRA_SCALE, -1);
            if (scale <= 0 || level < 0 || level > scale) return;
            int status = state.getIntExtra(BatteryManager.EXTRA_STATUS, BatteryManager.BATTERY_STATUS_UNKNOWN);
            boolean charging = status == BatteryManager.BATTERY_STATUS_CHARGING
                    || status == BatteryManager.BATTERY_STATUS_FULL;
            send(WirePacket.BATTERY, new JSONObject().put("currentCharge", (level * 100L) / scale)
                    .put("isCharging", charging).put("thresholdEvent", 0));
        }

        private void sendPair(boolean value, Long timestamp) throws Exception {
            JSONObject body = new JSONObject().put("pair", value);
            if (timestamp != null) body.put("timestamp", timestamp);
            send(WirePacket.PAIR, body);
        }

        private void send(String type, JSONObject body) throws Exception {
            if (isClosed()) throw new EOFException();
            ScheduledFuture<?> deadline = abortAfter(raw, 3_000);
            try {
                WirePacket.write(tls.getOutputStream(), type, body);
            } finally {
                deadline.cancel(false);
            }
        }

        private boolean paired() { return pairing.state() == Pairing.State.PAIRED; }

        private void retryAction() {
            publish(Phase.valueOf(pairing.state().name()), name, "An action expired. Try again.",
                    code, pairing.attempt(), true);
        }

        private void state(String detail) {
            publish(Phase.valueOf(pairing.state().name()), name, detail, code, pairing.attempt());
        }
    }

    private void publish(Phase phase, String desktop, String detail, String code, long attempt) {
        publish(phase, desktop, detail, code, attempt, false);
    }

    private void publish(Phase phase, String desktop, String detail, String code, long attempt, boolean retryAction) {
        if (!isClosed()) listener.accept(new View(phase, desktop, detail, code, attempt, savedDesktop, retryAction));
    }

    private boolean isClosed() { return !trust.isCurrent(); }

    private static long now() { return SystemClock.elapsedRealtime(); }
    private static long wall() { return System.currentTimeMillis() / 1_000L; }

    private static ScheduledFuture<?> abortAfter(Socket socket, int delay) {
        return DEADLINES.schedule(() -> abort(socket), delay, TimeUnit.MILLISECONDS);
    }

    private static void abort(Socket socket) {
        if (socket == null) return;
        try { socket.close(); } catch (IOException ignored) { }
    }

    private static String explain(Exception failure) {
        for (Throwable cause = failure; cause != null; cause = cause.getCause()) {
            if (cause instanceof CertificateException && cause.getMessage() != null) return cause.getMessage();
        }
        if (failure instanceof SocketTimeoutException) return "The desktop connection timed out.";
        if (failure instanceof java.net.ConnectException) return "Could not reach DES Connect at that address.";
        if (failure instanceof java.net.SocketException) return "The desktop connection closed.";
        if (failure instanceof javax.net.ssl.SSLException) return "Could not establish a secure desktop connection.";
        if (failure instanceof org.json.JSONException) return "The desktop sent an invalid packet.";
        if (failure instanceof IOException && failure.getMessage() != null) return failure.getMessage();
        return "Could not open the desktop connection.";
    }
}
