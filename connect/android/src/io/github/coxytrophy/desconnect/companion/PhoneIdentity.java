package io.github.coxytrophy.desconnect.companion;

import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import java.math.BigInteger;
import java.security.KeyPairGenerator;
import java.security.KeyStore;
import java.security.Principal;
import java.security.PrivateKey;
import java.security.SecureRandom;
import java.security.cert.CertificateException;
import java.security.cert.X509Certificate;
import java.util.Date;
import java.util.Locale;
import javax.net.ssl.KeyManager;
import javax.net.ssl.SSLContext;
import javax.net.ssl.TrustManager;
import javax.net.ssl.X509KeyManager;
import javax.net.ssl.X509TrustManager;
import javax.security.auth.x500.X500Principal;

final class PhoneIdentity {
    private static final String STORE = "AndroidKeyStore";
    private static final String ALIAS = "des-connect-preview-rsa-v1";
    final String id;
    final X509Certificate cert;
    private final PrivateKey key;

    private PhoneIdentity(X509Certificate cert, PrivateKey key) throws Exception {
        PeerIdentity.validate(cert);
        this.id = PeerIdentity.certificateId(cert);
        this.cert = cert;
        this.key = key;
    }

    static synchronized PhoneIdentity load() throws Exception {
        KeyStore store = KeyStore.getInstance(STORE);
        store.load(null);
        if (!store.containsAlias(ALIAS)) {
            byte[] random = new byte[16];
            new SecureRandom().nextBytes(random);
            String id = PeerIdentity.hex(random, random.length).toLowerCase(Locale.ROOT);
            long now = System.currentTimeMillis();
            KeyGenParameterSpec spec = new KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_SIGN)
                    .setKeySize(2048)
                    .setDigests(KeyProperties.DIGEST_NONE, KeyProperties.DIGEST_SHA256, KeyProperties.DIGEST_SHA384, KeyProperties.DIGEST_SHA512)
                    .setSignaturePaddings(KeyProperties.SIGNATURE_PADDING_RSA_PKCS1,
                            KeyProperties.SIGNATURE_PADDING_RSA_PSS)
                    // Conscrypt encodes PSS before invoking Keystore's raw RSA signing primitive.
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .setCertificateSubject(new X500Principal("CN=" + id))
                    .setCertificateSerialNumber(new BigInteger(1, random).add(BigInteger.ONE))
                    .setCertificateNotBefore(new Date(now - 86_400_000L))
                    .setCertificateNotAfter(new Date(now + 315_360_000_000L))
                    .build();
            KeyPairGenerator generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_RSA, STORE);
            generator.initialize(spec);
            generator.generateKeyPair();
        }
        return new PhoneIdentity((X509Certificate) store.getCertificate(ALIAS),
                (PrivateKey) store.getKey(ALIAS, null));
    }

    SSLContext tls(PinStore.Pin expected) throws Exception {
        X509KeyManager serverKey = new X509KeyManager() {
            @Override public String[] getServerAliases(String type, Principal[] issuers) {
                return "RSA".equals(type) ? new String[]{ALIAS} : null;
            }

            @Override public String chooseServerAlias(String type, Principal[] issuers, java.net.Socket socket) {
                return "RSA".equals(type) ? ALIAS : null;
            }

            @Override public X509Certificate[] getCertificateChain(String alias) {
                return ALIAS.equals(alias) ? new X509Certificate[]{cert} : null;
            }

            @Override public PrivateKey getPrivateKey(String alias) {
                return ALIAS.equals(alias) ? key : null;
            }

            @Override public String[] getClientAliases(String type, Principal[] issuers) { return null; }

            @Override public String chooseClientAlias(String[] types, Principal[] issuers, java.net.Socket socket) {
                return null;
            }
        };
        X509TrustManager peerTrust = new X509TrustManager() {
            @Override public void checkClientTrusted(X509Certificate[] chain, String authType)
                    throws CertificateException {
                if (chain == null || chain.length != 1)
                    throw new CertificateException("Expected one self-signed desktop certificate");
                try {
                    PeerIdentity.validate(chain[0]);
                    if (expected != null) expected.verify(chain[0]);
                } catch (CertificateException failure) {
                    throw failure;
                } catch (Exception failure) {
                    throw new CertificateException("Desktop certificate verification failed", failure);
                }
            }

            @Override public void checkServerTrusted(X509Certificate[] chain, String authType)
                    throws CertificateException {
                throw new CertificateException("Unexpected TLS role");
            }

            @Override public X509Certificate[] getAcceptedIssuers() { return new X509Certificate[0]; }
        };
        SSLContext context = SSLContext.getInstance("TLSv1.2");
        context.init(new KeyManager[]{serverKey}, new TrustManager[]{peerTrust}, null);
        return context;
    }
}
