#!/usr/bin/env bash
#
# Package a macOS .app bundle into a compressed, drag-to-install DMG.
#
# Usage:
#   launcher/scripts/make-dmg.sh [--lzma] [--out <file.dmg>] <path/to/App.app>
#
# The volume is named after the app and its CFBundleShortVersionString, which
# version.sh --stamp writes, and the window background, rendered for each
# build by generate-dmg-background.swift, shows that version too.
#
# The volume is APFS and the image is compressed with LZFSE (hdiutil format
# ULFO): good ratio, very fast decompression, and the pair Sparkle recommends
# for update archives, since it mounts and extracts the bundle from this same
# image. Pass --lzma for LZMA (ULMO), which compresses smaller but decompresses
# slower. Both need macOS 10.13+ for APFS, far below what the app itself
# needs (LSMinimumSystemVersion in the app's Info.plist).
#
# Only stock macOS tools are used (hdiutil, osascript, ditto, PlistBuddy),
# plus swift, which comes with Xcode or the Command Line Tools.

set -euo pipefail

# Finder window geometry. Must stay in sync with the artwork produced by
# generate-dmg-background.swift (660x400 pt canvas).
WINDOW_W=660
WINDOW_H=400
TITLEBAR_H=28
ICON_SIZE=128
APP_POS_X=165
APP_POS_Y=200
APPLICATIONS_POS_X=495
APPLICATIONS_POS_Y=200

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

require_macos "hdiutil, Finder"

FORMAT=ULFO
OUT=""
APP=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --lzma) FORMAT=ULMO; shift ;;
    --out) OUT="${2:?--out needs a path}"; shift 2 ;;
    -h|--help) usage 0 ;;
    -*) die "unknown option: $1" ;;
    *)
      [[ -z "$APP" ]] || die "unexpected extra argument: $1"
      APP="$1"; shift ;;
  esac
done

[[ -n "$APP" ]] || usage
[[ -d "$APP" ]] || die "not an app bundle: $APP"
[[ -f "$APP/Contents/Info.plist" ]] || die "missing Contents/Info.plist in: $APP"
# Without Xcode or the Command Line Tools, /usr/bin/swift is a stub that
# opens an install dialog instead of running.
xcrun -f swift >/dev/null 2>&1 || die "swift not found; install Xcode or the Command Line Tools"

APP_BASENAME="$(basename "$APP")"
APP_NAME="$($PLIST -c 'Print :CFBundleDisplayName' "$APP/Contents/Info.plist" 2>/dev/null \
  || $PLIST -c 'Print :CFBundleName' "$APP/Contents/Info.plist" 2>/dev/null \
  || basename "$APP_BASENAME" .app)"
VERSION="$($PLIST -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist" 2>/dev/null || true)"
[[ -n "$VERSION" ]] || die "no CFBundleShortVersionString in $APP/Contents/Info.plist (stamp the bundle with version.sh --stamp)"
[[ -n "$OUT" ]] || OUT="$(dirname "$APP")/$APP_NAME.dmg"

TMP_DIR="$(mktemp -d -t make-dmg)"
STAGING="$TMP_DIR/staging"
RW_DMG="$TMP_DIR/rw.dmg"
MOUNT_DIR=""

cleanup() {
  if [[ -n "$MOUNT_DIR" && -d "$MOUNT_DIR" ]]; then
    hdiutil detach "$MOUNT_DIR" -force >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

echo "==> Staging $APP_BASENAME"
mkdir -p "$STAGING/.background"
ditto "$APP" "$STAGING/$APP_BASENAME"
ln -s /Applications "$STAGING/Applications"

echo "==> Rendering background for version $VERSION"
swift "$LAUNCHER_DIR/scripts/generate-dmg-background.swift" \
  "$STAGING/.background/background.png" "$VERSION"

echo "==> Creating writable image"
hdiutil create -srcfolder "$STAGING" -volname "$APP_NAME $VERSION" \
  -fs APFS -format UDRW -ov -quiet "$RW_DMG"

echo "==> Mounting"
ATTACH_PLIST="$(hdiutil attach -readwrite -noverify -noautoopen -plist "$RW_DMG")"
MOUNT_DIR="$(printf '%s' "$ATTACH_PLIST" \
  | grep -A1 '<key>mount-point</key>' \
  | sed -n 's:.*<string>\(.*\)</string>.*:\1:p' \
  | head -1)"
[[ -d "$MOUNT_DIR" ]] || die "could not determine mount point"
# Finder addresses the volume by name; use the actual mount-point basename in
# case the volume name was deduplicated (e.g. "App 1" when "App" was mounted).
VOLUME_NAME="$(basename "$MOUNT_DIR")"

echo "==> Styling Finder window"
osascript <<EOF
tell application "Finder"
  tell disk "$VOLUME_NAME"
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set the bounds of container window to {400, 200, $((400 + WINDOW_W)), $((200 + WINDOW_H + TITLEBAR_H))}
    set viewOptions to the icon view options of container window
    set arrangement of viewOptions to not arranged
    set icon size of viewOptions to $ICON_SIZE
    set background picture of viewOptions to file ".background:background.png"
    set position of item "$APP_BASENAME" of container window to {$APP_POS_X, $APP_POS_Y}
    set position of item "Applications" of container window to {$APPLICATIONS_POS_X, $APPLICATIONS_POS_Y}
    -- Finder sometimes ignores the first bounds change while the window is
    -- still settling; set it again after layout so the saved size is right.
    delay 1
    set the bounds of container window to {400, 200, $((400 + WINDOW_W)), $((200 + WINDOW_H + TITLEBAR_H))}
    update without registering applications
    delay 1
    close
  end tell
end tell
EOF

sync

echo "==> Unmounting"
for _ in 1 2 3 4 5; do
  if hdiutil detach "$MOUNT_DIR" -quiet 2>/dev/null; then
    MOUNT_DIR=""
    break
  fi
  sleep 1
done
if [[ -n "$MOUNT_DIR" ]]; then
  hdiutil detach "$MOUNT_DIR" -force -quiet
  MOUNT_DIR=""
fi

echo "==> Compressing ($FORMAT)"
hdiutil convert "$RW_DMG" -format "$FORMAT" -ov -quiet -o "$OUT"
hdiutil verify -quiet "$OUT"

APP_BYTES="$(du -sk "$APP" | cut -f1)"
DMG_BYTES="$(du -sk "$OUT" | cut -f1)"
echo "==> Done: $OUT"
echo "    app:  $((APP_BYTES / 1024)) MiB, dmg: $((DMG_BYTES / 1024)) MiB ($((DMG_BYTES * 100 / APP_BYTES))% of original)"
