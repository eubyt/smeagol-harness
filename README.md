# Smeagol Harness

Harness local em Rust para registrar conhecimento útil de sessões de agentes,
recuperar contexto entre tarefas e reduzir tokens gastos reexplicando o projeto.

O projeto usa um workspace com crates separados por responsabilidade. A
especificação está em [`docs/SPEC.md`](docs/SPEC.md), e a instalação em
[`docs/INSTALL.md`](docs/INSTALL.md).
`setup --apply` adiciona hooks ao `hooks.json` do Codex; depois, abra o Codex,
execute `/hooks` e revise/confie nos hooks instalados.

Os padrões de código, comentários e validação estão em [`AGENTS.md`](AGENTS.md).
Memórias são isoladas por workspace e projeto com UUIDs persistentes. Veja
[`docs/WORKSPACES.md`](docs/WORKSPACES.md) para configurar o marcador,
consultar o contexto e reconstruir o índice. O formato novo não migra dados antigos.

## Instalação

Baixe o binário da sua plataforma na página de Releases. Também é possível
compilar com Rust 1.95 ou superior usando `cargo install --path crates/smeagol-harness-cli`.

## Crates

- `smeagol-harness-core`: tipos e validação de domínio.
- `smeagol-harness-store`: páginas Markdown e índice SQLite reconstruível.
- `smeagol-harness-setup`: detecção e planejamento de instalação.
- `smeagol-harness-hooks`: formato de eventos, sanitização e spool JSONL.
- `smeagol-harness-consolidate`: consolidação determinística de observações por sessão.
- `smeagol-harness-cli`: comandos que orquestram as crates.

## Comandos iniciais

```sh
cargo run -p smeagol-harness-cli -- setup --plan
cargo run -p smeagol-harness-cli -- setup --apply
cargo run -p smeagol-harness-cli -- setup --agent gemini-cli --apply
cargo run -p smeagol-harness-cli -- setup --agent antigravity --apply
cargo run -p smeagol-harness-cli -- setup --agent all --apply
cargo run -p smeagol-harness-cli -- init
cargo run -p smeagol-harness-cli -- status
cargo run -p smeagol-harness-cli -- remember --scope project:meu-projeto --id decisao-cache --title "Decisão de cache" --body "Usar cache local."
cargo run -p smeagol-harness-cli -- search "cache" --scope project:meu-projeto
cargo run -p smeagol-harness-cli -- capture < evento.json
cargo run -p smeagol-harness-cli -- consolidate --project meu-projeto --session sessao-1
cargo run -p smeagol-harness-cli -- brief "cache" --scope project:meu-projeto --budget 300
cargo run -p smeagol-harness-cli -- serve --transport stdio
cargo run -p smeagol-harness-cli -- serve --transport http --bind 127.0.0.1:49375
```

O servidor MCP oferece `memory_status`, `memory_search`, `memory_remember` e
`memory_brief`. Para o Codex, registre o comando
`smeagol-harness serve --transport stdio` como servidor stdio; o Codex inicia e encerra o processo
junto com a sessão. Para iniciar um servidor independente, rode
`smeagol-harness serve --transport http`; ele atende somente em loopback no
endereço configurado e publica o endpoint `/mcp`.

`setup --agent gemini-cli --apply` instala hooks globais do Gemini CLI;
`setup --agent antigravity --apply` instala os hooks do Antigravity CLI. Use
`setup --agent all --apply` para instalar Codex, Gemini CLI e Antigravity.
