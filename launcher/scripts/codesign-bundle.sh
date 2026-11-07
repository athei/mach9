#!/usr/bin/env bash
#
# Sign and notarize a macOS .app bundle, as two separate commands that are run
# one after the other:
#
#   launcher/scripts/codesign-bundle.sh sign     [options] <path/to/App.app>
#   launcher/scripts/codesign-bundle.sh notarize [options] <path/to/App.app>
#
# sign: every Mach-O inside the bundle is signed inside-out with the hardened
# runtime and (unless --adhoc) a Developer ID identity + secure timestamp. A
# Mach-O that already carries entitlements keeps exactly the set it has, so
# re-signing can never quietly drop an entitlement the binary was built to
# need; one that carries none gets none. The app has no entitlements of its
# own: every entitlement Wine needs is on the Wine loader, wine.app, which
# bin/bundle-wine.sh signs with its provisioning profile, and entitlements
# only take effect on a main executable, which no other Mach-O of the Wine
# tree is.
# Nested bundles holding an embedded provisioning profile (wine.app) and code
# Apple signed itself are never re-signed; see --skip. Any *.dSYM inside the
# bundle is removed first: its DWARF companion is a Mach-O the notary service
# would see unsigned.
#
# Both commands first check the Wine loader: every wine.app in the bundle has
# to carry its provisioning profile and the cross-architecture entitlement,
# and (for a Developer ID signature) belong to the team the bundle is signed
# for. A loader without them has been re-signed or built without the profile,
# and Wine would not start.
#
# Both commands stop early on a bundle that already carries a stapled ticket
# matching it: notarize has nothing to upload, and sign would invalidate the
# ticket for no gain. The ticket is bound to the signature it was issued for,
# so anything that changed the bundle makes stapler reject it and both commands
# do their work as normal; --force overrides either way.
#
# notarize: never signs. It verifies locally that every Mach-O is
# notarization-ready (valid Developer ID signature, hardened runtime, secure
# timestamp, no get-task-allow), so a bad bundle never costs an upload and a
# round trip through Apple's queue. Then it packs a compressed DMG (hdiutil,
# multithreaded LZFSE), submits with `notarytool submit --wait`, staples the
# ticket, and Gatekeeper-assesses the result.
#
# sign options:
#   --identity <name|sha1>    Signing identity. Default: the single "Developer
#                             ID Application" certificate in the keychain.
#   --adhoc                   Ad-hoc sign (no identity, no timestamp), for
#                             local testing only; can never be notarized.
#   --entitlements <glob>=<plist>
#                             Override rule: Mach-Os whose bundle-relative path
#                             matches <glob> are signed with <plist>.
#                             Repeatable; last match wins; beats the
#                             preserved entitlements.
#   --skip <glob>             Never codesign anything whose bundle-relative
#                             path matches <glob>, or that lives under a
#                             directory matching it. Repeatable. Bundles with
#                             an embedded.provisionprofile and code carrying
#                             Apple's own signature are skipped anyway.
#   --no-preserve-entitlements
#                             Do not reuse a binary's existing entitlements;
#                             only --entitlements rules give any.
#
# notarize options:
#   --profile <name>          notarytool keychain profile (created with
#                             `xcrun notarytool store-credentials`).
#                             Default: default, unless the environment names
#                             an API key (below)
#
# notarize environment:
#   NOTARY_KEY_PATH, NOTARY_KEY_ID, NOTARY_ISSUER
#                             An App Store Connect API key instead of a
#                             keychain profile: the path of its .p8 file, its
#                             key ID and its issuer ID, passed to notarytool
#                             as --key, --key-id and --issuer. All three or
#                             none; not together with --profile.
#
# common options:
#   --force                   Act even when the bundle already carries a valid
#                             stapled ticket, which both commands otherwise
#                             treat as nothing to do.
#   --dry-run                 Change nothing. With sign: list the .dSYM bundles
#                             that would be removed and every Mach-O and nested
#                             bundle that would be signed, with the
#                             entitlements each would get. With notarize: run
#                             the pre-flight verification, then stop without
#                             packing or uploading anything.
#   --jobs <n>                Parallel signing/verification jobs (default: CPU
#                             core count).
#
# com.apple.security.get-task-allow is refused in any entitlements file, in any
# set carried over from an existing signature, and in any existing signature at
# verification time: Apple's notary service hard-rejects it.
#
# Only stock macOS tools are used (codesign, security, xcrun, hdiutil,
# PlistBuddy).

set -euo pipefail

source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

# The entitlement only the Wine loader's provisioning profile can grant, and
# without which it cannot run x86 code with hardware memory ordering.
LOADER_ENTITLEMENT="com.apple.developer.cross-architecture-support"

# Whether the bundle already carries a stapled ticket that matches it. The
# ticket is bound to the code signature it was issued for, so this passing
# means the bundle is, byte for byte, the one Apple accepted. Both commands
# stop on it: notarize has nothing to upload, and sign would destroy it.
already_notarized() {
  xcrun stapler validate "$APP" >/dev/null 2>&1 \
    && spctl -a -t exec "$APP" >/dev/null 2>&1
}

require_macos codesign

COMMAND="${1:-}"
case "$COMMAND" in
  sign|notarize) shift ;;
  -h|--help) usage 0 ;;
  "") usage ;;
  *) die "unknown command: $COMMAND (expected 'sign' or 'notarize')" ;;
