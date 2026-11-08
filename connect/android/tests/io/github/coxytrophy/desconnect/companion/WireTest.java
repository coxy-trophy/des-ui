package io.github.coxytrophy.desconnect.companion;

import java.io.ByteArrayInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.SocketTimeoutException;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;

public final class WireTest {
    public static void main(String[] args) throws Exception {
        retainsPartialPacketAcrossPollTimeout();
        rejectsTruncatedPacket();
        rejectsOversizedPacket();
        sortsKeyBytesUnsigned();
        System.out.println("Wire regression checks passed");
    }

    private static void retainsPartialPacketAcrossPollTimeout() throws IOException {
        InputStream stream = new InputStream() {
            int next;
            @Override public int read(byte[] bytes, int offset, int length) throws IOException {
                int b = read();
                if (b < 0) return -1;
                bytes[offset] = (byte) b;
                return 1;
            }

            @Override public int read() throws IOException {
                switch (next++) {
                    case 0: return 'a';
                    case 1: throw new SocketTimeoutException();
                    case 2: return 'b';
                    case 3: return '\n';
                    default: return -1;
                }
            }
        };
        Frames reader = new Frames(stream, () -> 10L);
        expect(reader.read() == null, "an idle poll should not emit an unfinished frame");
        expect(Arrays.equals(reader.read(), "ab".getBytes(StandardCharsets.US_ASCII)),
                "the next read must include the prefix read before the timeout");
    }

    private static void rejectsTruncatedPacket() throws IOException {
        expectProtocolFailure(new Frames(new ByteArrayInputStream(new byte[] {'a'}), () -> 10L));
    }

    private static void rejectsOversizedPacket() throws IOException {
        byte[] bytes = new byte[64 * 1024 + 1];
        bytes[bytes.length - 1] = '\n';
        expectProtocolFailure(new Frames(new ByteArrayInputStream(bytes), () -> 10L));
    }

    private static void expectProtocolFailure(Frames reader) throws IOException {
        try {
            reader.read();
        } catch (java.net.ProtocolException expected) {
            return;
        }
        throw new AssertionError("the reader must close on an invalid frame");
    }

    private static void sortsKeyBytesUnsigned() throws Exception {
        byte[] low = { 0x7f, (byte) 0xff };
        byte[] high = { (byte) 0x80, (byte) 0xa0, 0x01 };
        expect("96DFC8D1".equals(PeerIdentity.code(low, high, 1_800_000_000L)),
                "the code must hash unsigned descending key bytes followed by the decimal timestamp");
        expect("96DFC8D1".equals(PeerIdentity.code(high, low, 1_800_000_000L)),
                "the code must be the same on both peers");
    }

    private static void expect(boolean value, String message) {
        if (!value) throw new AssertionError(message);
    }
}
