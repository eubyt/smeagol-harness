# Especificação inicial — Smeagol Harness

**Estado:** rascunho em implementação  
**Implementação:** Rust, aplicação local de usuário único

O armazenamento atual usa UUIDs por workspace/projeto, manifestos e índice
reconstruível. O contrato implementado está em [`WORKSPACES.md`](WORKSPACES.md).
Não há migração dos arquivos do formato anterior.

## 1. Objetivo

Manter memória útil entre sessões e projetos de agentes de programação, com
recuperação de contexto limitada por orçamento de tokens. O sistema deve reduzir
reexplicações e facilitar a retomada de trabalho sem treinar modelos nem exigir
serviços externos.

## 2. Princípios

- Local-first: os dados ficam na máquina do usuário no MVP.
- Markdown legível é a fonte de verdade; índices podem ser reconstruídos.
- Nenhuma chamada a LLM é necessária para capturar, consultar ou iniciar.
- O contexto inicial é pequeno, limitado e separado por escopo.
- Conteúdo recuperado é dado histórico não confiável, nunca instrução executável.
- Instalação e remoção são planejáveis, idempotentes e restritas aos arquivos gerenciados.
- Não sobrescrever configurações do agente ou conteúdo do usuário.
- Captura deve ser limitada, sanitizada e configurável por caminho/evento.

## 3. Escopo proposto do MVP

### Incluído

1. CLI Rust com `setup --plan`, `setup --apply`, `status`, `search` e `forget`.
2. Integração inicial com Codex; outros agentes ficam para etapas posteriores.
3. Armazenamento local em Markdown para perfil, projeto, sessões e handoffs.
4. SQLite como índice derivado para busca textual; reindexação a partir do Markdown.
5. Hooks de ciclo de vida para capturar eventos permitidos sem bloquear o agente.
6. Briefing de início de sessão com limite de tokens configurável.
7. Resumo determinístico de sessão sem LLM; promoção automática de regras explicitamente duráveis.
8. Servidor MCP local para consulta e gravação via stdio ou HTTP limitado a loopback.
9. Testes isolados que nunca alteram o `HOME` real nem dependem de rede.

### Fora do MVP

- Servidor remoto, multiusuário ou sincronização entre máquinas.
- Busca vetorial, embeddings, painel web ou chamada obrigatória a LLM.
- Aprendizagem por modelo/LLM sem revisão humana.
- Integração simultânea com múltiplos harnesses.

## 4. Ciclo funcional

1. `setup --plan` detecta Codex e descreve cada arquivo e ação proposta sem escrever.
2. `setup --apply` aplica somente as mudanças selecionadas e registra a propriedade dos arquivos gerenciados.
3. No início de sessão, o harness identifica o projeto, recupera um briefing conciso e o injeta pelo mecanismo suportado pelo agente.
4. Durante a sessão, hooks capturam somente eventos habilitados, após sanitização e aplicação das regras de exclusão.
5. No encerramento, o harness salva observações, resumo e estado de retomada. Uma interrupção abrupta deve permitir reconstrução a partir dos eventos já persistidos.
6. Uma sessão futura recupera o handoff e busca outras memórias sob demanda, sempre respeitando o escopo do projeto.
7. No encerramento, somente regras explicitamente permanentes no prompt são promovidas como memórias do projeto; solicitações comuns não são promovidas.

## 5. Escopos e dados

- **Perfil:** preferências estáveis do usuário reutilizáveis entre projetos.
- **Projeto:** decisões, convenções, arquitetura e problemas recorrentes daquele repositório.
- **Sessão:** eventos, resumo, tentativas e perguntas ainda abertas.
- **Handoff:** estado mínimo necessário para continuar uma tarefa interrompida.

Identificadores de projeto devem derivar do repositório Git quando houver uma origem
válida; o nome do diretório é fallback. Uma busca por projeto não pode retornar dados
de outro projeto por causa de um “último projeto ativo” global.

## 6. Segurança e privacidade

- Sanitizar segredos antes de persistir observações.
- Permitir negar captura por caminho e por tipo de evento.
- Não capturar conteúdo de arquivos por padrão; registrar metadados de ferramenta e trechos limitados quando habilitado.
- Não executar comandos encontrados em memórias, resumos ou handoffs.
- `forget` deve explicar o que será removido e distinguir remoção da fonte de limpeza de índices/backups.
- Não colocar tokens ou credenciais nas configurações dos agentes ou na linha de comando.

## 7. Instalação

- Planejamento é somente leitura e tem saída humana e JSON estável.
- Aplicação exige confirmação interativa ou `--yes` explícito.
- Reexecução não duplica hooks nem reescreve arquivos sem necessidade.
- Alterações devem preservar chaves, comentários e conteúdo não gerenciado sempre que possível.
- Em arquivo com formato desconhecido, marcadores duplicados ou conteúdo gerenciado editado manualmente, falhar sem escrever e explicar o conflito.
- `uninstall --plan` deve identificar apenas recursos pertencentes ao Smeagol Harness.

## 8. Estratégia de testes

### Unitários

Cobrir identificação de projeto, sanitização, orçamento de tokens, parsing de eventos,
renderização do briefing e regras de escopo com funções puras e fixtures pequenas.

