#!/bin/sh
# Turns the APK cargo-apk built into one with Java code: cargo-apk can neither add a provider to the manifest nor a
# classes.dex, and opening a file in another app needs a content provider (crates/macsplorer-android/java). Compiles
# the Java with javac, dexes it with d8, adds the provider to the manifest cargo-apk generated, and packs, aligns and
# signs again. Called by build-android.sh after `cargo apk build`; needs a JDK and the Android SDK (platform 34, build tools).
set -e
cd "$(dirname "$0")/.."
ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
TOOLS="$(ls -d "$ANDROID_HOME"/build-tools/* | sort -V | tail -1)"
PLATFORM="$ANDROID_HOME/platforms/android-34/android.jar"
CRATE=crates/macsplorer-android
APK_DIR=target/release/apk
WORK=target/release/apk-java
LIB=libmacsplorer_android.so

rm -rf "$WORK"
mkdir -p "$WORK/classes" "$WORK/apk/lib/arm64-v8a"
javac --release 11 -Xlint:all -cp "$PLATFORM" -d "$WORK/classes" $(find $CRATE/java -name '*.java')
"$TOOLS/d8" --release --min-api 26 --lib "$PLATFORM" --output "$WORK/apk" $(find "$WORK/classes" -name '*.class')

# The manifest cargo-apk wrote: code is allowed now, and the provider is declared.
python3 - "$APK_DIR/AndroidManifest.xml" "$WORK/AndroidManifest.xml" <<'PY'
import sys
s = open(sys.argv[1]).read()
s = s.replace('android:hasCode="false"', 'android:hasCode="true"')
provider = '<provider android:name="dev.macsplorer.ai.Files" android:authorities="dev.macsplorer.ai.files" android:exported="false" android:grantUriPermissions="true"/>'
assert '</application>' in s and 'android:hasCode="true"' in s
open(sys.argv[2], 'w').write(s.replace('</application>', provider + '</application>'))
PY

cp "$APK_DIR/lib/arm64-v8a/$LIB" "$WORK/apk/lib/arm64-v8a/$LIB"
"$TOOLS/aapt" package -f -F "$WORK/unaligned.apk" -M "$WORK/AndroidManifest.xml" -I "$PLATFORM" -S $CRATE/res
(cd "$WORK/apk" && "$TOOLS/aapt" add ../unaligned.apk classes.dex "lib/arm64-v8a/$LIB" >/dev/null)
"$TOOLS/zipalign" -f 4 "$WORK/unaligned.apk" "$APK_DIR/macsplorer.apk"
"$TOOLS/apksigner" sign --ks "$CARGO_APK_RELEASE_KEYSTORE" --ks-pass "pass:$CARGO_APK_RELEASE_KEYSTORE_PASSWORD" "$APK_DIR/macsplorer.apk"
