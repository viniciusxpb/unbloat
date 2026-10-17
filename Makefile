.DEFAULT_GOAL := help

# A 32-bit make redirects System32 to SysWOW64; Sysnative reaches 64-bit PowerShell.
WINDOWS_ROOT := $(subst \,/,$(or $(SystemRoot),$(SYSTEMROOT),$(windir),$(WINDIR)))
POWERSHELL := $(WINDOWS_ROOT)/$(if $(PROCESSOR_ARCHITEW6432),Sysnative,System32)/WindowsPowerShell/v1.0/powershell.exe

.PHONY: help build test run install uninstall package benchmark

help:
	@"$(POWERSHELL)" -NoProfile -ExecutionPolicy Bypass -File scripts/help.ps1

build:
	cargo build --workspace --release --locked

test:
	cargo test --workspace --all-targets --locked
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	"$(POWERSHELL)" -NoProfile -ExecutionPolicy Bypass -File scripts/test-localization.ps1 -SkipWindows

run:
	cargo run -p unbloat --locked -- --dev-run

install: build
	"$(POWERSHELL)" -NoProfile -ExecutionPolicy Bypass -File scripts/install.ps1

uninstall:
	"$(POWERSHELL)" -NoProfile -ExecutionPolicy Bypass -File scripts/uninstall.ps1

package: build
	"$(POWERSHELL)" -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1

benchmark:
	cargo run -p unbloat --release --locked --example benchmark
