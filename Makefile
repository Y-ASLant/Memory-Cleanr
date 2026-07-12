.PHONY: build check clean fmt test

fmt:
	cargo fmt --all

check:
	cargo clippy --release -- -D warnings

test:
	cargo test

build: check
	cargo build --release

clean:
	cargo clean
	-if exist dist rmdir /s /q dist
