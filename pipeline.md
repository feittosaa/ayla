# Ayla — Pipeline

Atualizada em **19/11/2026** · Branch: `codex/ayla-fundacao`, criada da `development` (`306ec9c`).

## 1. Ideia e decisões confirmadas

Ayla é uma plataforma pessoal offline-first de dados, serviços e automações. A IA é opcional e utiliza os serviços da aplicação. A visão pública continua no README; esta pipeline acompanha execução e decisões, e o `AGENTS.md` orienta os agentes.

- **Decisões de projeto são tomadas com o usuário:** explicitar opções e impactos antes de alterar arquitetura, escopo, produto, infraestrutura ou política de dados.
- **Pouco atrito técnico para o usuário final:** tokens, processos e caminhos internos não devem virar tarefas manuais de uso do produto.
- **Desempenho é requisito de arquitetura. Rust é uma das linguagens principais.** Decidir a divisão Rust/Python antes de expandir o núcleo; medir latência e consumo de recursos.
- **Linux primeiro; Windows depois.** P0 cobre desenvolvimento desktop Linux; instaladores ficam para outra etapa. Acesso web independente não é alvo deste P0.
- **Local por padrão; cloud opcional e habilitada explicitamente.** Sem credencial ou habilitação, não criar cliente cloud. Sem modelo, a aplicação continua abrindo.
- **Nenhum modelo local fixo ou obrigatório.** A direção é selecionar modelos conforme o pedido, com tipos opcionais escolhidos na instalação e perfis leve, moderado e máximo para diferentes hardwares.
- **Escopo acordado:** fechar a base P0 agora; desenhar configuração de modelos na próxima etapa. Catálogo, tipos, limites de hardware e regras de seleção ainda não foram decididos.
- UI, IA e automações devem consumir serviços determinísticos compartilhados. Projetos separados, como a Moneta, entram por contratos; não duplicar seu domínio nem presumir banco compartilhado.
- Preservar dados locais, definir sua fonte de verdade e adicionar infraestrutura conforme a entrega exigir.

## 2. Estado atual

| Área | Implementação atual |
|---|---|
| Desktop Linux | Tauri/Rust inicia backend em loopback com porta dinâmica, gera token do sistema operacional, entrega conexão por IPC e encerra o processo filho ao sair. |
| Backend | FastAPI; `/health` autenticado e independente de IA; `/chat` com payload validado e eventos SSE JSON de token, conclusão e erro. |
| Interface | React/Vite; conexão automática pelo Tauri, cliente HTTP único, tratamento de falhas e botão de interrupção que preserva texto parcial. |
| IA | Clientes carregados sob demanda; local primeiro; cloud opcional como fallback antes de qualquer token. Não misturar respostas de provedores após falha parcial. Modelo local sem valor obrigatório. |
| Privacidade | Cloud recebe somente a mensagem atual, quando habilitada e permitida pela heurística. A heurística restringe envio, mas não garante detecção de todos os dados sensíveis; política definitiva fica para o desenho da IA. |
| Memória | Janela de 10 mensagens em RAM e SQLite legado preservado. Resposta completa persistida; entrada atual aparece uma vez no prompt. Caminho estável, conexões por operação e abertura somente no uso. |
| Dados | Banco e backups ignorados pelo Git; banco retirado do índice sem apagar a cópia local. Backup consistente criado em `backend/backups/`. |
| Órgãos/infra | Sem Moneta ou outros órgãos integrados; sem RAG, migrations, Compose, filas ou instalação de modelos. `docs/vision.md` continua vazio. |

O P0 corrigiu os defeitos de persistência, autenticação, imports, inicialização e streaming encontrados na revisão inicial. A memória ainda usa resumos simples e compressão aleatória; isolamento por conversa, limites e deduplicação continuam pendentes.

## 3. Dependências e projetos separados

