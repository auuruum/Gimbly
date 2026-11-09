#!/bin/sh
# Build a local application bundle. Set both signing variables to embed the
# virtual camera extension. A preview-only bundle can use ad hoc signing.
set -eu

if [ "$(uname -s)" != Darwin ]; then
    echo "Run this script on macOS with the Xcode Command Line Tools installed." >&2
    exit 1
fi
cd "$(dirname "$0")/../.."
identity=${GIMBLY_SIGN_IDENTITY:--}
team=${GIMBLY_TEAM_ID:-}
build_number=${GIMBLY_BUILD_NUMBER:-1}
case "$build_number" in
    ""|*[!0-9]*) echo "GIMBLY_BUILD_NUMBER must be a positive integer." >&2; exit 1 ;;
esac
if [ "$build_number" -eq 0 ]; then
    echo "GIMBLY_BUILD_NUMBER must be a positive integer." >&2
    exit 1
fi

features=
if [ "$identity" != "-" ] || [ -n "$team" ]; then
    case "$team" in
        ""|*[!A-Z0-9]*) echo "Set GIMBLY_TEAM_ID to your Apple Developer Team ID." >&2; exit 1 ;;
    esac
    if [ "${#team}" -ne 10 ] || [ "$identity" = "-" ]; then
        echo "A camera extension build requires a Team ID and a Developer ID Application identity." >&2
        exit 1
    fi
    features=macos-camera-extension
fi

destination=${1:-target/Gimbly.app}
if [ -e "$destination" ]; then
    echo "Destination already exists. Choose a new output path." >&2
    exit 1
fi
mkdir -p "$(dirname "$destination")"
staging=$(mktemp -d "$(dirname "$destination")/.gimbly.XXXXXXXX")
trap 'rm -rf "$staging"' EXIT HUP INT TERM
bundle="$staging/Gimbly.app"
contents="$bundle/Contents"
mkdir -p "$contents/MacOS"
cp packaging/macos/Info.plist "$contents/Info.plist"
plutil -replace CFBundleVersion -string "$build_number" "$contents/Info.plist"

# Use a predictable target directory even if the caller has Cargo overrides.
if [ -n "$features" ]; then
    MACOSX_DEPLOYMENT_TARGET=13.0 cargo build --locked --release -p gimbly --bins \
        --features "$features" --target-dir target
else
    MACOSX_DEPLOYMENT_TARGET=13.0 cargo build --locked --release -p gimbly --bin gimbly --target-dir target
fi
cp target/release/gimbly "$contents/MacOS/gimbly"

if [ -n "$features" ]; then
    extension="$contents/Library/SystemExtensions/io.github.auuruum.gimbly.camera.systemextension/Contents"
    mkdir -p "$extension/MacOS"
    cp target/release/gimbly-camera-extension "$extension/MacOS/gimbly-camera-extension"
    sed "s/TEAM_ID/$team/" packaging/macos/CameraExtension.plist > "$extension/Info.plist"
    plutil -replace CFBundleVersion -string "$build_number" "$extension/Info.plist"
    group="$team.io.github.auuruum.gimbly"
    cat > "$staging/extension-entitlements.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>com.apple.security.app-sandbox</key><true/>
<key>com.apple.security.application-groups</key><array><string>$group</string></array>
</dict></plist>
PLIST
    cat > "$staging/app-entitlements.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>com.apple.developer.system-extension.install</key><true/>
<key>com.apple.security.device.camera</key><true/>
<key>com.apple.security.application-groups</key><array><string>$group</string></array>
</dict></plist>
PLIST
    if [ -n "${GIMBLY_APP_PROFILE:-}" ]; then
        cp "$GIMBLY_APP_PROFILE" "$contents/embedded.provisionprofile"
    fi
    if [ -n "${GIMBLY_EXTENSION_PROFILE:-}" ]; then
        cp "$GIMBLY_EXTENSION_PROFILE" "$extension/embedded.provisionprofile"
    fi
    codesign --force --sign "$identity" --timestamp --options runtime \
        --entitlements "$staging/extension-entitlements.plist" "$(dirname "$extension")"
    codesign --force --sign "$identity" --timestamp --options runtime \
        --entitlements "$staging/app-entitlements.plist" "$bundle"
else
    codesign --force --sign - "$bundle"
fi
codesign --verify --deep --strict "$bundle"
mv "$bundle" "$destination"
printf 'Built %s\n' "$destination"
