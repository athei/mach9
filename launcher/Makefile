# Builds Mach9.app's launcher and Dock tile plug-in and assembles the app.
# Run it from anywhere as `make -C launcher <target>`; every path is relative
# to this file.
#
#   make app WINE_RUNTIME=<dir>   a complete, unsigned Mach9.app in STAGE
#   make install                  the launcher's parts into the existing APP,
#                                 signed, for development
#   make build                    the launcher and the plug-in only
#   make clean                    everything this file built or downloaded
#
# Prereq: rustup target add aarch64-apple-darwin

LAUNCHER := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
ROOT     := $(abspath $(LAUNCHER)/..)
SCRIPTS  := $(LAUNCHER)/scripts
PLIST    := /usr/libexec/PlistBuddy

# Everything built or downloaded here goes under the repository's ignored
# target/, next to cargo's own output.
BUILD := $(ROOT)/target/launcher

# Bundle `make install` writes into. It has to exist already, with the Wine
# runtime that bin/bundle-wine.sh --runtime-only writes into
# Contents/Resources/wine; `install` only adds the launcher's parts.
# Override it on the command line to install somewhere else.
APP := /Applications/Mach9.app

# Where `make app` assembles the bundle, and the Wine runtime it copies in:
# a tree bin/bundle-wine.sh --runtime-only wrote, without development files.
STAGE        := $(BUILD)/stage
WINE_RUNTIME :=

BIN := $(BUILD)/gamelauncher

# The Dock tile plug-in: a loadable bundle the Dock runs while the app is
# pinned and not running.
PLUGIN_NAME  := GameLauncherDockTile
PLUGIN       := $(BUILD)/$(PLUGIN_NAME).docktileplugin
PLUGIN_BIN   := $(PLUGIN)/Contents/MacOS/$(PLUGIN_NAME)
PLUGIN_PLIST := $(LAUNCHER)/docktile/Info.plist

# The updater framework. Not committed: scripts/fetch-sparkle.sh downloads one
# pinned release and checks it against a known SHA-256. The release build
# links against the copy here (app/build.rs reads SPARKLE_DIR) and resolves
# the copy in Contents/Frameworks at run time.
SPARKLE_DIR   := $(BUILD)/sparkle
SPARKLE_FW    := $(SPARKLE_DIR)/Sparkle.framework
SPARKLE_STAMP := $(SPARKLE_DIR)/.version

# The app's Info.plist. There is one bundle, so this file is the whole truth
# about it, apart from the version. It carries the identifier, the games
# category and Game Mode keys, and the Sparkle feed and public key.
INFO_SRC := $(LAUNCHER)/app/Info.plist

# The version stamped into the app and the plug-in (scripts/version.sh): the
# workspace version in the root Cargo.toml unless given here. A release run
# passes the tag's, X.Y.Z or X.Y.Z-rc.N.
VERSION :=
ICON     := $(LAUNCHER)/app/mach9.icns
APP_NAME := $(shell $(PLIST) -c 'Print :CFBundleName' "$(INFO_SRC)")
STAGE_APP = $(STAGE)/$(APP_NAME).app

# The only architecture: the bundled wine is the arm64 build, so the app runs
# on Apple silicon only. cargo runs at the repository root, so
# .cargo/config.toml, which pins the deployment target, applies.
TARGET      := aarch64-apple-darwin
TARGET_DIR  := $(ROOT)/target
RELEASE     := $(TARGET_DIR)/$(TARGET)/release
CARGO_FLAGS := --release --quiet --locked
CARGO       := cd "$(ROOT)" && CARGO_TARGET_DIR="$(TARGET_DIR)"

.PHONY: all build cargo-build app install clean

all: build

build: $(BIN) $(PLUGIN_BIN)

$(SPARKLE_STAMP): $(SCRIPTS)/fetch-sparkle.sh
	SPARKLE_DIR="$(SPARKLE_DIR)" $(SCRIPTS)/fetch-sparkle.sh

# Both packages are built by one cargo run. It runs on every build and lets
# cargo decide what is out of date.
cargo-build: $(SPARKLE_STAMP)
	mkdir -p $(BUILD)
	$(CARGO) SPARKLE_DIR="$(SPARKLE_DIR)" cargo build $(CARGO_FLAGS) -p gamelauncher -p gamelauncher-docktile --target $(TARGET)

# Removed before the copy, so the copy is a new file: the linker signs arm64
# code ad hoc, and the kernel can keep the signature of the file a copy
# overwrites in place, then kill the new binary for not matching it.
$(BIN): cargo-build
	rm -f $@
	cp $(RELEASE)/gamelauncher $@

$(PLUGIN_BIN): cargo-build $(PLUGIN_PLIST)
	mkdir -p $(dir $@)
	rm -f $@
	cp $(RELEASE)/libdocktile.dylib $@
	cp $(PLUGIN_PLIST) $(PLUGIN)/Contents/Info.plist

