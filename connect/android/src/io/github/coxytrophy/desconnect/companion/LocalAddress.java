package io.github.coxytrophy.desconnect.companion;

import java.io.IOException;
import java.net.InetAddress;

final class LocalAddress {
    static InetAddress parse(String host) throws IOException {
        String text = host == null ? "" : host.trim();
        if (text.startsWith("[") && text.endsWith("]"))
            text = text.substring(1, text.length() - 1);
        InetAddress address;
        if (text.indexOf(':') >= 0 && text.matches("[0-9A-Fa-f:.]+(%[A-Za-z0-9_.-]+)?")) {
            address = InetAddress.getByName(text);
        } else if (text.matches("[0-9]{1,3}(\\.[0-9]{1,3}){3}")) {
            String[] parts = text.split("\\.");
            byte[] bytes = new byte[4];
            for (int i = 0; i < bytes.length; i++) {
                int value = Integer.parseInt(parts[i]);
                if (value > 255) throw new IOException("Enter a valid local IP address.");
                bytes[i] = (byte) value;
            }
            address = InetAddress.getByAddress(bytes);
        } else {
            throw new IOException("Enter a valid local IP address.");
        }
        byte[] bytes = address.getAddress();
        int first = bytes[0] & 255;
        int second = bytes[1] & 255;
        boolean local = bytes.length == 4
                ? first == 10 || first == 127 || (first == 169 && second == 254)
                || (first == 172 && second >= 16 && second <= 31) || (first == 192 && second == 168)
                : address.isLoopbackAddress() || address.isLinkLocalAddress() || (first & 254) == 252;
        if (!local) throw new IOException("DES Connect requires a local IP address.");
        return address;
    }
}