| Projeto/serviço | Papel e próximo requisito |
|---|---|
| **Moneta** | Projeto financeiro desenvolvido separadamente. Não foi localizado repositório, responsável, versão ou contrato nas fontes consultadas. Identificar esses dados antes da integração; começar por consulta de contas/transações sem IA. |
| **Pluggy** | Sincronização bancária prevista na Moneta. Credenciais, acesso, atualização e falhas pertencem ao projeto responsável; não implementar sincronização duplicada na Ayla. Só bloqueia a validação com dados bancários reais. |
| **Securo** | Referência financeira citada no README; não é dependência de execução. Verificar licença antes de reutilizar código. |
| **Ollama / modelos locais** | Adaptador existente e configurável para desenvolvimento. Instalação, catálogo, perfis de hardware e seleção por pedido ainda serão desenhados; nenhum modelo é carregado no início da aplicação. |
| **Cloud** | Adaptador opcional existente. Exige habilitação e chave no ambiente de desenvolvimento; futura experiência de configuração e política de envio dependem de decisão conjunta. |
| **VoiceAssistant** | Referência de voz/desempenho citada no README, sem integração. Consultar quando o caso de uso de voz entrar em execução. |
| **Athena e demais órgãos** | Backlog conceitual sem contratos verificados. Escolher integração por utilidade real, após a primeira entrega. |
| **PostgreSQL, Qdrant, Redis, Docker** | Propostas, sem infraestrutura implementada. Adotar apenas quando uma entrega exigir. SQLite sustenta a base local atual. |

A Moneta pode evoluir independentemente. O contrato mínimo, a disponibilidade do serviço e a propriedade dos dados precisam estar claros antes de conectar a Ayla.

## 4. Execução

### P0 — Base desktop Linux confiável · concluído

- [x] Corrigir persistência da resposta e duplicação da mensagem atual no prompt.
- [x] Unificar autenticação; recusar token ausente/não configurado; entregar e usar token internamente no desktop.
- [x] Centralizar configuração, tornar IA opcional e iniciar sem modelo, chave ou rede externa.
- [x] Corrigir imports e inicialização pelo Tauri; validar desenvolvimento Linux. Empacotamento Windows removido do fluxo P0; instaladores adiados por decisão do usuário.
- [x] Unificar cliente HTTP, validar status e SSE, distinguir conclusão/erro, preservar resposta parcial e permitir interrupção.
- [x] Estabilizar caminho SQLite; preservar banco, criar backup e retirar dados pessoais do índice Git.

**Evidências de aceite:**

- 21 testes Python: memória, backup/restauração em banco temporário, autenticação, payload, SSE, roteamento e adaptadores simulados.
- 5 testes do cliente: chunks fragmentados, Unicode, espaços/linhas, erro parcial, EOF incompleto, HTTP e cancelamento.
- TypeScript em modo estrito, build Vite e lint JavaScript passaram. O lint atual cobre JS/JSX; os arquivos TS/TSX foram verificados pelo compilador TypeScript.
- `cargo check --locked` passou. Teste Rust iniciou backend real sem IA, verificou `/health` autorizado e 401 sem token, e confirmou encerramento. Amostra única de prontidão: **207 ms**, em desenvolvimento; não é benchmark de inferência.
- Tauri abriu no Linux com provedores desligados e banco temporário; a UI consultou `/health` com sucesso. Usuário confirmou a mensagem de prontidão e fechou a janela; backend encerrou junto.
- README preservado nesta rodada e alteração preexistente no `frontend/package-lock.json` mantida. Integração com `development` depende de autorização separada.

**Limites:** inferência real local/cloud, cancelamento durante geração real e instaladores não foram homologados. Seleção dinâmica, perfis e configuração de instalação não fazem parte do P0 concluído.

### P1 — Configuração de modelos e núcleo independente do chat · próxima etapa

