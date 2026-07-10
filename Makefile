.PHONY: test check fmt install serve-book

test:
	+cargo test --workspace

check:
	+cargo clippy --workspace --all-targets -- -D warnings

fmt:
	+cargo fmt --all

install:
	+cargo install --path crates/mensura-cli

# Live-reload the book locally.  Builds the `mensura-mdbook` preprocessor first
# so `mdbook serve` (which invokes it) check-gates every ```mensura block on
# the first request rather than failing part way in.
serve-book:
	+cargo build --package mensura-mdbook
	mdbook serve book