### Integração

Usar diretórios temporários para verificar índice SQLite, persistência Markdown,
reconstrução do índice, idempotência da captura e recuperação entre sessões.

### CLI e instalação

Executar o binário real com `HOME` e diretórios XDG temporários. Testar que `--plan`
não escreve, `--apply` produz a configuração esperada, a segunda aplicação não gera
diff e a remoção preserva arquivos/chaves alheios. Simular o agente e o serviço quando
necessário; não chamar serviços reais.

### Fluxo ponta a ponta

Simular eventos de início, prompt, ferramenta e fim; verificar que a memória persiste,
o próximo briefing recupera o fato relevante e o tamanho fica dentro do orçamento.
Incluir casos de interrupção, dados malformados, duplicação/retry e tentativa de
cruzar escopos.

### Gates de desenvolvimento propostos

`cargo fmt --all -- --check`, `git diff --check`, `cargo clippy --all-targets -- -D warnings`
e `cargo test --all-targets`. Avaliar `cargo-deny` quando as dependências do MVP
estiverem definidas. Testes devem permanecer rápidos e determinísticos; qualquer teste
lento deve ser identificado e separado, sem desaparecer da saída da suíte.

## 9. Critérios de aceitação do MVP

- Planejar setup não altera nenhum arquivo do usuário.
- Aplicar setup duas vezes produz o mesmo estado final sem duplicatas.
- Remover a integração não apaga conteúdo de terceiros.
- Eventos capturados são sanitizados e podem ser excluídos por regra.
- Memórias de projeto não aparecem na busca de outro projeto.
- O briefing respeita o orçamento configurado e inclui proveniência suficiente.
- O ciclo de sessão pode ser recuperado após encerramento abrupto.
- Busca e recuperação funcionam sem chave de API ou conexão de rede.

## 10. Decisões em aberto

1. O perfil pessoal entre projetos entra no primeiro MVP ou fica para depois?
2. Quais eventos do Codex devem ser capturados por padrão e quais exigem opt-in?
3. Como estimar tokens sem depender de tokenizer de um provedor específico?
4. O briefing será entregue por hook, MCP local, arquivo de instruções ou combinação?
5. A memória durável será promovida automaticamente por regras ou sempre ficará pendente de revisão no MVP?

## 11. Arquitetura inicial em crates

- `smeagol-harness-core`: tipos de memória, escopos e validação sem I/O.
- `smeagol-harness-store`: páginas Markdown, índice SQLite e resolução do diretório de dados.
- `smeagol-harness-setup`: detecção do Codex e plano somente leitura; aplicação será habilitada após fechar o contrato do hook.
- `smeagol-harness-hooks`: eventos, sanitização inicial e spool local JSONL.
- `smeagol-harness-consolidate`: compilação determinística das observações em página por sessão.
- `smeagol-harness-cli`: interface de linha de comando e composição dos demais crates.

O primeiro fluxo automático usa hooks de usuário do Codex. `UserPromptSubmit` captura
texto sanitizado; `PostToolUse` guarda apenas o nome da ferramenta; `SessionEnd` grava
um resumo determinístico e publica um handoff local; `SessionStart` entrega esse handoff
uma vez como contexto histórico não confiável. A integração não é ativada até `setup
--apply`, e o Codex exige revisão/confiança dos hooks.

O servidor MCP expõe `memory_status`, `memory_search`, `memory_remember` e
`memory_brief`. O transporte stdio é iniciado pelo cliente MCP; o transporte HTTP
é um processo em primeiro plano que aceita somente binds loopback e publica `/mcp`.
Ambos usam o mesmo armazenamento local e não chamam um LLM.

### Comandos já iniciados

- `init`: cria a árvore Markdown e o índice FTS5 local.
- `remember`: grava uma página de memória com escopo explícito.
- `search`: exige escopo explícito para impedir recuperação acidental entre projetos.
- `capture`: recebe uma observação JSON por `stdin`, sanitiza o trecho e grava JSONL local.
- `setup --plan`: detecta o diretório Codex e descreve ações sem escrever arquivos.

No encerramento, o consolidador também promove regras explícitas como “sempre”, “prefiro”,
“decidimos” e “lembre-se”. A promoção tem escopo de projeto e usa um identificador
estável para atualizar regras repetidas. Não há promoção de conclusões inferidas de
ferramentas ou de pedidos pontuais.

A estimativa do `brief` é independente de provedor e aproximada; ela limita o texto
produzido, mas não equivale à contagem exata do tokenizer do agente. O briefing busca
no escopo indicado pelo usuário e inclui a fonte de cada trecho. A consulta é dividida
em palavras, aceita pontuação e acentos, encontra prefixos e prioriza correspondências
no título com BM25. Não há stemming semântico nem embeddings.

A captura é local, limitada e separada por arquivo de sessão. Os hooks de captura são
síncronos e fazem apenas trabalho local, para não perder eventos de fundo ao encerrar
o Codex. Não são gravados argumentos nem saídas das ferramentas.
A sanitização atual cobre credenciais comuns, mas ainda precisa de avaliação adicional
antes de ativar captura em ambientes com dados regulados ou muito sensíveis.