- [ ] **Decidir juntos:** responsabilidades de Rust e Python no núcleo, considerando desempenho, dependências e manutenção. Não iniciar uma reescrita sem essa definição.
- [ ] **Decidir juntos:** tipos de modelos opcionais, catálogo inicial e critérios dos perfis leve, moderado e máximo, conforme hardware disponível.
- [ ] **Decidir juntos:** como selecionar modelo por pedido, comportamento sem um tipo instalado e habilitação/limites de cloud.
- [ ] Desenhar configuração simples na instalação/primeiro uso; evitar terminal, tokens manuais e nomes técnicos como exigência para o usuário final.
- [ ] Definir instalação, remoção e carregamento sob demanda dos modelos, com orçamento de RAM/VRAM, cancelamento e liberação de recursos.
- [ ] Criar tela inicial de serviços/configuração acessível sem conversa e sem IA.
- [ ] Definir serviços mínimos, persistência por domínio, migrations, isolamento e limites da memória.
- [ ] Medir inicialização, primeiro token, vazão, cancelamento e consumo antes de escolher runtimes/infraestrutura novos.

**Depende de:** P0 e decisões acima. **Aceite a detalhar com o desenho:** usuário configura apenas capacidades desejadas; aplicação continua útil sem modelos; recursos selecionados respeitam hardware e orçamento definidos.

### P2 — Primeira integração com a Moneta

- [ ] Identificar repositório, responsável, versão e capacidades realmente disponíveis.
- [ ] Acordar transporte, autenticação, IDs, moeda, datas, paginação, erros e fonte de verdade.
- [ ] Implementar adaptador com fixture do contrato para desenvolver sem acesso bancário real.
- [ ] Entregar consultas de contas/transações na UI, com estados vazio e indisponível.
- [ ] Validar com Moneta real e, quando disponível, sua sincronização Pluggy.

**Depende de:** serviços mínimos do P1 e contrato da Moneta. **Aceite:** consultar finanças sem LLM, sem duplicar o domínio e sem derrubar a Ayla se a Moneta estiver offline.

### P3 — IA sobre serviços reais

- [ ] Expor inicialmente ferramentas de leitura da integração validada.
- [ ] Definir permissões e política de envio antes de disponibilizar dados dos órgãos à cloud.
- [ ] Responder a partir de consultas verificáveis, identificando período e ausência de dados.
- [ ] Evoluir memória/RAG quando houver necessidade comprovada.

**Depende de:** P2. **Aceite:** UI e IA consomem o mesmo serviço; respostas usam registros reais, sem inventá-los.

### Depois

Athena e outros órgãos → Biblioteca e retrospectivas. Mercúrio permanece como proposta de comparação de preços. Voz e multimodalidade dependem dos tipos opcionais acordados e de casos de uso. Instalador Linux e, posteriormente, port Windows terão escopo próprio.

## 5. Comandos de desenvolvimento e verificação

- Desktop Linux: na pasta `frontend`, `npm run tauri -- dev`. Requer Rust/Cargo, WebKitGTK e `.venv` do projeto com `backend/requirements.txt`; nenhuma IA precisa estar instalada. `AYLA_PYTHON` permite apontar outro interpretador.
- Backend: da raiz, `PYTHONPATH=backend .venv/bin/python -m unittest discover -s backend/tests -v`.
- Cliente/build: em `frontend`, `npm test`, `npm run build` e `npm run lint`.
- Tipos: em `frontend`, `tsc --noEmit --strict --jsx react-jsx --module esnext --moduleResolution bundler --target es2022 --lib es2022,dom --allowImportingTsExtensions --skipLibCheck src/App.tsx src/components/Chat.tsx src/services/Api.ts` (compilador disponível neste ambiente).
- Desktop: em `frontend/src-tauri`, `cargo check --locked`; teste de processo com banco descartável inexistente: `AYLA_DB_PATH=/tmp/ayla-lifecycle-test.db AYLA_LOCAL_ENABLED=false AYLA_CLOUD_ENABLED=false cargo test --locked --lib`.
- Backup: `python backend/backup.py`. Restauração: fechar a Ayla, preservar o banco atual e copiar o backup para o caminho do banco. `AYLA_DB_PATH` substitui o padrão estável `backend/ayla_memory.db`.

As variáveis documentadas em `backend/.env.example` são recursos de desenvolvimento, não a experiência de instalação do produto. O arquivo não é carregado automaticamente. Neste ambiente, o Cargo foi validado com cache temporário em `/tmp/ayla-p0-cargo`; isso não é requisito do projeto.