esac

IDENTITY=""
ADHOC=0
PRESERVE_ENTITLEMENTS=1
PROFILE=""
FORCE=0
JOBS="$(sysctl -n hw.ncpu)"
DRY_RUN=0
OVERRIDE_RULES=()
SKIP_GLOBS=()
APP=""

sign_only() {
  [[ "$COMMAND" == "sign" ]] || die "$1 is a 'sign' option (command is '$COMMAND')"
}
notarize_only() {
  [[ "$COMMAND" == "notarize" ]] || die "$1 is a 'notarize' option (command is '$COMMAND')"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --identity) sign_only "$1"; IDENTITY="${2:?--identity needs a value}"; shift 2 ;;
    --adhoc) sign_only "$1"; ADHOC=1; shift ;;
    --entitlements)
      sign_only "$1"
      rule="${2:?--entitlements needs <glob>=<plist>}"
      [[ "$rule" == *=* ]] || die "--entitlements expects <glob>=<plist>, got: $rule"
      OVERRIDE_RULES+=("$rule"); shift 2 ;;
    --skip)
      sign_only "$1"
      SKIP_GLOBS+=("${2:?--skip needs a glob}"); shift 2 ;;
    --no-preserve-entitlements) sign_only "$1"; PRESERVE_ENTITLEMENTS=0; shift ;;
    --profile) notarize_only "$1"; PROFILE="${2:?--profile needs a keychain profile name}"; shift 2 ;;
    --force) FORCE=1; shift ;;
    --jobs) JOBS="${2:?--jobs needs a number}"; shift 2 ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage 0 ;;
    -*) die "unknown option: $1" ;;
    *)
      [[ -z "$APP" ]] || die "unexpected extra argument: $1"
      APP="$1"; shift ;;
  esac
done

[[ -n "$APP" ]] || usage
APP="${APP%/}"
[[ -d "$APP" ]] || die "not an app bundle: $APP"
[[ -f "$APP/Contents/Info.plist" ]] || die "missing Contents/Info.plist in: $APP"
APP="$(cd "$(dirname "$APP")" && pwd)/$(basename "$APP")"
if [[ "$ADHOC" == 1 && -n "$IDENTITY" ]]; then
  die "--adhoc and --identity are mutually exclusive"
fi

