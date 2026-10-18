# UNBLOAT

**Ship the code. Not the node_modules.**

Clique com o botão direito numa pasta, escolha **Compactar com Unbloat** e pronto:
você recebe um `.7z` só com o que importa, respeitando o `.gitignore` e o `.ignore`
do projeto. Nada de `node_modules`, `target/` ou `.git` no pacote.

- Windows 10/11 x64, escrito em Rust.
- Não precisa de Git, Rust nem 7-Zip instalados.
- Não precisa de admin.
- Sem rede, sem telemetria.

## Instalar

1. Baixe e extraia `Unbloat-<versão>-windows-x64.zip`.
2. Abra o `unbloat.exe`.
3. Clique em **Instalar integração**.

Fica instalado em `%LOCALAPPDATA%\Programs\Unbloat`. Pela linha de comando:

```powershell
.\unbloat.exe --install
```

Para desinstalar, abra o `unbloat.exe` e clique em **Desinstalar**, ou:

```powershell
& "$env:LOCALAPPDATA\Programs\Unbloat\unbloat.exe" --uninstall
```

Não precisa reiniciar o Explorer nem sair da sessão.

## Usar

Selecione um ou mais arquivos/pastas no mesmo diretório, clique com o botão direito
e escolha **Compactar com Unbloat** (no Windows 11, fica em **Mostrar mais opções**).

Uma janela mostra o progresso e, no fim, um resumo com o tamanho final e o espaço
economizado. Dá para cancelar a qualquer momento.

O que é gerado:

| Seleção                 | Resultado                       |
| ----------------------- | ------------------------------- |
| Uma pasta               | `pasta-unbloat.7z`              |
| Um arquivo              | `foto.png-unbloat.7z`           |
| Vários itens            | `Unbloat-AAAAMMDD-HHMMSS.7z`    |

Se o nome já existir, ganha `-2`, `-3` etc. Também dá para usar direto pelo terminal:

```powershell
unbloat.exe 'C:\projetos\frontend' 'C:\projetos\backend'
```

## O que entra e o que fica de fora

- Regras de `.gitignore` e `.ignore` (inclusive aninhados) são respeitadas.
- `.git` nunca entra. Arquivos ocultos entram.
- Links simbólicos, junctions e arquivos de nuvem (OneDrive etc.) ficam de fora.
- Pastas e arquivos vazios são preservados. Permissões e datas originais, não.
- Se algum arquivo mudar durante a compactação, a operação é abortada.
- O `.7z` só aparece depois de verificado (CRC). Se der erro ou você cancelar, o arquivo temporário é removido.

A compressão (LZMA2, nível, tamanho do bloco) é configurada em `config.toml`,
ao lado do executável.

## Desenvolver

Requisitos: Windows x64, Rust **1.93+** e Visual Studio Build Tools (MSVC + Windows SDK).
O `make` é opcional; dá para usar o Cargo direto.

```powershell
make build      # build release
make test       # testes + rustfmt + clippy
make run        # roda com a pasta de teste do config.toml
make install    # gera o pacote e instala a partir dele
make uninstall  # desinstala
make package    # gera dist/Unbloat-<versão>-windows-x64.zip
make benchmark  # benchmark, relatório em target/benchmark/RESULTS.md
make review     # copia instruções de review + diff staged
```

Sem `make`: `cargo xtask help` mostra os mesmos comandos.

Os textos da interface ficam em `locales/pt-BR.txt` e `locales/en.txt`. O idioma
segue o do Windows; para forçar, use `$env:UNBLOAT_LANG = 'en'` (ou `'pt-BR'`).

Para lançar uma versão nova, mude só o `version` em `[workspace.package]` no
`Cargo.toml` e rode:

```powershell
cargo check --workspace
make test
make package
```

Mais detalhes de testes e validação em [VALIDATION.md](VALIDATION.md).

## Estrutura

- `crates/unbloat` — motor de compactação, executável e janela.
- `crates/unbloat-shell` — DLL do menu de contexto do Explorer.
- `crates/xtask` — tarefas de build, pacote e licenças.

## Roadmap

- [ ] Suporte a Linux.

## Licença

[MIT](LICENSE). Licenças das dependências em [licenses/THIRD_PARTY.md](licenses/THIRD_PARTY.md).