# Puts the launcher's parts into the bundle $(1): the launcher, the Dock tile
# plug-in, Sparkle, the icon and the Info.plist, with the version stamped
# into both Info.plists. The executable is removed before the copy for the
# reason given at $(BIN).
#
# The plug-in's principal class has to be unique per installed bundle,
# because every pinned copy of the app loads into the same Dock plug-in host,
# and the plug-in finds the app it belongs to by that name. The rule here
# (prefix plus the bundle id with everything but letters and digits turned
# into underscores) is the one core/src/docktile.rs `class_name` implements.
define populate
	mkdir -p "$(1)/Contents/MacOS" "$(1)/Contents/Resources" "$(1)/Contents/Frameworks" "$(1)/Contents/PlugIns"
	rm -f "$(1)/Contents/MacOS/gamelauncher"
	cp $(BIN) "$(1)/Contents/MacOS/gamelauncher"
	rm -rf "$(1)/Contents/PlugIns/$(PLUGIN_NAME).docktileplugin"
	cp -R "$(PLUGIN)" "$(1)/Contents/PlugIns/"
	rm -rf "$(1)/Contents/Frameworks/Sparkle.framework"
	ditto "$(SPARKLE_FW)" "$(1)/Contents/Frameworks/Sparkle.framework"
	cp "$(ICON)" "$(1)/Contents/Resources/"
	cp "$(INFO_SRC)" "$(1)/Contents/Info.plist"
	$(SCRIPTS)/version.sh --stamp "$(1)" $(VERSION)
	$(SCRIPTS)/version.sh --stamp "$(1)/Contents/PlugIns/$(PLUGIN_NAME).docktileplugin" $(VERSION)
	@set -e; \
	app_id="$$($(PLIST) -c 'Print :CFBundleIdentifier' "$(1)/Contents/Info.plist")"; \
	class="$(PLUGIN_NAME)_$$(printf '%s' "$$app_id" | tr -c '[:alnum:]' '_')"; \
	plist="$(1)/Contents/PlugIns/$(PLUGIN_NAME).docktileplugin/Contents/Info.plist"; \
	$(PLIST) -c "Set :CFBundleIdentifier $$app_id.docktile" "$$plist"; \
	$(PLIST) -c "Set :NSPrincipalClass $$class" "$$plist"
endef

# A complete bundle from nothing: the Wine runtime first, then the launcher's
# parts. Left unsigned (apart from the ad-hoc signatures the linker gives the
# two binaries and the signatures the Wine tree and Sparkle carry), for
# scripts/codesign-bundle.sh sign or scripts/make-release.sh.
app: build
	@test -n "$(WINE_RUNTIME)" \
	  || { echo "error: set WINE_RUNTIME to a tree bin/bundle-wine.sh --runtime-only wrote" >&2; exit 1; }
	@test -d "$(WINE_RUNTIME)/lib/wine/aarch64-unix/wine.app" \
	  || { echo "error: $(WINE_RUNTIME) has no lib/wine/aarch64-unix/wine.app, so it is no Wine runtime" >&2; exit 1; }
	@test ! -e "$(WINE_RUNTIME)/lib/wine/tests" && test ! -e "$(WINE_RUNTIME)/lib/wine/x86_64-windows" \
	  || { echo "error: $(WINE_RUNTIME) holds development files; bundle it with --runtime-only" >&2; exit 1; }
	rm -rf "$(STAGE_APP)"
	mkdir -p "$(STAGE_APP)/Contents/Resources"
	ditto "$(WINE_RUNTIME)" "$(STAGE_APP)/Contents/Resources/wine"
	$(call populate,$(STAGE_APP))
	@echo "Assembled $(STAGE_APP)"

# The bundle directory is touched last. Launch Services decides whether its
# record of a bundle is out of date by the bundle directory's modification
# date, and copying into an existing bundle leaves that date alone (codesign
# only changes Contents/). Game Mode's gamepolicyd runs as root with a Launch
# Services database of its own; without the touch it keeps the record it had
# before the Info.plist changed, and a new Game Mode key never reaches it.
install: build
	@test -d "$(APP)/Contents/Resources/wine" \
	  || { echo "error: $(APP) has no Wine runtime in Contents/Resources/wine; use 'make app' for a new bundle" >&2; exit 1; }
	$(call populate,$(APP))
	$(SCRIPTS)/codesign-bundle.sh sign "$(APP)"
	touch "$(APP)"
	@echo "Dock menu: if $(notdir $(APP)) was pinned before it had the plug-in, remove it from the Dock and add it again;"
	@echo "otherwise 'killall Dock' picks up this build. The app must have been opened once."

clean:
	rm -rf $(BUILD) $(TARGET_DIR)/$(TARGET)
