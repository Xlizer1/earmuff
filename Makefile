# User install into ~/.local by default: make install PREFIX=/usr/local for a system-wide one.
# Packages set DESTDIR to their staging folder.
PREFIX ?= $(HOME)/.local
DESTDIR ?=
APP_ID := io.github.xlizer1.Earmuff
UDEV_RULES := /etc/udev/rules.d/70-earmuff.rules

.PHONY: build check test test-hardware screenshots install uninstall install-udev uninstall-udev

build:
	cargo build --release

# What CI runs
check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace
	appstreamcli validate --no-net data/$(APP_ID).metainfo.xml
	desktop-file-validate data/$(APP_ID).desktop

test:
	cargo test --workspace

# Needs a HyperX Cloud Alpha Wireless, switched on; restores its settings afterwards
test-hardware:
	cargo test -p earmuff-infrastructure --test hardware -- --ignored --test-threads=1

# Retakes the pictures in the README, with simulated headsets on the headless test display
screenshots:
	EARMUFF_SCREENSHOTS=$(CURDIR)/docs/screenshots cargo test -p earmuff --test screenshots

# The desktop entry gets the full path: app launchers often lack ~/.local/bin in their PATH
install: build
	install -Dm755 target/release/earmuff $(DESTDIR)$(PREFIX)/bin/earmuff
	install -d $(DESTDIR)$(PREFIX)/share/applications
	sed 's|^Exec=earmuff|Exec=$(PREFIX)/bin/earmuff|' data/$(APP_ID).desktop > $(DESTDIR)$(PREFIX)/share/applications/$(APP_ID).desktop
	install -Dm644 data/$(APP_ID).metainfo.xml $(DESTDIR)$(PREFIX)/share/metainfo/$(APP_ID).metainfo.xml
	install -Dm644 data/fonts/OFL.txt $(DESTDIR)$(PREFIX)/share/licenses/earmuff/IBMPlexSans-OFL.txt
	install -Dm644 data/icons/scalable/apps/$(APP_ID).svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg
	install -Dm644 data/icons/scalable/apps/$(APP_ID)-symbolic.svg $(DESTDIR)$(PREFIX)/share/icons/hicolor/symbolic/apps/$(APP_ID)-symbolic.svg

uninstall:
	rm -f $(PREFIX)/bin/earmuff $(PREFIX)/share/applications/$(APP_ID).desktop $(PREFIX)/share/metainfo/$(APP_ID).metainfo.xml
	rm -rf $(PREFIX)/share/licenses/earmuff
	rm -f $(PREFIX)/share/icons/hicolor/scalable/apps/$(APP_ID).svg $(PREFIX)/share/icons/hicolor/symbolic/apps/$(APP_ID)-symbolic.svg

# Not needed when HeadsetControl is installed: its udev rule covers the same dongles
install-udev:
	sudo install -Dm644 data/udev/70-earmuff.rules $(UDEV_RULES)
	sudo udevadm control --reload
	sudo udevadm trigger --subsystem-match=hidraw

uninstall-udev:
	sudo rm -f $(UDEV_RULES)
	sudo udevadm control --reload
