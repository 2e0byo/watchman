.PHONY: install

target/release/watchman:
	cargo build -r

install: target/release/watchman
	mkdir ~/.local/bin || true
	cp target/release/watchman ~/.local/bin/
