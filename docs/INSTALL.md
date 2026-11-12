# Instalação do Smeagol Harness

O Smeagol Harness requer Rust 1.95 ou superior para compilar do código. Baixe o binário pronto na página de Releases ou instale a partir do checkout:

```sh
cargo install --path crates/adrian-harness-cli
smeagol-harness init
smeagol-harness setup --agent all --plan
smeagol-harness setup --agent all --apply
```

Para conectar o MCP stdio ao Codex, registre o binário com:

```sh
codex mcp add smeagol-harness -- /caminho/absoluto/para/smeagol-harness serve --transport stdio
codex mcp list
```

O comando grava a conexão MCP no `config.toml` do Codex. Também é possível
adicionar a entrada diretamente ao `$CODEX_HOME/config.toml` (ou
`~/.codex/config.toml`):

```toml
[mcp_servers.smeagol-harness]
command = "/caminho/absoluto/para/smeagol-harness"
args = ["serve", "--transport", "stdio"]
```

Com o binário do checkout, use o caminho mostrado por `cargo build --release`:
`target/release/smeagol-harness`.
O Codex lança esse processo sob demanda e usa stdin/stdout para MCP; não há
daemon separado nesse modo.

Para executar o servidor como processo independente, abra um terminal e rode:

```sh
smeagol-harness serve --transport http --bind 127.0.0.1:49375
```

O processo permanece em primeiro plano e atende em
`http://127.0.0.1:49375/mcp`. O servidor recusa binds fora de loopback. Encerre
com `Ctrl+C`; para iniciar junto do sistema, configure o comando como serviço
do usuário no `launchd` ou systemd.

Com o servidor HTTP em execução, registre-o em outra configuração MCP usando
`codex mcp add smeagol-harness-http --url http://127.0.0.1:49375/mcp`.

`setup --plan` é somente leitura. `setup --apply` mescla os hooks do Smeagol Harness
em `$CODEX_HOME/hooks.json` (ou `~/.codex/hooks.json`) e preserva outros handlers.
Depois, abra o Codex, execute `/hooks`, revise e confie nos hooks do Smeagol Harness,
e reinicie a sessão. O Codex exige essa revisão antes de executar hooks não
gerenciados.

Os hooks guardam prompts sanitizados e metadados de ferramentas no spool local.
`SessionEnd` cria a página de sessão e um handoff; `SessionStart` entrega o handoff
uma vez como contexto histórico. Os dados ficam em
`$SMEAGOL_HARNESS_HOME` (com suporte legado a `$ADRIAN_HARNESS_HOME`),
`$XDG_DATA_HOME/smeagol-harness` ou `~/.local/share/smeagol-harness`, nessa ordem.

O seletor `--agent` aceita `codex`, `gemini-cli`, `antigravity` ou `all`.
Os arquivos globais usados são `hooks.json` do Codex,
`~/.gemini/settings.json` do Gemini CLI e `~/.gemini/config/hooks.json` do
Antigravity CLI. Reinicie cada agente depois da instalação; no Gemini CLI
confira `/hooks panel` e no Antigravity CLI `/hooks`.

O Gemini captura início/fim de sessão, prompts, ferramentas e compactação. O
Antigravity captura início de sessão, invocações do modelo e ferramentas; o CLI
não oferece um evento `SessionEnd`, então o hook `Stop` registra o fim de cada
execução, mas não fecha automaticamente a página/handoff da sessão.

No Windows, o armazenamento usa `%LOCALAPPDATA%\SmeagolHarness` por padrão,
e recorre a `%USERPROFILE%` quando `LOCALAPPDATA` não está definido.

Para remover a integração nesta versão, remova somente os handlers cujo
`statusMessage` começa com `Smeagol Harness managed:` do arquivo `hooks.json`. O
comando automatizado de desinstalação ainda será implementado.
