.DEFAULT_GOAL := help

.PHONY: help build test run install uninstall package benchmark review

help:
	@cargo xtask help

build:
	cargo build --workspace --release --locked

test:
	cargo test --workspace --all-targets --locked
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	cargo xtask localization

run:
	cargo run -p unbloat --locked -- --dev-run

install:
	cargo xtask install

uninstall:
	cargo xtask uninstall

package: build
	cargo xtask package

benchmark:
	cargo run -p unbloat --release --locked --example benchmark

review:
	@cargo xtask review
