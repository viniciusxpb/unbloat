.DEFAULT_GOAL := help

.PHONY: help build test run install uninstall package benchmark

help:
	@echo Uso: make [comando]
	@echo ------------------------------------------------------------
	@echo Comandos disponíveis:
	@echo   help        Mostra esta ajuda.
	@echo   build       Compila o executável e a DLL em modo release.
	@echo   test        Executa os testes e verifica formatação e clippy.
	@echo   run         Executa usando development.test_input do config.toml.
	@echo   install     Compila e instala a integração para o usuário atual.
	@echo   uninstall   Remove a integração e os arquivos da instalação.
	@echo   package     Compila e gera a distribuição e o ZIP em dist/.
	@echo   benchmark   Executa o benchmark e gera o relatório em target/benchmark/.

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
