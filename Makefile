.DEFAULT_GOAL := help

# install/uninstall run the Windows installer scripts.
# A 32-bit make redirects System32 to SysWOW64; Sysnative reaches 64-bit PowerShell.
WINDOWS_ROOT := $(subst \,/,$(or $(SystemRoot),$(SYSTEMROOT),$(windir),$(WINDIR)))
POWERSHELL := $(WINDOWS_ROOT)/$(if $(PROCESSOR_ARCHITEW6432),Sysnative,System32)/WindowsPowerShell/v1.0/powershell.exe

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

install: build
	"$(POWERSHELL)" -NoProfile -ExecutionPolicy Bypass -File scripts/install.ps1

uninstall:
	"$(POWERSHELL)" -NoProfile -ExecutionPolicy Bypass -File scripts/uninstall.ps1

package: build
	cargo xtask package

benchmark:
	cargo run -p unbloat --release --locked --example benchmark

review:
	@cargo xtask review
