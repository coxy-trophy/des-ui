package io.github.coxytrophy.desconnect.companion;

import android.util.JsonReader;
import android.util.JsonToken;
import java.io.IOException;
import java.io.OutputStream;
import java.io.StringReader;
import java.net.ProtocolException;
import java.nio.ByteBuffer;
import java.nio.charset.CodingErrorAction;
import java.nio.charset.StandardCharsets;
import java.util.regex.Pattern;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

final class WirePacket {
    static final String IDENTITY = "kdeconnect.identity";
    static final String PAIR = "kdeconnect.pair";
    static final String PING = "kdeconnect.ping";
    static final String BATTERY = "kdeconnect.battery";
    static final String BATTERY_REQUEST = "kdeconnect.battery.request";
    private static final Pattern TOKEN = Pattern.compile("[A-Za-z0-9._-]{1,128}");
    private static final Pattern INTEGER = Pattern.compile("-?(0|[1-9][0-9]*)");

    final String type;
    final JSONObject body;

    private WirePacket(String type, JSONObject body) {
        this.type = type;
        this.body = body;
    }

    static WirePacket parse(byte[] frame) throws IOException, JSONException {
        String utf8 = StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
                .decode(ByteBuffer.wrap(frame)).toString();
        try (JsonReader reader = new JsonReader(new StringReader(utf8))) {
            Object value = readValue(reader, 0);
            if (!(value instanceof JSONObject) || reader.peek() != JsonToken.END_DOCUMENT)
                throw new ProtocolException("Invalid packet");
            JSONObject packet = (JSONObject) value;
            Object type = packet.opt("type");
            Object body = packet.opt("body");
            if (!(type instanceof String) || !TOKEN.matcher((String) type).matches()
                    || !(body instanceof JSONObject)
                    || (packet.has("id") && !(packet.opt("id") instanceof Long)))
                throw new ProtocolException("Invalid packet header");
            if (packet.has("payloadSize") || packet.has("payloadTransferInfo"))
                throw new ProtocolException("Packet payloads are unsupported");
            return new WirePacket((String) type, (JSONObject) body);
        }
    }

    private static Object readValue(JsonReader reader, int depth) throws IOException, JSONException {
        if (depth > 32) throw new ProtocolException("JSON nesting exceeds the limit");
        switch (reader.peek()) {
            case BEGIN_OBJECT:
                JSONObject object = new JSONObject();
                reader.beginObject();
                while (reader.hasNext()) {
                    String name = reader.nextName();
                    if (object.has(name)) throw new ProtocolException("Duplicate JSON field");
                    object.put(name, readValue(reader, depth + 1));
                }
                reader.endObject();
                return object;
            case BEGIN_ARRAY:
                JSONArray array = new JSONArray();
                reader.beginArray();
                while (reader.hasNext()) array.put(readValue(reader, depth + 1));
                reader.endArray();
                return array;
            case STRING:
                return reader.nextString();
            case NUMBER:
                String number = reader.nextString();
                if (INTEGER.matcher(number).matches()) {
                    try {
                        return Long.valueOf(number);
                    } catch (NumberFormatException ignored) {
                        // Values outside signed 64-bit range cannot be protocol integers.
                    }
                }
                return new java.math.BigDecimal(number);
            case BOOLEAN:
                return reader.nextBoolean();
            case NULL:
                reader.nextNull();
                return JSONObject.NULL;
            default:
                throw new ProtocolException("Invalid JSON value");
        }
    }

    static void write(OutputStream output, String type, JSONObject body) throws IOException, JSONException {
        byte[] bytes = (new JSONObject()
                .put("id", System.currentTimeMillis())
                .put("type", type)
                .put("body", body).toString() + "\n").getBytes(StandardCharsets.UTF_8);
        if (bytes.length > Frames.MAX) throw new ProtocolException("Packet exceeds 64 KiB");
        output.write(bytes);
        output.flush();
    }

    String desktopName(String peer, String local) throws ProtocolException {
        Object name = body.opt("deviceName");
        Object kind = body.opt("deviceType");
        Object port = body.opt("tcpPort");
        if (!IDENTITY.equals(type) || !peer.equals(body.opt("deviceId"))
                || peer.equals(local)
                || !Long.valueOf(8).equals(body.opt("protocolVersion"))
                || !("desktop".equals(kind) || "laptop".equals(kind))
                || (body.has("targetDeviceId") && !local.equals(body.opt("targetDeviceId")))
                || (port != null && (!(port instanceof Long) || (Long) port < 1 || (Long) port > 65535))
                || !(name instanceof String) || !validName((String) name)
                || !validCapabilities(body.optJSONArray("incomingCapabilities"))
                || !validCapabilities(body.optJSONArray("outgoingCapabilities")))
            throw new ProtocolException("Desktop identity does not match its TLS certificate or protocol");
        return (String) name;
    }

    boolean supports(String capability) {
        return contains(body.optJSONArray("incomingCapabilities"), capability)
                && contains(body.optJSONArray("outgoingCapabilities"), capability);
    }

    private static boolean contains(JSONArray list, String value) {
        if (list == null) return false;
        for (int i = 0; i < list.length(); i++) {
            if (value.equals(list.opt(i))) return true;
        }
        return false;
    }

    private static boolean validName(String name) {
        if (name.codePointCount(0, name.length()) > 256 || name.trim().isEmpty()) return false;
        for (int i = 0; i < name.length(); ) {
            int ch = name.codePointAt(i);
            if (Character.isISOControl(ch) || (ch >= 0xd800 && ch <= 0xdfff)) return false;
            i += Character.charCount(ch);
        }
        return true;
    }

    private static boolean validCapabilities(JSONArray caps) {
        if (caps == null || caps.length() > 256) return false;
        for (int i = 0; i < caps.length(); i++) {
            Object cap = caps.opt(i);
            if (!(cap instanceof String) || !TOKEN.matcher((String) cap).matches()) return false;
        }
        return true;
    }
}
