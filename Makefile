# Build and install Backslash.
#   make && sudo make install            → /usr/local
#   make install PREFIX=$HOME/.local     → per-user install
#   make install DESTDIR=pkg PREFIX=/usr → for distribution packages

PREFIX ?= /usr/local
DESTDIR ?=
SHARE := $(DESTDIR)$(PREFIX)/share

.PHONY: all check install uninstall

all:
	cargo build --release --locked

check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test
	deno fmt --check
	deno lint
	deno check sdk/*.ts extensions/*/commands/*.ts crates/backslash-core/tests/fixtures/*/commands/*.ts

install:
	install -Dm755 target/release/backslash $(DESTDIR)$(PREFIX)/bin/backslash
	install -d $(SHARE)/backslash/sdk $(SHARE)/backslash/extensions
	install -m644 sdk/*.ts $(SHARE)/backslash/sdk/
	cp -r extensions/. $(SHARE)/backslash/extensions/
	install -Dm644 data/app.backslash.Backslash.desktop $(SHARE)/applications/app.backslash.Backslash.desktop
	install -Dm644 data/icons/backslash.png $(SHARE)/icons/hicolor/512x512/apps/app.backslash.Backslash.png

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/backslash
	rm -rf $(SHARE)/backslash
	rm -f $(SHARE)/applications/app.backslash.Backslash.desktop
	rm -f $(SHARE)/icons/hicolor/512x512/apps/app.backslash.Backslash.png
