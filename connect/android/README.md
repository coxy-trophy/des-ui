# DES Connect Preview for Android

This is DES UI's standalone Android companion. It installs alongside the current
DES Connect app as `io.github.coxytrophy.desconnect.companion`, with its own keys
and pairings. The preview links to one desktop while the app is visible.

Build with JDK 17 or newer, `zip`, Android SDK Platform 36, and SDK Build-Tools
36.0.0. The build needs no Gradle or Maven downloads. Set `ANDROID_SDK_ROOT` if
the SDK is outside `~/Android/Sdk`. From the desktop source checkout:

```sh
connect/android/build.sh
adb -s DEVICE_SERIAL install -r connect/android/build/des-connect-preview.apk
```

The build writes a locally signed preview APK and verifies its signature. Keep
the app-specific debug signing key outside the checkout. This build targets
Android API 36 and can install on API 26 and later; installation compatibility
does not imply that every Android version has been tested.

1. Start the desktop's CONNECT service and open **DES Connect Preview** on the
   phone. Run **Discover** in the desktop panel. Tap the desktop when it appears,
   or enter its local IP address and TCP port manually.
2. Start pairing on either device and compare the eight-character code. When
   starting on the phone, tap **Codes match** there before accepting on the
   desktop. If pairing is interrupted and only the desktop remembers it, use
   **Unpair** on the desktop and start again.
3. Use **Send ping** to check the link. The desktop shows the phone's battery and
   charging state sampled on pairing, reconnect and desktop battery requests.
   Leaving the app closes the connection. Reopen it and tap **Reconnect** to use
   the saved trust. If the desktop's identity changed, verify the reason before
   using **Forget saved desktop** and pairing again.

UDP discovery only suggests addresses. The authenticated connection uses
mutual TLS 1.2, the Android Keystore, and a pinned desktop certificate. New
pairings require comparing codes. For simultaneous pairing requests to be
handled safely on both sides, use a desktop build from this revision or later.

The current companion is still needed for file transfers and media controls.
See [wireless casting](../CAST.md) for Android screen sharing and optional remote
gestures. A foreground service runs only during an explicitly approved cast;
pairing and ordinary use do not request screen capture or accessibility access.

The host Java checks exercise framing, verification codes, pairing approval and
the lifecycle of saved trust. They need no device or SDK:

```sh
src=connect/android/src/io/github/coxytrophy/desconnect/companion
tests=connect/android/tests/io/github/coxytrophy/desconnect/companion
classes=$(mktemp -d)
javac --release 8 -d "$classes" "$src"/{Frames,PeerIdentity,Pairing,TrustGate,CastSession,CastGeometry}.java "$tests"/*.java && (
    for test in WireTest PairingTest TrustGateTest CastSessionTest; do
        java -cp "$classes" "io.github.coxytrophy.desconnect.companion.$test" || exit 1
    done
)
```
