#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
sdk=${ANDROID_SDK_ROOT:-${ANDROID_HOME:-"$HOME/Android/Sdk"}}
tools="$sdk/build-tools/36.0.0"
platform="$sdk/platforms/android-36/android.jar"
keystore="${XDG_DATA_HOME:-"$HOME/.local/share"}/des-connect-preview/debug.keystore"
build="$PWD/build"

for tool in aapt2 d8 zipalign apksigner; do
    if [[ ! -x "$tools/$tool" ]]; then
        printf 'Install Android SDK Build-Tools 36.0.0 or set ANDROID_SDK_ROOT.\n' >&2
        exit 1
    fi
done
if [[ ! -f "$platform" ]]; then
    printf 'Install Android SDK Platform 36 or set ANDROID_SDK_ROOT.\n' >&2
    exit 1
fi
# The SDK launchers honor JAVA_HOME, so use PATH if a stale setting points nowhere.
if [[ -n "${JAVA_HOME:-}" && -x "$JAVA_HOME/bin/javac" ]]; then
    export PATH="$JAVA_HOME/bin:$PATH"
else
    unset JAVA_HOME
fi
for tool in javac jar keytool zip; do
    if ! command -v "$tool" >/dev/null; then
        printf 'Install a JDK (17 or newer recommended) and zip; missing %s.\n' "$tool" >&2
        exit 1
    fi
done

mkdir -p "$build"
scratch=$(mktemp -d "$build/.assemble.XXXXXXXX")
trap 'rm -rf -- "$scratch"' EXIT
mkdir -p "$scratch/classes" "$scratch/dex" "$scratch/generated" "$scratch/assets"
cp ../../shell/assets/fonts/{ChakraPetch-Medium.ttf,ChakraPetch-SemiBold.ttf,ChakraPetch-OFL.txt,JetBrainsMono.ttf,JetBrainsMono-OFL.txt} "$scratch/assets/"
cp ../../LICENSE "$scratch/assets/DES-UI-LICENSE.txt"
"$tools/aapt2" compile --dir res -o "$scratch/resources.zip"
"$tools/aapt2" link -I "$platform" --manifest AndroidManifest.xml \
    -A "$scratch/assets" --java "$scratch/generated" --auto-add-overlay \
    -o "$scratch/unsigned.apk" "$scratch/resources.zip"

export LC_ALL=C TZ=UTC
mapfile -d '' sources < <(find src "$scratch/generated" -type f -name '*.java' -print0 | sort -z)
javac --release 8 -Xlint:-options -encoding UTF-8 -classpath "$platform" -d "$scratch/classes" "${sources[@]}"
jar --create --file "$scratch/classes.jar" --no-manifest -C "$scratch/classes" .
"$tools/d8" --lib "$platform" --min-api 26 --release --output "$scratch/dex" "$scratch/classes.jar"
(
    cd "$scratch/dex"
    touch -t 200001010000 classes*.dex
    zip -Xq "$scratch/unsigned.apk" classes*.dex
)
"$tools/zipalign" -f 4 "$scratch/unsigned.apk" "$scratch/aligned.apk"

# Create a reusable, app-specific debug key. Never stage signing material in the checkout.
mkdir -p "$(dirname -- "$keystore")"
keystore_dir=$(cd -- "$(dirname -- "$keystore")" && pwd -P)
repo_dir=$(cd ../.. && pwd -P)
case "$keystore_dir/" in
    "$repo_dir/"*) printf 'Set XDG_DATA_HOME outside the source tree for the debug key.\n' >&2; exit 1 ;;
esac
if [[ ! -f "$keystore" ]]; then
    (
        umask 077
        private=$(mktemp -d "$keystore_dir/.signer.XXXXXXXX")
        trap 'rm -rf -- "$private"' EXIT
        keytool -genkeypair -keystore "$private/debug.keystore" -storetype PKCS12 \
            -alias des-connect-preview -keyalg RSA -keysize 2048 -validity 10000 \
            -dname 'CN=DES Connect Preview Debug' -storepass android -keypass android
        # Publish once, even if two builds create the debug key concurrently.
        ln -- "$private/debug.keystore" "$keystore" 2>/dev/null || [[ -f "$keystore" ]]
    )
fi
"$tools/apksigner" sign --ks "$keystore" --ks-key-alias des-connect-preview \
    --ks-pass pass:android --key-pass pass:android --v4-signing-enabled false \
    --out "$scratch/des-connect-preview.apk" "$scratch/aligned.apk"
"$tools/apksigner" verify "$scratch/des-connect-preview.apk"
mv -f -- "$scratch/des-connect-preview.apk" "$build/des-connect-preview.apk"
printf '%s\n' "$build/des-connect-preview.apk"
