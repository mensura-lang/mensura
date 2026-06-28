.PHONY: test check fmt install

test:
	+cargo test --workspace

check:
	+cargo clippy --workspace --all-targets -- -D warnings

fmt:
	+cargo fmt --all

install:
	+cargo install --path crates/mensura-cli
