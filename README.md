# UNBLOAT

**Ship the code. Not the node_modules.**

Utilitário Windows 10/11 x64 em Rust. Compacta uma pasta ou várias pastas irmãs
em `.7z` com LZMA2, respeitando `.gitignore` e `.ignore` aninhados. Funciona sem
Git, Rust ou 7-Zip instalados no computador de destino. Não usa rede ou telemetria.

## Instalar e usar

Extraia `Unbloat-0.2.0-windows-x64.zip`, abra PowerShell **64 bits** na pasta
extraída e execute:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\install.ps1
```

A instalação por usuário fica em `%LOCALAPPDATA%\Programs\Unbloat`, sem admin.
Selecione uma ou várias pastas irmãs, clique com o botão direito e escolha
**Compactar com Unbloat**. No Windows 11, use **Mostrar mais opções**. A entrada
é direta, sem submenu. A janela nativa informa a atividade; **Cancelar** ou
fechar a janela solicita cancelamento e espera a limpeza segura. Ao concluir,
a janela mostra o destino e fecha automaticamente após dois segundos.

Uma pasta gera `pasta-unbloat.7z` ao lado da original. Várias pastas geram
`Unbloat-AAAAMMDD-HHMMSS.7z` no pai imediato comum. Colisões usam `-2`, `-3` etc.
Uma seleção de pastas com pais diferentes é recusada.

Para desinstalar:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\Programs\Unbloat\uninstall.ps1"
```

Se Explorer ou Unbloat mantiverem um binário em uso, o script informa os arquivos
bloqueados. Feche o aplicativo e saia/entre na sessão do Windows antes de tentar
novamente. Os scripts não encerram o Explorer nem exigem elevação. Reinstalações
preservam `config.toml`. A desinstalação preserva arquivos extras que você colocou
na pasta de instalação. Não modifica registros do 7-Zip.

## Desenvolver

Requisitos de build: Windows x64, Rust estável **1.93 ou superior**, Visual Studio
Build Tools (MSVC x64 e Windows SDK). `make` é opcional: os comandos Cargo e
PowerShell abaixo funcionam diretamente. O runtime C é ligado estaticamente.

```powershell
make build      # cargo build --workspace --release --locked
make test       # testes, rustfmt e clippy com warnings como erros
make run        # executa development.test_input do config.toml
make install    # build + instalação por usuário
make uninstall
make package    # gera dist/Unbloat e ZIP de distribuição
make benchmark # benchmark real, relatório em target/benchmark/RESULTS.md
```

```powershell
cargo build --workspace --release --locked
cargo test --workspace --all-targets --locked
cargo run -p unbloat -- --dev-run
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/collect-licenses.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package.ps1
```

O executável também aceita caminhos diretamente:

```powershell
.\dist\Unbloat\unbloat.exe 'C:\projetos\frontend' 'C:\projetos\backend'
```

Não existem opções de CLI para níveis, codecs ou exclusões. A configuração é
validada estritamente, rejeitando campos desconhecidos. O release lê o arquivo
ao lado do executável; o debug também aceita o arquivo da raiz do repositório.
`--dev-run` lê explicitamente o config da raiz e resolve `development.test_input`
em relação a ele. `[logging].enabled` controla mensagens de atividade em stderr;
não há log persistente com nomes dos arquivos.

## Regras e garantias

- Scanner usa `ignore::WalkBuilder`, incluindo sua precedência de `.ignore`
  sobre `.gitignore`. Não consulta o índice, regras globais, `.git/info/exclude`
  ou arquivos de ignore acima da seleção. Também funciona fora de repositórios.
- Diretórios ignorados são podados pelo walker. `!` não recupera um arquivo se
  o diretório pai permanece excluído. Arquivos ocultos são incluídos por padrão.
- Diretórios `.git` são sempre excluídos. Arquivos `.gitignore` continuam normais.
- Links, junctions e todos os reparse points (incluindo placeholders de nuvem)
  são excluídos. Seleções através desses pontos são recusadas. Materialize/copie
  arquivos de nuvem para uma pasta regular antes de compactar.
- O manifesto tem nomes determinísticos, relativos, com `/`, sem travessias ou
  caminhos de unidade. Conteúdos são lidos incrementalmente, nunca pelo scanner.
- O compressor consome exclusivamente o manifesto. LZMA2 nível 5, blocos sólidos
  de até 64 MiB/4096 entradas por padrão, com apenas um arquivo aberto por vez.
  Arquivos maiores que o bloco são transmitidos em um stream individual; não
  são carregados em RAM. O encoder mantém seu dicionário e buffers próprios.