# How notarytool authenticates: a keychain profile, or an App Store Connect
# API key from the environment. Settled before anything else, so a missing
# credential costs nothing.
NOTARY_AUTH=()
NOTARY_AUTH_LABEL=""
if [[ "$COMMAND" == "notarize" ]]; then
  if [[ -n "${NOTARY_KEY_PATH:-}${NOTARY_KEY_ID:-}${NOTARY_ISSUER:-}" ]]; then
    [[ -n "${NOTARY_KEY_PATH:-}" && -n "${NOTARY_KEY_ID:-}" && -n "${NOTARY_ISSUER:-}" ]] \
      || die "NOTARY_KEY_PATH, NOTARY_KEY_ID and NOTARY_ISSUER have to be set together"
    [[ -z "$PROFILE" ]] \
      || die "--profile and an API key in NOTARY_KEY_PATH are mutually exclusive"
    [[ -f "$NOTARY_KEY_PATH" ]] || die "NOTARY_KEY_PATH names no file: $NOTARY_KEY_PATH"
    NOTARY_AUTH=(--key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER")
    NOTARY_AUTH_LABEL="API key $NOTARY_KEY_ID"
  else
    PROFILE="${PROFILE:-default}"
    NOTARY_AUTH=(--keychain-profile "$PROFILE")
    NOTARY_AUTH_LABEL="keychain profile $PROFILE"
  fi
fi

# --- entitlements hygiene (sign) ----------------------------------------------

refuse_get_task_allow() {
  local plist="$1"
  [[ -f "$plist" ]] || die "entitlements file not found: $plist"
  if /usr/libexec/PlistBuddy -c 'Print :com.apple.security.get-task-allow' \
      "$plist" >/dev/null 2>&1; then
    die "refusing $plist: it contains com.apple.security.get-task-allow, which the notary service hard-rejects"
  fi
}

if [[ "$COMMAND" == "sign" ]]; then
  for rule in ${OVERRIDE_RULES[@]+"${OVERRIDE_RULES[@]}"}; do
    refuse_get_task_allow "${rule#*=}"
  done
fi

TMP_DIR="$(mktemp -d -t codesign-bundle)"
trap 'rm -rf "$TMP_DIR"' EXIT
ENTS_DIR="$TMP_DIR/ents"
mkdir -p "$ENTS_DIR"

# --- dangling symlinks ----------------------------------------------------------

# codesign seals a symlink by its target path, so a link pointing outside the
# bundle, or at a file that was never copied in, breaks the seal. Both
# `codesign --verify` and the notary service then report a bare "No such file or
# directory" against the app as a whole, naming neither the link nor the target,
# which is hard to debug across a few thousand files.
DANGLING=()
while IFS= read -r -d '' l; do
  [[ -e "$l" ]] || DANGLING+=("$l")
done < <(find "$APP" -type l -print0 | sort -z)

if [[ "${#DANGLING[@]}" -gt 0 ]]; then
  echo "error: ${#DANGLING[@]} symlink(s) in $(basename "$APP") resolve to nothing:" >&2
  for l in "${DANGLING[@]:0:10}"; do
    printf '    %s -> %s\n' "${l#"$APP"/}" "$(readlink "$l")" >&2
  done
  if (( ${#DANGLING[@]} > 10 )); then
    echo "    ... and $(( ${#DANGLING[@]} - 10 )) more" >&2
  fi
  die "a sealed symlink has to resolve inside the bundle; copy the missing target in, or drop the link"
fi

# --- the Wine loader and the signing identity ----------------------------------

# Both are settled before anything in the bundle changes (the .dSYM removal
# below is the first change), so a bundle this script refuses is left as it
# was.

# Every code bundle in the app. Used to find the Wine loader, the skip roots
# below, and to drive the deepest-first bundle signing pass.
find "$APP" -mindepth 1 -type d \
  \( -name '*.framework' -o -name '*.app' -o -name '*.xpc' \
     -o -name '*.appex' -o -name '*.bundle' -o -name '*.plugin' \
     -o -name '*.docktileplugin' \) \
  -print0 | sort -z > "$TMP_DIR/bundles"

# The team a signature belongs to, empty for an ad-hoc or unsigned one.
signature_team() {
  { codesign -d -vv "$1" 2>&1 || true; } \
    | sed -n 's/^TeamIdentifier=\([A-Z0-9]*\)$/\1/p'
}

# The team of a signing identity, a SHA-1 hash or a name as `security` lists
# it, in any case, read from the "(TEAMID)" that ends a Developer ID
# certificate's name.
identity_team() {
  { security find-identity -v -p codesigning | grep -i -F -- "$1" || true; } \
    | sed -n '1s/.*(\([A-Z0-9]\{10\}\))"$/\1/p'
}

# Check every Wine loader in the bundle, wine.app, which this script never
# re-signs: it has to keep its provisioning profile, so the skip below covers
# it, carry $LOADER_ENTITLEMENT, and, when a team is given, belong to that
# team. A bundle without a loader has no Wine runtime, which Mach9 cannot be
# shipped without. Each check names what to rebuild, since none of them can
# be fixed here: the loader's signature comes from the Wine bundle.
check_wine_loaders() {
  local team="$1" found=0 d rel got ents
  while IFS= read -r -d '' d; do
    [[ "$d" == */wine.app ]] || continue
    found=1
    rel="${d#"$APP"/}"
    [[ -f "$d/Contents/embedded.provisionprofile" ]] \
      || die "$rel has no embedded.provisionprofile, so it would be re-signed without its entitlements; deploy a Wine bundle signed with its profile"
    ents="$TMP_DIR/loader-entitlements.plist"
    codesign -d --entitlements - --xml "$d" > "$ents" 2>/dev/null \
      || die "cannot read the entitlements of $rel"
    got="$(/usr/libexec/PlistBuddy -c "Print :$LOADER_ENTITLEMENT" "$ents" 2>/dev/null || true)"
    [[ "$got" == "true" ]] \
      || die "$rel does not carry $LOADER_ENTITLEMENT, and Wine does not start without it; deploy a Wine bundle whose loader was signed with its provisioning profile"
    if [[ -n "$team" ]]; then
      got="$(signature_team "$d")"
      [[ "$got" == "$team" ]] \
        || die "$rel is signed for team ${got:-none}, the bundle for team $team; deploy a Wine bundle signed by the same team"
    fi
  done < "$TMP_DIR/bundles"
  [[ "$found" == 1 ]] \
    || die "no wine.app in $(basename "$APP"): the Wine runtime is missing; deploy it first"
}

if [[ "$COMMAND" == "sign" ]]; then
  # Re-signing a notarized bundle is destructive: the new signature is not the
  # one the ticket was issued for, so a bundle that Gatekeeper accepted offline
  # a moment ago stops being accepted, and the only way back is another
  # notarization round trip. Nothing here would change the signature's content
  # anyway, so there is nothing to gain by it.
  if [[ $FORCE -eq 0 ]] && already_notarized; then
    echo "==> Already signed and notarized: not re-signing"
    spctl -a -vv -t exec "$APP" 2>&1 | sed 's/^/    /'
    echo "    re-signing would invalidate the stapled ticket; pass --force to do it anyway"
    exit 0
  fi

  if [[ "$DRY_RUN" == 1 ]]; then
    # A dry run resolves no identity, so the team is not compared.
    echo "==> Checking the Wine loader"
    check_wine_loaders ""
  elif [[ "$ADHOC" == 1 ]]; then
    IDENTITY="-"
    TIMESTAMP_FLAG="--timestamp=none"
    echo "==> Signing ad-hoc (no identity, no timestamp)"
    # An ad-hoc signature has no team to compare.
    echo "==> Checking the Wine loader"
    check_wine_loaders ""
  else
    TIMESTAMP_FLAG="--timestamp"
    if [[ -z "$IDENTITY" ]]; then
      ids="$(security find-identity -v -p codesigning | grep "Developer ID Application" || true)"
      [[ -n "$ids" ]] || die "no 'Developer ID Application' identity in the keychain; pass --identity or --adhoc"
      if [[ "$(wc -l <<<"$ids")" -gt 1 ]]; then
        die "multiple Developer ID Application identities; pass --identity:
$ids"
      fi
      IDENTITY="$(awk '{print $2}' <<<"$ids")"
      echo "==> Using identity: $(sed 's/^ *[0-9]*) [0-9A-F]* //' <<<"$ids")"
    fi
    echo "==> Checking the Wine loader"
    SIGNING_TEAM="$(identity_team "$IDENTITY")"
    [[ -n "$SIGNING_TEAM" ]] \
      || die "cannot tell the team of identity $IDENTITY from 'security find-identity'"
    check_wine_loaders "$SIGNING_TEAM"
  fi
else
  # notarize changes nothing in the bundle, but it checks the loader first
  # too: an upload of a bundle whose Wine cannot start is wasted.
  echo "==> Checking the Wine loader"
  check_wine_loaders "$(signature_team "$APP")"
fi

# --- debug symbols --------------------------------------------------------------

# A *.dSYM holds a DWARF companion that is itself a Mach-O, so the scan below
# would find it, codesign has no business signing it, and shipping it unsigned
# is a notarization rejection waiting to happen. It is a build artifact: the
# Makefiles that install it next to the .so do so for local symbolication, and
# will put it back on the next `make install`. Removed only once the checks
# above have passed, so a bundle that cannot be signed is not modified either.
DSYMS=()
while IFS= read -r -d '' d; do DSYMS+=("$d"); done \
  < <(find "$APP" -mindepth 1 -type d -name '*.dSYM' -print0 | sort -z)

if [[ "${#DSYMS[@]}" -gt 0 ]]; then
  if [[ "$COMMAND" == "notarize" ]]; then
    die "${#DSYMS[@]} .dSYM bundle(s) still in $APP; run 'codesign-bundle.sh sign' on it first (it removes them)"
  fi
  if [[ "$DRY_RUN" == 1 ]]; then
    echo "==> Would remove ${#DSYMS[@]} .dSYM bundle(s)"
  else
    echo "==> Removing ${#DSYMS[@]} .dSYM bundle(s)"
  fi
  for d in "${DSYMS[@]}"; do
    echo "    ${d#"$APP"/}"
    [[ "$DRY_RUN" == 1 ]] || rm -rf "$d"
  done
fi

# --- subtrees that are never re-signed -----------------------------------------

# Bundle-relative globs whose matches, and everything beneath them, are left
# byte for byte as they are. A literal path is just a glob that matches once.
SKIP_ROOTS=()
SKIP_REASONS=()

# A nested bundle carrying a provisioning profile holds entitlements only its
# original signer can grant: the profile has to match the signature for them to
# take effect. Re-signing it does not fail, it silently downgrades the binary.
# The Wine loader, wine.app, is why this exists: its profile grants
# $LOADER_ENTITLEMENT (hardware memory ordering for FEX), and Wine refuses to
# start without it.
while IFS= read -r -d '' d; do
  if [[ -f "$d/Contents/embedded.provisionprofile" \
     || -f "$d/embedded.provisionprofile" ]]; then
    SKIP_ROOTS+=("${d#"$APP"/}")
    SKIP_REASONS+=("embedded.provisionprofile")
  fi
done < "$TMP_DIR/bundles"

for glob in ${SKIP_GLOBS[@]+"${SKIP_GLOBS[@]}"}; do
  SKIP_ROOTS+=("$glob")
  SKIP_REASONS+=("--skip")
done

# Skipping keeps codesign out. It does not exempt anything from `xattr -cr`,
# which cannot invalidate a signature (extended attributes live outside both the
# Mach-O and CodeResources), nor from verification, which still has to hold for
# the bundle to notarize.
is_skipped() {
  local rel="$1" root
  for root in ${SKIP_ROOTS[@]+"${SKIP_ROOTS[@]}"}; do
    if [[ "$rel" == $root || "$rel" == $root/* ]]; then return 0; fi
  done
  return 1
}

# --- entitlements resolution ----------------------------------------------------

# Write the entitlements a Mach-O already carries to a temp plist and print its
# path; print nothing if it carries none or is unsigned. Identical sets land in
# one file, so the signing pass still groups them into a single codesign run.
dump_entitlements() {
  local f="$1" xml sum out
  xml="$(codesign -d --entitlements :- "$f" 2>/dev/null || true)"
  case "$xml" in
    *'<key>'*) ;;
    *) return 0 ;;
  esac
  sum="$(printf '%s' "$xml" | shasum | cut -d' ' -f1)"
  out="$ENTS_DIR/$sum.plist"
  [[ -f "$out" ]] || printf '%s\n' "$xml" > "$out"
  printf '%s' "$out"
}

# Print "<source>\t<plist>" for a bundle-relative path, plist empty for none.
# Precedence: --entitlements override (last match wins) > the set the file
# already carries > nothing. Whoever built a binary knows what it needs better
# than a rule keyed on its file name; the Wine tree's own binaries other than
# the loader arrive with none and need none.
resolve_entitlements() {
  local rel="$1" preserved="${2:-}" plist="" src="none" rule glob
  if [[ -n "$preserved" ]]; then
    plist="$preserved"
    src="preserved"
  fi
  for rule in ${OVERRIDE_RULES[@]+"${OVERRIDE_RULES[@]}"}; do
    glob="${rule%%=*}"
    if [[ "$rel" == $glob ]]; then
      plist="${rule#*=}"
      src="override"
    fi
  done
  printf '%s\t%s' "$src" "$plist"
}

# --- Mach-O discovery ----------------------------------------------------------

# Detect Mach-O by magic bytes, not extension or exec bit: wine ships Mach-Os
# named *.so and extensionless, plus PE .exe/.dll files codesign cannot sign.
is_macho() {
  local f="$1" magic sub
  magic="$(od -An -N4 -tx1 "$f" 2>/dev/null | tr -d ' \n')" || return 1
  case "$magic" in
    feedface|feedfacf|cefaedfe|cffaedfe) return 0 ;;
    cafebabe|cafebabf)
      # Same magic as Java class files. Fat Mach-O's nfat_arch (big-endian u32
      # at offset 4) is tiny; Java's version field there is >= 45.
      sub="$(od -An -j4 -N4 -tx1 "$f" 2>/dev/null | tr -d ' \n')"
      if [[ -n "$sub" ]] && (( 16#$sub < 30 )); then return 0; fi
      return 1 ;;
    *) return 1 ;;
  esac
}

scan_batch() {
  local f
  for f in "$@"; do
    if is_macho "$f"; then printf '%s\0' "$f"; fi
  done
  return 0
}
export -f is_macho scan_batch

echo "==> Scanning $(basename "$APP") for Mach-O files"
find "$APP" -name '*.dSYM' -prune -o -type f -print0 \
  | xargs -0 -n 64 -P "$JOBS" bash -c 'scan_batch "$@"' _ \
  | sort -z > "$TMP_DIR/machos"

# Every Mach-O in the bundle, skipped subtrees included: they are part of the
# notarization payload, so they still have to pass verification.
MACHOS=()
while IFS= read -r -d '' f; do MACHOS+=("$f"); done < "$TMP_DIR/machos"

# Code Apple signed itself: the authority chain is Apple's platform one, not
# a Developer ID. Frameworks and libraries Apple ships for redistribution
# arrive this way.
is_apple_signed() {
  local info
  info="$(codesign -d -vv "$1" 2>&1 || true)"
  case "$info" in
    *"Authority=Software Signing"*"Authority=Apple Code Signing Certification Authority"*) return 0 ;;
    *) return 1 ;;
  esac
}
apple_batch() {
  local f
  for f in "$@"; do
    if is_apple_signed "$f"; then printf '%s\0' "$f"; fi
  done
  return 0
}
export -f is_apple_signed apple_batch

# Apple-signed code is left as it is, the way CrossOver ships D3DMetal inside
# its own Developer ID bundle. Re-signing it gains nothing (a library carries
# no entitlements, and wine loads it with library validation disabled) and
# only replaces Apple's authority with ours; the notary service accepts
# Apple-signed components without a hardened runtime. A bundle is checked as a
# whole so its leaves come along; a standalone dylib is its own skip root.
while IFS= read -r -d '' d; do
  if is_skipped "${d#"$APP"/}"; then continue; fi
  if is_apple_signed "$d"; then
    SKIP_ROOTS+=("${d#"$APP"/}")
    SKIP_REASONS+=("Apple-signed")
  fi
done < "$TMP_DIR/bundles"
while IFS= read -r -d '' f; do
  if is_skipped "${f#"$APP"/}"; then continue; fi
  SKIP_ROOTS+=("${f#"$APP"/}")
  SKIP_REASONS+=("Apple-signed")
done < <(xargs -0 -n 32 -P "$JOBS" bash -c 'apple_batch "$@"' _ < "$TMP_DIR/machos" | sort -z)
[[ "${#MACHOS[@]}" -gt 0 ]] || die "no Mach-O files found in $APP"

# --- verification ---------------------------------------------------------------

# With REQUIRE_DEVID=1 a Developer ID authority and a secure timestamp are also
# demanded, notary-service requirements that plain `codesign --verify` does
# not cover (an ad-hoc signature passes it). Code Apple signed itself only has
# to verify: it carries neither a hardened runtime nor a Developer ID, and the
# notary service takes it as it is.
verify_batch() {
  local f fail=0 info
  for f in "$@"; do
    if ! codesign --verify --strict "$f" 2>/dev/null; then
      echo "FAIL(signature) $f"; fail=1; continue
    fi
    if is_apple_signed "$f"; then continue; fi
    info="$(codesign -d -vv "$f" 2>&1 || true)"
    case "$info" in
      *"flags="*"runtime"*) ;;
      *) echo "FAIL(no hardened runtime) $f"; fail=1 ;;
    esac
    if [[ "${REQUIRE_DEVID:-0}" == 1 ]]; then
      case "$info" in
        *"Authority=Developer ID Application"*) ;;
        *) echo "FAIL(not Developer ID) $f"; fail=1 ;;
      esac
      case "$info" in
        *"Timestamp="*) ;;
        *) echo "FAIL(no secure timestamp) $f"; fail=1 ;;
      esac
    fi
    if codesign -d --entitlements - --xml "$f" 2>/dev/null | grep -q get-task-allow; then
      echo "FAIL(get-task-allow) $f"; fail=1
    fi
  done
  return $fail
}
export -f verify_batch

verify_bundle() {
  codesign --verify --deep --strict --verbose=2 "$APP" 2>&1 | sed 's/^/    /' \
    || return 1
  xargs -0 -n 32 -P "$JOBS" bash -c 'verify_batch "$@"' _ < "$TMP_DIR/machos" \
    || return 1
}

# ===============================================================================
# notarize
# ===============================================================================

if [[ "$COMMAND" == "notarize" ]]; then
  # Before anything expensive, and before the pre-flight: uploading again would
  # only produce the ticket that is already on it.
  if [[ $FORCE -eq 0 ]] && already_notarized; then
    echo "==> Already notarized and stapled: nothing to upload"
    spctl -a -vv -t exec "$APP" 2>&1 | sed 's/^/    /'
    echo "    pass --force to submit it again anyway"
    exit 0
  fi

  export REQUIRE_DEVID=1
  echo "==> Pre-flight: verifying signatures are notarization-ready"
  verify_bundle \
    || die "bundle is not notarization-ready (see FAIL lines above); run 'codesign-bundle.sh sign' on it first"
  echo "    ${#MACHOS[@]} Mach-O file(s): Developer ID, hardened runtime, secure timestamp"

  if [[ "$DRY_RUN" == 1 ]]; then
    echo "==> Pre-flight passed; would pack a DMG and submit with $NOTARY_AUTH_LABEL"
    exit 0
  fi

  echo "==> Packing DMG"
  DMG="$TMP_DIR/$(basename "$APP" .app).dmg"
  hdiutil create -srcfolder "$APP" -volname "$(basename "$APP" .app)" \
    -fs HFS+ -format ULFO -ov -quiet "$DMG"
  echo "    $(du -h "$DMG" | cut -f1)"

  echo "==> Notarizing ($NOTARY_AUTH_LABEL)"
  echo "    the upload can take a while; from another shell, follow it with"
  echo "    xcrun notarytool info <submission id> and the same credentials"
  SUBMIT_LOG="$TMP_DIR/submit.log"
  # Under a pty notarytool draws its upload progress; through a plain pipe it
  # prints nothing at all until the upload finishes, which reads as a hang.
  # The output is deliberately not indented: `sed` would buffer the whole
  # carriage-return-redrawn progress line and undo the point of the pty.
  if [[ -t 1 ]]; then
    script -q /dev/null xcrun notarytool submit "$DMG" \
      "${NOTARY_AUTH[@]}" --wait 2>&1 | tee "$SUBMIT_LOG" || true
  else
    xcrun notarytool submit "$DMG" "${NOTARY_AUTH[@]}" --wait 2>&1 \
      | tee "$SUBMIT_LOG" || true
  fi
  # A pty terminates every line with CR LF; strip the CR before it rides along
  # into the submission id.
  sub_id="$(tr -d '\r' < "$SUBMIT_LOG" \
    | sed -n 's/^[[:space:]]*id: \(.*\)$/\1/p' | head -1)"
  if ! grep -q 'status: Accepted' "$SUBMIT_LOG"; then
    if [[ -n "$sub_id" ]]; then
      log_out="$(dirname "$APP")/$(basename "$APP" .app)-notarization-log.json"
      xcrun notarytool log "$sub_id" "${NOTARY_AUTH[@]}" "$log_out" \
        >/dev/null 2>&1 || true
      die "notarization was not accepted (submission $sub_id); log: $log_out"
    fi
    die "notarization submission failed (see output above)"
  fi
  echo "==> Stapling"
  xcrun stapler staple "$APP" | sed 's/^/    /'
  xcrun stapler validate "$APP" >/dev/null || die "stapler validate failed"
  echo "==> Gatekeeper assessment"
  spctl -a -vv -t exec "$APP" 2>&1 | sed 's/^/    /' \
    || die "Gatekeeper rejected the stapled bundle"

  echo "==> Done: $APP"
  echo "    Mach-O leaves verified: ${#MACHOS[@]}"
  echo "    notarization:  Accepted (submission $sub_id), stapled"
  exit 0
fi

# ===============================================================================
# sign
# ===============================================================================

# The main executable of a Contents/-style bundle is signed with the bundle,
# not as a leaf: given that path, codesign resolves it to the bundle and
# validates the bundle's subcomponents, and a PlugIns entry is not signed yet
# while the leaves are being done. The bundle pass signs the executable with
# the entitlements resolved against it (bundle_entitlements, MAIN_ENTITLEMENTS).
BUNDLE_EXECS=()
while IFS= read -r -d '' b; do
  if [[ -f "$b/Contents/Info.plist" ]]; then
    exec_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' \
      "$b/Contents/Info.plist" 2>/dev/null || true)"
    [[ -n "$exec_name" ]] && BUNDLE_EXECS+=("$b/Contents/MacOS/$exec_name")
  fi
done < <(printf '%s\0' "$APP"; cat "$TMP_DIR/bundles")
is_bundle_exec() {
  local e
  for e in ${BUNDLE_EXECS[@]+"${BUNDLE_EXECS[@]}"}; do
    if [[ "$1" == "$e" ]]; then return 0; fi
  done
  return 1
}

# The leaves to sign: every Mach-O that is not inside a skipped subtree and
# not a bundle's main executable.
SIGN_MACHOS=()
RELS=()
PLIST_FOR=()
SRC_FOR=()
SKIPPED_MACHOS=0
for f in "${MACHOS[@]}"; do
  rel="${f#"$APP"/}"
  if is_skipped "$rel"; then
    SKIPPED_MACHOS=$(( SKIPPED_MACHOS + 1 ))
    continue
  fi
  if is_bundle_exec "$f"; then continue; fi
  preserved=""
  if [[ "$PRESERVE_ENTITLEMENTS" == 1 ]]; then
    preserved="$(dump_entitlements "$f")"
    if [[ -n "$preserved" ]]; then
      if grep -q 'get-task-allow' "$preserved"; then
        die "refusing to re-sign $rel: the entitlements already on it include com.apple.security.get-task-allow, which the notary service hard-rejects"
      fi
      plutil -lint "$preserved" >/dev/null 2>&1 \
        || die "could not parse the entitlements already on $rel"
    fi
  fi
  res="$(resolve_entitlements "$rel" "$preserved")"
  SIGN_MACHOS+=("$f")
  RELS+=("$rel")
  SRC_FOR+=("${res%%$'\t'*}")
  PLIST_FOR+=("${res#*$'\t'}")
done

# Nested code bundles must be signed as bundles (deepest first) after their
# leaves, so every signature seals already-signed content.
NESTED=()
DEPTHS=()
while IFS= read -r -d '' d; do
  if is_skipped "${d#"$APP"/}"; then continue; fi
  slashes="${d//[^\/]/}"
  NESTED+=("$d")
  DEPTHS+=("${#slashes}")
done < "$TMP_DIR/bundles"

n="${#NESTED[@]}"
for ((i = 0; i < n; i++)); do
  max=$i
  for ((j = i + 1; j < n; j++)); do
    if (( DEPTHS[j] > DEPTHS[max] )); then max=$j; fi
  done
  if (( max != i )); then
    tmp="${NESTED[i]}"; NESTED[i]="${NESTED[max]}"; NESTED[max]="$tmp"
    tmp="${DEPTHS[i]}"; DEPTHS[i]="${DEPTHS[max]}"; DEPTHS[max]="$tmp"
  fi
done

# A *.bundle/*.plugin directory is only signable if it is actually a code
# bundle; some are plain data directories.
bundle_has_plist() {
  [[ -f "$1/Contents/Info.plist" || -f "$1/Resources/Info.plist" || -f "$1/Info.plist" ]]
}

# Entitlements for a bundle are resolved against its main executable's path.
# Prints "<source>\t<plist>", like resolve_entitlements.
bundle_entitlements() {
  local b="$1" exec_name exec_path preserved=""
  if [[ -f "$b/Contents/Info.plist" ]]; then
    exec_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' \
      "$b/Contents/Info.plist" 2>/dev/null || true)"
    if [[ -n "$exec_name" ]]; then
      exec_path="$b/Contents/MacOS/$exec_name"
      if [[ "$PRESERVE_ENTITLEMENTS" == 1 && -f "$exec_path" ]]; then
        preserved="$(dump_entitlements "$exec_path")"
      fi
      resolve_entitlements "${b#"$APP"/}/Contents/MacOS/$exec_name" "$preserved"
      return 0
    fi
  fi
  printf 'none\t'
}

# Number of keys in an entitlements plist, for reporting.
ent_key_count() {
  local n
  n="$(tr '<' '\n' < "$1" | grep -c '^key>' || true)"
  printf '%s' "${n:-0}"
}

# How a resolved entitlements set is reported.
ent_label() {
  local src="$1" plist="$2"
  case "$src" in
    preserved) printf 'preserved: %s key(s) already on the file' "$(ent_key_count "$plist")" ;;
    override)  printf '%s (override)' "$plist" ;;
    *)         printf 'no entitlements' ;;
  esac
}

print_skipped() {
  local i
  [[ "${#SKIP_ROOTS[@]}" -gt 0 ]] || return 0
  echo "==> Leaving ${#SKIP_ROOTS[@]} subtree(s) unsigned ($SKIPPED_MACHOS Mach-O file(s), verified but never re-signed):"
  for ((i = 0; i < ${#SKIP_ROOTS[@]}; i++)); do
    printf '    %s\t[%s]\n' "${SKIP_ROOTS[i]}" "${SKIP_REASONS[i]}"
  done
}

MAIN_EXEC="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' \
  "$APP/Contents/Info.plist" 2>/dev/null || true)"
[[ -n "$MAIN_EXEC" ]] || die "no CFBundleExecutable in $APP/Contents/Info.plist"
MAIN_PRESERVED=""
if [[ "$PRESERVE_ENTITLEMENTS" == 1 && -f "$APP/Contents/MacOS/$MAIN_EXEC" ]]; then
  MAIN_PRESERVED="$(dump_entitlements "$APP/Contents/MacOS/$MAIN_EXEC")"
fi
MAIN_RES="$(resolve_entitlements "Contents/MacOS/$MAIN_EXEC" "$MAIN_PRESERVED")"
MAIN_SRC="${MAIN_RES%%$'\t'*}"
MAIN_ENTITLEMENTS="${MAIN_RES#*$'\t'}"

if [[ "$DRY_RUN" == 1 ]]; then
  echo "==> Would sign ${#SIGN_MACHOS[@]} Mach-O file(s):"
  for ((i = 0; i < ${#SIGN_MACHOS[@]}; i++)); do
    printf '    %s\t[%s]\n' "${RELS[i]}" \
      "$(ent_label "${SRC_FOR[i]}" "${PLIST_FOR[i]}")"
  done
  echo "==> Would sign ${#NESTED[@]} nested bundle(s) (deepest first):"
  for b in ${NESTED[@]+"${NESTED[@]}"}; do
    if bundle_has_plist "$b"; then
      res="$(bundle_entitlements "$b" || true)"
      printf '    %s\t[%s]\n' "${b#"$APP"/}" \
        "$(ent_label "${res%%$'\t'*}" "${res#*$'\t'}")"
    else
      printf '    %s\t[skipped: no Info.plist, data directory]\n' "${b#"$APP"/}"
    fi
  done
  print_skipped
  printf '==> Would sign %s\t[%s]\n' "$(basename "$APP")" \
    "$(ent_label "$MAIN_SRC" "$MAIN_ENTITLEMENTS")"
  exit 0
fi

# --- sign ------------------------------------------------------------------

echo "==> Stripping extended attributes"
xattr -cr "$APP" 2>/dev/null \
  || echo "    (xattr -cr reported errors; continuing, codesign will catch real detritus)"

# Group leaves by entitlements so each group is one parallel xargs run. Files
# that carried identical entitlements share one dumped plist, so preserving
# does not fragment the groups.
UNIQ_PLISTS=()
UNIQ_SRCS=()
UNIQ_COUNTS=()
for ((i = 0; i < ${#PLIST_FOR[@]}; i++)); do
  p="${PLIST_FOR[i]}"
  found=-1
  for ((u = 0; u < ${#UNIQ_PLISTS[@]}; u++)); do
    if [[ "${UNIQ_PLISTS[u]}" == "$p" ]]; then found=$u; fi
  done
  if (( found >= 0 )); then
    UNIQ_COUNTS[found]=$(( UNIQ_COUNTS[found] + 1 ))
  else
    UNIQ_PLISTS+=("$p")
    UNIQ_SRCS+=("${SRC_FOR[i]}")
    UNIQ_COUNTS+=(1)
  fi
done

print_skipped

echo "==> Signing ${#SIGN_MACHOS[@]} Mach-O file(s) (jobs: $JOBS)"
for ((u = 0; u < ${#UNIQ_PLISTS[@]}; u++)); do
  plist="${UNIQ_PLISTS[u]}"
  SIGN_ARGS=(--force --sign "$IDENTITY" --options runtime "$TIMESTAMP_FLAG")
  if [[ -n "$plist" ]]; then
    SIGN_ARGS+=(--entitlements "$plist")
    echo "    ${UNIQ_COUNTS[u]} file(s) with entitlements: $(ent_label "${UNIQ_SRCS[u]}" "$plist")"
  else
    echo "    ${UNIQ_COUNTS[u]} file(s) with hardened runtime only"
  fi
  : > "$TMP_DIR/group"
  for ((i = 0; i < ${#SIGN_MACHOS[@]}; i++)); do
    if [[ "${PLIST_FOR[i]}" == "$plist" ]]; then
      printf '%s\0' "${SIGN_MACHOS[i]}" >> "$TMP_DIR/group"
    fi
  done
  xargs -0 -n 16 -P "$JOBS" codesign "${SIGN_ARGS[@]}" < "$TMP_DIR/group" \
    || die "codesign failed for a Mach-O leaf (see above)"
done

if [[ "${#NESTED[@]}" -gt 0 ]]; then
  echo "==> Signing ${#NESTED[@]} nested bundle(s) (deepest first)"
  for b in "${NESTED[@]}"; do
    if ! bundle_has_plist "$b"; then
      echo "    skipping data directory: ${b#"$APP"/}"
      continue
    fi
    SIGN_ARGS=(--force --sign "$IDENTITY" --options runtime "$TIMESTAMP_FLAG")
    res="$(bundle_entitlements "$b")"
    e="${res#*$'\t'}"
    if [[ -n "$e" ]]; then SIGN_ARGS+=(--entitlements "$e"); fi
    echo "    ${b#"$APP"/}"
    codesign "${SIGN_ARGS[@]}" "$b" || die "codesign failed for nested bundle: $b"
  done
fi

echo "==> Signing $(basename "$APP")"
SIGN_ARGS=(--force --sign "$IDENTITY" --options runtime "$TIMESTAMP_FLAG")
if [[ -n "$MAIN_ENTITLEMENTS" ]]; then SIGN_ARGS+=(--entitlements "$MAIN_ENTITLEMENTS"); fi
codesign "${SIGN_ARGS[@]}" "$APP" || die "codesign failed for $APP"

# --- verify ------------------------------------------------------------------

export REQUIRE_DEVID=$(( ADHOC == 1 ? 0 : 1 ))
echo "==> Verifying"
verify_bundle || die "verification failed (see FAIL lines above)"

# --- summary -------------------------------------------------------------------

echo "==> Done: $APP"
if [[ "$ADHOC" == 1 ]]; then
  echo "    identity:      ad-hoc (NOT distributable, NOT notarizable)"
else
  echo "    identity:      $IDENTITY"
fi
echo "    Mach-O leaves: ${#SIGN_MACHOS[@]} signed, ${#MACHOS[@]} verified"
for ((u = 0; u < ${#UNIQ_PLISTS[@]}; u++)); do
  echo "        ${UNIQ_COUNTS[u]} x $(ent_label "${UNIQ_SRCS[u]}" "${UNIQ_PLISTS[u]}")"
done
echo "    nested bundles: ${#NESTED[@]}"
if [[ "${#SKIP_ROOTS[@]}" -gt 0 ]]; then
  echo "    left unsigned:  ${#SKIP_ROOTS[@]} subtree(s), $SKIPPED_MACHOS Mach-O file(s)"
fi
if [[ "$ADHOC" == 0 ]]; then
  echo "    next:          ${BASH_SOURCE[0]} notarize \"$APP\""
fi
