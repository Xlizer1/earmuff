# User install into ~/.local by default: make install PREFIX=/usr/local for a system-wide one.
PREFIX ?= $(HOME)/.local
APP_ID := io.github.xlizer1.Earmuff
UDEV_RULES := /etc/udev/rules.d/70-earmuff.rules

.PHONY: build check test test-hardware install uninstall install-udev uninstall-udev

build:
	cargo build --release

# What CI runs
check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace

test:
	cargo test --workspace

# Needs a HyperX Cloud Alpha Wireless, switched on; restores its settings afterwards
test-hardware:
	cargo test -p earmuff-infrastructure --test hardware -- --ignored --test-threads=1

# The desktop entry gets the full path: app launchers often lack ~/.local/bin in their PATH
install: build
	install -Dm755 target/release/earmuff $(PREFIX)/bin/earmuff
	install -d $(PREFIX)/share/applications
	sed 's|^Exec=earmuff|Exec=$(PREFIX)/bin/earmuff|' data/$(APP_ID).desktop > $(PREFIX)/share/applications/$(APP_ID).desktop

uninstall:
	rm -f $(PREFIX)/bin/earmuff $(PREFIX)/share/applications/$(APP_ID).desktop

# Not needed when HeadsetControl is installed: its udev rule covers the same dongles
install-udev:
	sudo install -Dm644 data/udev/70-earmuff.rules $(UDEV_RULES)
	sudo udevadm control --reload
	sudo udevadm trigger --subsystem-match=hidraw

uninstall-udev:
	sudo rm -f $(UDEV_RULES)
	sudo udevadm control --reload