- Diretórios vazios e arquivos vazios são preservados. ACLs, ADS, hard links,
  permissões e timestamps originais não são preservados nesta versão.
- Fontes alteradas/desaparecidas e falhas de leitura abortam a operação. Tamanho
  e data de modificação são conferidos antes/depois e antes da publicação.
  No Windows, a leitura nega compartilhamento de escrita/exclusão. Isso não é um
  snapshot de filesystem: feche ferramentas que continuam alterando o projeto.
- A saída temporária é exclusiva e fica no destino. O programa finaliza, sincroniza
  e relê todos os streams verificando CRC antes de publicar por rename atômico
  sem substituição. Erros/cancelamento removem o temporário quando possível.
  Encerramento forçado/queda de energia pode deixar `.unbloat-*.unbloat-part`;
  esses arquivos não são resultados válidos e podem ser removidos manualmente.
- Configuração aceita níveis 0–9 e blocos de 1–1024 MiB; v0.2 exige
  `exclude_git_directory = true`, `follow_symlinks = false`, formato `7z`, método
  `lzma2` e conflito `rename`.

## Arquitetura e validação

Dois pacotes: `unbloat` (motor + executável + janela Win32) e `unbloat-shell`
(DLL COM). A DLL só consulta os itens selecionados e lança o processo, transportando
caminhos UTF-16 em arquivo exclusivo com limites e validação. O executável remove
o transporte depois de ler. Não existe scanner/compressor dentro do Explorer.

`crates/unbloat/tests/engine.rs` contém o preflight real: dois projetos, regras
diferentes, scanner, manifesto, compactação sólida, extração e comparação de
conteúdo/hierarquia. Quando disponível, usa o 7-Zip original somente para teste
independente de integridade e extração. Há testes de precedência, nomes Unicode,
espaços, arquivos ocultos, junctions, poda, erros, cancelamento e concorrência.

`shell_probe` carrega a DLL real, cria um `IDataObject` nativo a partir de PIDLs,
verifica o menu direto e as referências COM e pode invocar o processo/janela:

```powershell
cargo build --workspace --release --examples --bins --lib --locked
.\target\release\examples\shell_probe.exe .\target\release\unbloat_shell.dll .\tests\fixtures\workspace\frontend .\tests\fixtures\workspace\backend
```

Use `scripts/test-shell.ps1` para os testes completos em diretório isolado:
seleções de uma, duas e três pastas, Unicode/espaços, cancelamento, instalação,
ativação COM pelo registro, reinstalação, arquivos em uso e desinstalação. Esse
teste registra temporariamente o handler real no HKCU e remove tudo no `finally`;
nunca o execute sobre uma instalação de produção existente.

A renderização/posição visual no menu do Explorer precisa também ser conferida
manualmente em Windows 10 e no menu clássico do Windows 11. O probe testa o
contrato real de menu, mas não constitui inspeção visual do Explorer. Consulte
[VALIDATION.md](VALIDATION.md) para resultados e limitações do ambiente testado.

APIs verificadas nas fontes de `sevenz-rust2` **0.23.0**, `ignore` **0.4.33** e
`windows` **0.62.2**. Referências primárias:
[ArchiveWriter](https://docs.rs/sevenz-rust2/0.23.0/sevenz_rust2/struct.ArchiveWriter.html),
[WalkBuilder](https://docs.rs/ignore/0.4.33/ignore/struct.WalkBuilder.html),
[IContextMenu](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-icontextmenu).

## Licenças

Código/ícone próprios: [MIT](LICENSE). Dependências e textos originais:
[licenses/THIRD_PARTY.md](licenses/THIRD_PARTY.md). `Cargo.lock` fixa as versões.
`collect-licenses.ps1` confere as expressões e copia licenças/avisos de todas as
crates bloqueadas, incluindo dependências específicas de outras plataformas.
Execute novamente ao atualizar dependências e revise antes de publicar.

`licenses/DEPENDENCIES.md` e `licenses/source-notices/` são gerados e ficam fora
do Git. O empacotamento e a instalação a partir do repositório regeneram esses
arquivos automaticamente; eles continuam incluídos na distribuição. `target/`,
`dist/`, binários, arquivos temporários e configurações locais também são ignorados.
Mantenha `Cargo.lock`, `.cargo/config.toml`, os assets e as fixtures no repositório.
