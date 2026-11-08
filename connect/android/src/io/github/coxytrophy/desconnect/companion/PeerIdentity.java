package io.github.coxytrophy.desconnect.companion;

import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.MessageDigest;
import java.security.cert.CertificateException;
import java.security.cert.X509Certificate;
import java.security.interfaces.RSAPublicKey;
import javax.security.auth.x500.X500Principal;

final class PeerIdentity {
    static boolean validId(String id) {
        return id != null && id.matches("[A-Za-z0-9_-]{32,38}");
    }

    static String certificateId(X509Certificate cert) throws CertificateException {
        String dn = cert.getSubjectX500Principal().getName(X500Principal.RFC2253);
        String id = null;
        // RFC 2253 separates RDNs on unescaped commas and their values on
        // unescaped plus signs. IDs use only unescaped ASCII token characters.
        int start = 0;
        for (int i = 0; i <= dn.length(); i++) {
            if (i == dn.length() || dn.charAt(i) == ',' || dn.charAt(i) == '+') {
                if (dn.startsWith("CN=", start)) {
                    if (id != null) throw new CertificateException("Desktop certificate has multiple IDs");
                    id = dn.substring(start + 3, i);
                    if (!validId(id)) throw new CertificateException("Invalid device certificate ID");
                }
                start = i + 1;
            } else if (dn.charAt(i) == '\\') {
                i++;
            }
        }
        if (id == null) throw new CertificateException("Missing device certificate ID");
        return id;
    }

    static void validate(X509Certificate cert) throws GeneralSecurityException {
        certificateId(cert);
        cert.checkValidity();
        if (!cert.getSubjectX500Principal().equals(cert.getIssuerX500Principal()))
            throw new CertificateException("Device certificate is not self-signed");
        if (cert.getPublicKey() instanceof RSAPublicKey
                && ((RSAPublicKey) cert.getPublicKey()).getModulus().bitLength() < 2048)
            throw new CertificateException("Weak device certificate");
        cert.verify(cert.getPublicKey());
    }

    static String fingerprint(X509Certificate cert) throws GeneralSecurityException {
        return hex(MessageDigest.getInstance("SHA-256").digest(cert.getEncoded()), 32);
    }

    static String code(byte[] a, byte[] b, long timestamp) throws GeneralSecurityException {
        MessageDigest hash = MessageDigest.getInstance("SHA-256");
        if (compare(a, b) >= 0) {
            hash.update(a);
            hash.update(b);
        } else {
            hash.update(b);
            hash.update(a);
        }
        return hex(hash.digest(Long.toString(timestamp).getBytes(StandardCharsets.US_ASCII)), 4);
    }

    static String hex(byte[] bytes, int length) {
        char[] output = new char[length * 2];
        String alphabet = "0123456789ABCDEF";
        for (int i = 0; i < length; i++) {
            output[i * 2] = alphabet.charAt((bytes[i] & 255) >>> 4);
            output[i * 2 + 1] = alphabet.charAt(bytes[i] & 15);
        }
        return new String(output);
    }

    private static int compare(byte[] a, byte[] b) {
        for (int i = 0; i < Math.min(a.length, b.length); i++) {
            int difference = (a[i] & 255) - (b[i] & 255);
            if (difference != 0) return difference;
        }
        return a.length - b.length;
    }
}
