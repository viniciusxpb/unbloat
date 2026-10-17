# Instruções de code review — Unbloat

> `make review` copia este texto para a área de transferência seguido do
> `git diff --staged`. Revise o diff que vem logo abaixo.

## O projeto

Unbloat é um utilitário em Rust para Windows 10/11 x64 que compacta em `.7z`
(LZMA2) arquivos e pastas selecionados no Explorer. Ele respeita `.gitignore` e
`.ignore` aninhados, sem rede e sem exigir Git, Rust ou 7-Zip instalados.

- `crates/unbloat`: motor (scanner → manifesto → compressor), executável e janelas Win32.
- `crates/unbloat-shell`: DLL COM do menu de contexto, carregada **dentro do Explorer**.
- `crates/xtask`: tarefas de desenvolvimento portáveis (`cargo xtask`), chamadas pelo Makefile.
- `scripts/*.ps1`: instalador/desinstalador por usuário (HKCU, sem admin) e testes de Explorer/registro.
- `locales/pt-BR.txt` e `locales/en.txt`: todos os textos, embutidos no build.

## Como revisar

- Revise o diff à luz das invariantes abaixo. Se precisar de um arquivo inteiro
  para concluir algo, peça; não presuma o conteúdo.
- Prioridade: 1) perda ou vazamento de dados e segurança; 2) quebra de
  instalações existentes; 3) crash do Explorer; 4) correção em geral; 5) UX e
  mensagens; 6) simplificação.
- Não comente formatação nem lints: `make test` roda rustfmt e
  `clippy -D warnings`.
- Para cada achado: severidade (alta/média/baixa), `arquivo:linha`, o defeito em
  uma frase, um cenário concreto que o dispara (entrada → resultado errado) e a
  correção sugerida. Marque o que for suspeita não confirmada.
- Se não houver nada relevante, diga isso. Não invente achados para encher a lista.
- Responda em português.

## Invariantes que o diff não pode quebrar

**Motor** (`scanner`, `manifest`, `archiver`, `output`)
- O compressor só consome o manifesto. Nada fora das raízes selecionadas entra.
  Nomes internos são relativos, com `/`, sem `..` nem unidade, e sem duplicatas
  que difiram só em maiúsculas.
- Links, junctions e qualquer reparse point ficam fora da árvore e são recusados
  na seleção, inclusive nos ancestrais. Diretórios `.git` sempre ficam fora.
- Falha ou cancelamento nunca publica nada e remove o `.unbloat-*.unbloat-part`.
  A publicação acontece só depois de reler o arquivo e conferir o CRC, por rename
  atômico sem sobrescrever (`-2`, `-3`…). Fonte alterada durante a operação aborta.
- Arquivo de ignore ilegível aborta; nunca inclui em silêncio o que ele excluiria.
- Caminhos mostrados ao usuário passam por `display_path` (sem `\\?\`); o I/O usa
  o caminho canônico.

**DLL do Explorer** (`crates/unbloat-shell`)
- Roda dentro do `explorer.exe`: um panic derruba o Explorer (atenção a
  `unwrap`, `expect`, indexação e `RefCell`). Nada de trabalho pesado ou
  bloqueante, nada do motor: só lê a seleção, grava o arquivo de transporte e
  inicia `unbloat.exe`.
- Contratos COM: contagem de referências e `DllCanUnloadNow`, valor de retorno de
  `QueryContextMenu`, limites dos buffers de `GetCommandString`,
  `ReleaseStgMedium`. CLSIDs antigos continuam aceitos.

**Config**
- A reinstalação preserva o `config.toml` do usuário: campo novo precisa de
  padrão (`#[serde(default)]`), e campo removido precisa continuar sendo aceito.
  Campos desconhecidos são rejeitados de propósito.

**Instalador** (`scripts/*.ps1`)
- Windows PowerShell 5.1: sem `&&`, `??` ou ternário. Um `.ps1` com caractere não
  ASCII precisa estar em UTF-8 com BOM. Não use variáveis automáticas como nome
  (`$input`, `$args`, `$error`…).
- Nunca apagar fora da pasta de instalação. Recusar raiz de unidade e junctions.
  Tirar a barra final dos caminhos antes de comparar ou concatenar. Nunca pedir
  elevação, encerrar o Explorer ou tocar nos registros do 7-Zip.

**Localização**
- Chave nova vai para os dois catálogos, com os mesmos `{0}`, `{1}`. Os catálogos
  são UTF-8 com CRLF.
- Testes não podem depender do idioma do Windows: compare com `i18n::text(...)`,
  nunca com o texto literal.

**Portabilidade e ferramentas**
- O motor e o `xtask` devem compilar fora do Windows; código específico fica
  atrás de `cfg(windows)`. Tarefa de desenvolvimento nova vai para o `xtask`, não
  para script de shell.
- O Makefile é interface pública: targets existentes não são removidos nem
  renomeados.
- Dependência nova: licença na lista aceita (`crates/xtask/src/licenses.rs`),
  `Cargo.lock` atualizado, e nada que seja só de desenvolvimento como dependência
  do produto.

**Testes**
- Mudança de comportamento do motor vem com teste em
  `crates/unbloat/tests/engine.rs`.
- `scripts/test-shell.ps1` e `scripts/test-launcher.ps1` alteram o HKCU: não
  sugira rodá-los numa máquina com o Unbloat instalado.
