.PHONY: build test run install uninstall package benchmark

build:
	cargo build --workspace --release --locked

test:
	cargo test --workspace --all-targets --locked
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked -- -D warnings

run:
	cargo run -p unbloat --locked -- --dev-run

install: build
	powershell -NoProfile -ExecutionPolicy Bypass -File scripts/install.ps1

uninstall:
	powershell -NoProfile -ExecutionPolicy Bypass -File scripts/uninstall.ps1

package: build
	powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1

benchmark:
	cargo run -p unbloat --release --locked --example benchmark
