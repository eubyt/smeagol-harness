# Memórias por workspace e projeto

O armazenamento separa o nome legível de um projeto da sua identidade física.
O SQLite associa nomes e identidades de repositórios a UUIDs persistentes; os
nomes nunca são usados como diretórios da wiki. Isso permite projetos com o
mesmo nome em workspaces diferentes e renomeações sem mover as páginas.

## Definir o contexto

Crie `.smeagol-harness.toml` na raiz do projeto:

```toml
workspace = "trabalho"
project = "sistema-financeiro"
```

O marcador mais próximo com configurações de contexto prevalece. Marcadores
que contenham apenas `[capture]` não substituem o workspace do ancestral.
O marcador configura o contexto; regras de captura nessa seção ainda não são
implementadas pelo Smeagol Harness.

Sem marcador, o workspace é `default`. O projeto é derivado de um remoto Git
válido (`upstream`, depois `origin`), ou do nome da pasta quando não há remoto.
A identidade do remoto inclui o host, enquanto o nome legível usa o caminho do
repositório. Repositórios de hosts diferentes não são fundidos apenas porque
seus nomes legíveis coincidem: o segundo recebe um sufixo como `-2`.

Para agrupar subdiretórios de um repositório sem remoto, use:

```toml
workspace = "pessoal"
project_strategy = "repo-root"
```

Um `project` explícito no marcador agrupa intencionalmente os checkouts sob
esse nome dentro do workspace. Para manter a identidade quando o nome do
marcador mudar, declare uma identidade independente:

```toml
workspace = "trabalho"
project = "novo-nome"
identity = "sistema-financeiro-estavel"
```

Os nomes aceitam Unicode. Nomes vazios, espaços nas extremidades, controles,
separadores de caminho e mais de 128 caracteres são recusados. `_profile` é um
nome de projeto reservado ao perfil do workspace.

A busca por marcadores sobe até o diretório do usuário quando o projeto está
dentro dele. Fora desse diretório, para na raiz do checkout; sem checkout,
verifica somente o diretório atual. Um marcador inválido gera erro, sem trocar
silenciosamente o destino da memória.

## Comandos

```sh
smeagol-harness context
smeagol-harness --workspace trabalho --project sistema-financeiro context
smeagol-harness --workspace trabalho remember --scope project:sistema-financeiro --id banco --title "Banco de dados" --body "Usamos SQLite."
smeagol-harness --workspace trabalho search "SQLite" --scope project:sistema-financeiro
smeagol-harness --workspace trabalho --project sistema-financeiro search "retomar" --scope session:sessao-1
smeagol-harness --workspace trabalho rename-project --from sistema-financeiro --to financeiro
smeagol-harness reindex
```

As opções explícitas `--workspace` e `--project` prevalecem sobre a resolução do
marcador. Uma seleção explícita de projeto consulta o nome existente; não cria
uma nova identidade remota. O comando `context` resolve e registra o namespace
e mostra nomes, UUIDs e a origem da configuração.

## Organização dos dados

```text
<diretório-de-dados>/
├── wiki/
│   └── <workspace-uuid>/
│       ├── _meta.md
│       └── <project-uuid>/
│           ├── _meta.md
│           ├── memories/<id>.md
│           └── sessions/<session-id>.md
├── spool/sessions/<workspace-uuid>/<project-uuid>/<session-id>.jsonl
├── handoffs/<workspace-uuid>/<project-uuid>.md
└── db/workspaces.sqlite
```

Páginas distintas no mesmo projeto ficam em arquivos distintos. Os manifestos
`_meta.md` preservam os UUIDs, nomes e identidades fora do banco. `reindex`
reconstrói o índice a partir dos manifestos e páginas. Metadados inválidos
fazem a reconstrução falhar e sua transação é revertida.

As escritas do armazenamento são serializadas por um lock entre processos e
as páginas são publicadas por substituição de arquivo. Os eventos JSONL têm
um lock próprio por spool, compartilhado também pelos leitores. Isso não é
uma transação única entre arquivos e SQLite: após uma falha entre a publicação
da página e a atualização do índice, `reindex` restaura a busca.

O perfil é isolado por workspace. O handoff é isolado por workspace e projeto
e consumido uma vez; ele não transmite mensagens entre chats simultâneos.
A wiki não recebe commits Git automáticos nesta implementação.

## MCP e agentes no mesmo computador

As ferramentas MCP aceitam `workspace`, `project` e `cwd` opcionais. `cwd`
resolve o marcador e o remoto; opções explícitas de workspace/projeto têm
precedência. Sem essas opções, vale o contexto de inicialização do servidor.
`memory_context` mostra os nomes e UUIDs usados na operação.

Para clientes MCP que trabalham simultaneamente em vários repositórios,
informe `workspace` e `project` em cada chamada, ou informe o `cwd` correto.
O servidor não mantém uma variável global de “projeto ativo”.

Codex, Gemini e Antigravity compartilham as memórias quando seus hooks e o MCP
usam o mesmo diretório de dados e resolvem o mesmo namespace. Os hooks obtêm
o contexto do diretório informado pelo agente. Eventos e sessões persistidos
usam o UUID do projeto, não seu nome mutável.

## Dados anteriores

Esta estrutura usa um índice novo em `db/workspaces.sqlite`. Não há migração:
o banco antigo `db/memory.sqlite` e os diretórios antigos ficam intactos e não
são importados nem consultados. A versão nova começa a registrar dados na
estrutura com UUIDs. Execute `reindex` apenas para reconstruir essa estrutura
nova; ele não converte páginas do formato anterior.
