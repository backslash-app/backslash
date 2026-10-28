# Build and install Backslash.
#   make && sudo make install            → /usr/local
#   make install PREFIX=$HOME/.local     → per-user install
#   make install DESTDIR=pkg PREFIX=/usr → for distribution packages
#   make dist                            → dist/*.tar.gz, .deb and .rpm (needs nfpm)

PREFIX ?= /usr/local
DESTDIR ?=
SHARE := $(DESTDIR)$(PREFIX)/share
VERSION ?= $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH ?= $(shell uname -m)
DEB_ARCH := $(if $(filter aarch64,$(ARCH)),arm64,amd64)

.PHONY: all check install uninstall dist

all:
	cargo build --release --locked

check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test
	deno fmt --check
	deno lint
	deno check sdk/*.ts extensions/*/commands/*.ts crates/backslash-core/tests/fixtures/*/commands/*.ts
	python3 -m py_compile sdk/python/*.py crates/backslash-core/tests/fixtures/pydemo/commands/*.py

install:
	install -Dm755 target/release/backslash $(DESTDIR)$(PREFIX)/bin/backslash
	install -d $(SHARE)/backslash/sdk $(SHARE)/backslash/extensions
	install -m644 sdk/*.ts $(SHARE)/backslash/sdk/
	install -d $(SHARE)/backslash/sdk/python
	install -m644 sdk/python/*.py $(SHARE)/backslash/sdk/python/
	cp -r extensions/. $(SHARE)/backslash/extensions/
	install -Dm644 data/app.backslash.Backslash.desktop $(SHARE)/applications/app.backslash.Backslash.desktop
	install -Dm644 data/icons/backslash.png $(SHARE)/icons/hicolor/512x512/apps/app.backslash.Backslash.png
	install -Dm644 data/icons/backslash.svg $(SHARE)/icons/hicolor/scalable/apps/app.backslash.Backslash.svg
	install -Dm644 data/app.backslash.Backslash.metainfo.xml $(SHARE)/metainfo/app.backslash.Backslash.metainfo.xml

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/backslash
	# Only what install put there: with PREFIX=~/.local, $(SHARE)/backslash is
	# also where your installed extensions and their data live.
	rm -rf $(SHARE)/backslash/sdk
	for e in extensions/*/; do rm -rf "$(SHARE)/backslash/extensions/$$(basename $$e)"; done
	rm -f $(SHARE)/applications/app.backslash.Backslash.desktop
	rm -f $(SHARE)/icons/hicolor/512x512/apps/app.backslash.Backslash.png
	rm -f $(SHARE)/icons/hicolor/scalable/apps/app.backslash.Backslash.svg
	rm -f $(SHARE)/metainfo/app.backslash.Backslash.metainfo.xml

# Release files. The tarball unpacks into any prefix: `tar -xzf … -C ~/.local`.
dist: all
	rm -rf dist
	$(MAKE) install DESTDIR=dist/root PREFIX=/usr
	tar -czf dist/backslash-$(VERSION)-$(ARCH)-linux.tar.gz -C dist/root/usr --owner=0 --group=0 .
	ARCH=$(DEB_ARCH) VERSION=$(VERSION) nfpm package -f packaging/nfpm.yaml -p deb -t dist/
	ARCH=$(DEB_ARCH) VERSION=$(VERSION) nfpm package -f packaging/nfpm.yaml -p rpm -t dist/
	cd dist && sha256sum *.tar.gz *.deb *.rpm > SHA256SUMS
