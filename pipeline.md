# Ayla — Pipeline

Atualizada em **19/11/2026** · Branch: `codex/ayla-fundacao`, criada da `development` (`306ec9c`).

## 1. Ideia e decisões confirmadas

Ayla é uma plataforma pessoal offline-first de dados, serviços e automações. A IA é opcional e utiliza os serviços da aplicação. A visão pública continua no README; esta pipeline acompanha execução e decisões, e o `AGENTS.md` orienta os agentes.

- **Decisões de projeto são tomadas com o usuário:** explicitar opções e impactos antes de alterar arquitetura, escopo, produto, infraestrutura ou política de dados.
- **Pouco atrito técnico para o usuário final:** tokens, processos e caminhos internos não devem virar tarefas manuais de uso do produto.
- **Desempenho é requisito de arquitetura. Rust é uma das linguagens principais.** Núcleo em Rust, Python apenas como apoio quando necessário; medir latência e consumo de recursos.
- **Linux primeiro; Windows depois.** P0 cobre desenvolvimento desktop Linux; instaladores ficam para outra etapa. Acesso web independente não é alvo deste P0.
- **Local por padrão; cloud opcional e habilitada explicitamente.** Sem credencial ou habilitação, não criar cliente cloud. Sem modelo, a aplicação continua abrindo.
- **Nenhum modelo local fixo ou obrigatório.** A direção é selecionar modelos conforme o pedido, com tipos opcionais escolhidos na instalação e perfis leve, moderado e máximo para diferentes hardwares.
- **P1 acordado:** texto, raciocínio, visão, transcrição, síntese e memória semântica; importação livre, runtimes gerenciados e perfis ajustáveis de RAM em 25/50/75%. Órgãos continuam P2+.
- UI, IA e automações devem consumir serviços determinísticos compartilhados. Projetos separados, como a Moneta, entram por contratos; não duplicar seu domínio nem presumir banco compartilhado.
- Preservar dados locais, definir sua fonte de verdade e adicionar infraestrutura conforme a entrega exigir.

## 2. Estado atual

| Área | Implementação atual |
|---|---|
| Desktop Linux | Tauri com núcleo Rust; tela inicial e configuração abrem sem Python, modelo ou credencial. Backend Python legado iniciado apenas se solicitado por IPC. |
| Núcleo | Configuração JSON versionada, hardware, política de envio, SQLite com migrations, tarefas e ciclo de vida dos modelos em Rust. |
| Interface | Conversa como entrada e Configurações, com modelos/recursos em opções avançadas; importação local por seletor, instalação explícita dos runtimes, progresso, cancelamento e descarregamento. |
| IA local | Seleção automática por tipo de pedido e orçamento; seleção manual disponível. Um worker ativo, carregado sob demanda e liberado por inatividade. |
| Cloud | Endpoint compatível com OpenAI configurável; chave por sessão ou cofre do sistema. Envia pedido atual e personalidade explicitamente revisados e declarados públicos; bloqueia conteúdo sensível/duvidoso e padrões adicionais. A heurística não garante detectar todo dado sensível. |
| Memória/dados | Banco nativo separado, conversas isoladas, limites de contexto, respostas incompletas identificadas, backup SQLite e importação explícita do legado preservando origem. Vetores separados por modelo. |
| Órgãos | Moneta e demais órgãos ainda sem integração; serviços independentes de IA continuam como direção P2. |

## 3. Dependências e projetos separados

| Projeto/serviço | Papel e próximo requisito |
|---|---|
| **Moneta** | Projeto financeiro desenvolvido separadamente. Não foi localizado repositório, responsável, versão ou contrato nas fontes consultadas. Identificar esses dados antes da integração; começar por consulta de contas/transações sem IA. |
| **Pluggy** | Sincronização bancária prevista na Moneta. Credenciais, acesso, atualização e falhas pertencem ao projeto responsável; não implementar sincronização duplicada na Ayla. Só bloqueia a validação com dados bancários reais. |
| **llama.cpp** | Runtime gerenciado para texto, raciocínio, visão e embeddings; pacotes Linux x86_64 CPU/Vulkan versionados, verificados por SHA-256. Ollama permanece somente no adaptador legado. |
| **Cloud** | Opcional e configurável no desktop. Modos local, cloud, híbrido, por conversa, pedido explícito ou sugestão; veto de privacidade em todos. Híbrido escolhe destino por tarefa, sem fallback silencioso. |
| **whisper.cpp** | Transcrição local de WAV e microfone, com pacote gerenciado e hash. A Ayla responde em texto: a síntese (sherpa-onnx) saiu em 20/11/2026. Microfone e modelos são opcionais. |
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

### P1 — Configuração de modelos e núcleo independente do chat · concluído

**Decisões aprovadas:** núcleo Rust; Python auxiliar; llama.cpp, whisper.cpp e sherpa-onnx gerenciados; todas as capacidades de IA nesta etapa, órgãos em P2+. Importação livre; perfis ajustáveis; cloud configurável com veto para conteúdo sensível ou duvidoso.

- [x] Tela inicial e configuração nativas independentes do chat e da IA.
- [x] Preferências persistentes, detecção de RAM/CPU/GPU, perfis 25/50/75% ajustáveis e reserva de 2 GiB para o sistema. Estimativa de admissão conservadora; medir consumo real antes de recomendar modelos.
- [x] Importar/remover cópias locais compatíveis: GGUF e projetor para visão, ggml para whisper, pasta VITS/ONNX com tokens e dados auxiliares para síntese. Modelos não são baixados nem escolhidos por padrão.
- [x] Instalar runtimes após confirmação do tamanho, com progresso, hash, cancelamento, extração protegida e versões fixadas. Um worker por vez, carregamento sob demanda, cancelamento e liberação por inatividade/saída.
- [x] Seleção por pedido (texto/raciocínio/visão), orçamento e opção manual. Ausência de modelo gera orientação e não inicia cloud automaticamente.
- [x] Modos cloud configuráveis, revisão do payload e consentimento; credencial opcional no cofre do sistema. Histórico, órgãos e anexos não seguem automaticamente. Transcrição de áudio cloud bloqueada nesta etapa.
- [x] SQLite nativo com migrations, conversas isoladas, contexto limitado, gravação atômica do turno, respostas incompletas, backup e importação explícita do histórico legado.
- [x] Interfaces para voz e memória semântica; microfone iniciado pelo usuário (até 30 s), WAV local e playback. Síntese local inicialmente VITS; liberdade de importação limitada aos formatos suportados pelos runtimes.
- [x] Inferência real temporária nas seis capacidades; cancelamento após início do streaming mata o worker; descarregamento por inatividade confirmado.
- [x] Verificações finais concluídas; usuário confirmou tela inicial/abas, alterou e salvou perfil e fechou a janela. JSON temporário confirmou perfil leve, modo local e onboarding concluído; processo e workers encerraram.

**Aceite desktop:** teste em `/tmp/ayla-p1-ui-smoke`, sem dados pessoais. Aplicação iniciou sem backend Python/modelos. Preferências salvas com permissão 0600; encerramento confirmado. README e lockfile NPM preexistentes preservados. Commit/push desta etapa autorizado pelo usuário após verificações.

**Evidência automatizada final:** 18 testes Rust passaram (2 de rede/modelos ficam ignorados por padrão e foram executados separadamente com autorização); 21 testes Python legados passaram; cliente SSE, TypeScript estrito, build Vite e lint JS passaram. As quatro distribuições oficiais foram baixadas em pasta temporária, verificadas por tamanho/SHA-256, extraídas e executadas com `--help`. ABI do adaptador de síntese conferida com header oficial. Resultados de inferência e medições estão abaixo.

**Inferência real em dados temporários:** 1.340.998.042 bytes de pesos/arquivo de voz, mais imagem pública de teste de 694.498 bytes; fontes Qwen, ggml-org, ggerganov/whisper.cpp e k2-fsa. Qwen3 0.6B Q8 respondeu em português e resolveu soma; SmolVLM 256M Q8 descreveu gatos e controles do exemplo oficial; EmbeddingGemma 300M Q4 gerou 768 dimensões; VITS Piper pt_BR Faber gerou WAV; whisper tiny transcreveu o áudio com erros de palavras/nome próprio. Nenhum modelo virou padrão e nenhuma chamada cloud foi feita.

**Amostra local, sem valor de benchmark:** texto 481 ms totais, primeiro conteúdo 30 ms após iniciar a chamada HTTP; raciocínio 921 ms totais, primeiro conteúdo 436 ms. RSS observado do worker ~303 MB no Qwen e ~348 MB em embeddings; RSS não mede VRAM nem memória total. Tempos totais incluem preparação/carregamento, primeiro conteúdo HTTP não inclui essa etapa. Chunks SSE não são contagem de tokens. A qualidade do modelo visual pequeno foi insuficiente em figuras sintéticas simples; importação compatível não equivale a homologação de qualidade. Detalhes dos modelos usados não se aplicam automaticamente à shortlist.

**Limites explícitos:** Linux x86_64 nos pacotes atuais; instalador e Windows depois. Perfis são estimativas, não garantias contra todo OOM. Sugestão de cloud não dispensa revisão/autorização. Shortlist abaixo ainda não homologada. Sem chamada cloud paga, conta real ou órgão conectado nesta validação.

### Refinamento de experiência após P1

- [x] Conversa como entrada; aba Início removida. Navegação principal reduzida a Conversa e Configurações. Primeira mensagem cria a conversa, sem exigir um passo anterior.
- [x] Perfil Ayla automático por padrão: orçamento acompanha hardware, seleção de modelo acompanha pedido; escolhas manuais existentes são preservadas e podem ser restauradas ao automático.
- [x] Preferência de uso (Automático, Leve, Moderado, Máximo) visível fora das opções avançadas. Modelos, componentes, ferramentas e limites técnicos em opções avançadas recolhidas. Controles técnicos por resposta também recolhidos; revisão de cloud continua visível quando aplicável.
- [x] Personalidade em configuração simples: texto livre (até 4.000 caracteres), persistência e restauração da original. Aplicada nas próximas respostas locais/cloud; cloud exige revisão do pedido e da personalidade e mantém veto de privacidade. Mudança após revisão invalida envio.
- [x] Perfil principal pronto numa instalação nova. Numa instalação sem modelo, a conversa recomenda o melhor modelo do catálogo que cabe no orçamento e mostra o tamanho total para confirmação. Ao confirmar, baixa e verifica o llama.cpp (Vulkan só com GPU dedicada e carregador Vulkan; senão CPU) e o modelo, com progresso e interrupção. A tela Modelos também lista o catálogo. Um arquivo serve várias capacidades, e a conversa escolhe o maior modelo que cabe; voz e memória continuam com a escolha estável. O catálogo usa a memória medida mais 20% de margem. O contexto padrão é 8192, com preferências na versão 2.
  - **Aceite:** 27 testes Rust (2 ignorados por padrão), incluindo registro legado, seleção, catálogo fixado, instalação com servidor local (sucesso, hash divergente e cancelamento sem resíduos), migração de preferências e flag de raciocínio. Também passaram 21 testes Python, o cliente SSE, TypeScript estrito, build Vite, lint e `git diff --check`. As quatro URLs fixadas conferem em tamanho e SHA-256. No desktop, com dados temporários em `/tmp/ayla-setup-smoke`, o usuário confirmou o download de 6,1 GB e conversou com texto e imagem. O registro ficou com uma única cópia do Gemma, com texto, raciocínio e visão, hashes conferidos e llama-vulkan instalado. Primeiro trecho em 2,4 s, com carga, e 1,6 s.
  - **Limites:** sem retomada de download interrompido e sem checagem prévia de espaço em disco; a falta de espaço aparece como erro de escrita. O orçamento é calculado sobre a RAM, mas boa parte do modelo vai para a VRAM. No perfil automático, máquinas com menos de 12 GB de RAM (25%) não comportam nenhum modelo do catálogo. Encerrar o app por sinal deixou o `llama-server` órfão (o fechamento normal já estava validado); pendente tratar.
**Validação deste refinamento:** 20 testes Rust (2 de modelos/rede ignorados por padrão), 21 testes Python, cliente SSE, TypeScript estrito, build Vite, lint JS e `git diff --check` passaram. Revisão em navegador com IPC simulado: conversa, configurações, preferência fora das opções avançadas, salvar perfil, editar/restaurar personalidade, modelos, voz/memória e confirmação de operação. Tema padronizado com cores explícitas para campos, seleções, ações, foco e estados desabilitados. Essa revisão visual não é uma nova homologação nativa de inferência.

- [x] Inspeção visual em navegador com fixtures deste refinamento; aceite desktop nativo anterior preservado como evidência separada.

### Shortlist inicial de modelos — proposta do usuário

Shortlist inicial reafirmada pelo usuário por imagem: Qwen3.5 0.8B, 4B e 9B; Gemma 4 E4B e 12B; GPT-OSS 20B. Lista provisória e sujeita a alterações após benchmarks. Sugestões opcionais para comparar; não são modelos padrão nem uma associação fixa aos perfis. Importação livre preservada. Compatibilidade de cada arquivo com a versão gerenciada do runtime ainda precisa ser medida.

| Modelo | Papel proposto | Pesos Q4 informados pelo usuário |
|---|---|---|
| Qwen3.5 0.8B | Ultraleve, comandos simples | 0,5–0,8 GB |
| Qwen3.5 4B | Principal econômico | 2,5–3,5 GB |
| Gemma 4 E4B | Alternativa leve, multimodal | Depende da variante |
| Qwen3.5 9B | Principal equilibrado | 5–6 GB |
| Gemma 4 12B | Qualidade superior, a comparar | 7–9 GB |
| GPT-OSS 20B | Raciocínio mais exigente | Cerca de 12 GB; distinguir MXFP4 de Q4 |

**Verificação em 19/11/2026:** os tamanhos citados existem nas fontes oficiais. Os valores acima são estimativas iniciais do usuário, não medições da Ayla. RAM/VRAM total inclui contexto, buffers e componentes multimodais. O “E” de Gemma indica parâmetros efetivos, não o tamanho completo dos pesos. Google lista referências Q4_0 de 4,5 GB para E4B e 6,7 GB para 12B, com condições próprias; a OpenAI referencia sistemas com pelo menos 16 GB para GPT-OSS 20B em MXFP4. Não comparar esses números como se medissem a mesma coisa.

**Decisão de 19/11/2026:** próxima frente é o perfil principal pronto numa instalação nova, antes do P2. Benchmark autorizado com até ~17 GB em pasta temporária: Qwen3.5 0.8B, 4B e 9B e Gemma 4 E4B em Q4, de fontes oficiais, sem definir padrão nem chamar cloud. Gemma 4 12B e GPT-OSS 20B ficam para depois.

**Benchmark preliminar de 19/11/2026.** Hardware: Ryzen 7 7800X3D, 30 GB de RAM, RX 9060 XT com 16 GB (Vulkan). Runtime llama.cpp b12531 gerenciado, com as flags da Ayla e perfil automático de 50%. Persona padrão, 7 prompts em PT-BR, uma execução por prompt e avaliação qualitativa. Arquivos em `/var/tmp/ayla-benchmark`, verificados por SHA-256.

| Modelo (arquivo) | Fonte | GPU tg / CPU tg | Carga | VRAM / RSS | Qualidade observada |
|---|---|---|---|---|---|
| Qwen3.5 0.8B Q4_0 (0,56 GB) | ggml-org | 264 / 60 t/s | 0,5 s | 0,65 / 0,54 GiB | Inadequado em PT: errou o cálculo, alucinou e ignorou o JSON. |
| Qwen3.5 4B Q4_K_M + mmproj Q8 (3,4 GB) | ggml-org | 78 / 11 t/s | 1,6 s | 3,4 / 0,8–1,0 GiB | Boa conversa e boa visão; cálculo certo; errou a correção gramatical; colocou texto de conversa dentro do JSON. |
| Gemma 4 E4B QAT Q4_0 + mmproj (6,1 GB) | Google | 79 / 14 t/s | 1,7 s | 3,8 / 3,0 GiB | Melhor do conjunto: correção gramatical certa, JSON correto, visão correta e raciocínio curto (~750 tokens). Verboso, com muitos emojis por causa da persona. |
| Qwen3.5 9B Q4_K_M (5,7 GB) | Unsloth (terceiro, autorizado) | 49 / 8 t/s | 2,1 s | 5,0 / 0,8 GiB | Sem ganho sobre o 4B nestes prompts; errou a gramática e escreveu "remetete" no JSON. Visão não testada. |

- **Raciocínio:** com o contexto padrão de 2048 (máximo de 1024 tokens de resposta), o Qwen 4B e o 9B esgotaram o limite antes de responder. Com 8192, responderam certo, mas após 2.250 e 2.745 tokens (28 s e 51 s até a resposta). O Gemma E4B levou 6,6 s. Nos Qwen, o contexto de 8192 custou pouca VRAM a mais: +0,1 GiB no 4B e +0,6 GiB no 9B.
- **Correção aplicada:** o Qwen3.5 raciocina por padrão, mas a Ayla não enviava `enable_thinking` e descartava `reasoning_content`, então pedidos de texto também raciocinavam sem exibir nada. Agora o modo só liga em pedidos roteados para raciocínio. Há teste em `runtime.rs`.
- **Lacunas encontradas:** `models::choose` escolhe o *menor* modelo que cabe no orçamento, então com 0.8B e 4B instalados usaria o 0.8B. O registro também aceita uma única capacidade por arquivo, o que obrigaria importar o mesmo Gemma três vezes (texto, raciocínio e visão).
- **Limites:** amostra pequena, uma execução por prompt e avaliação feita pelo agente. Não mede transcrição, síntese nem embeddings. Os números de GPU valem para este hardware.

**Decisões após o benchmark (19/11/2026):**

- O catálogo inicial terá Gemma 4 E4B QAT como principal (texto, raciocínio e visão) e Qwen3.5 4B como alternativa leve. Qwen 0.8B e 9B ficam fora.
- Os arquivos vêm do Hugging Face, com revisão e hash fixos, e só são baixados após confirmação do tamanho.
- Um arquivo de modelo pode declarar várias capacidades. Os modelos já registrados são migrados com a capacidade que tinham.
- O contexto padrão passa a ser 8192 tokens, preservando valores escolhidos pelo usuário.
- A instalação principal cobre só conversa e visão. Voz e memória semântica serão oferecidas no primeiro uso, depois de um benchmark próprio.

**Próximo aceite:** confirmar arquivo, quantização, licença, compatibilidade, qualidade em português, latência e memória no hardware alvo antes de recomendar downloads. A shortlist não cobre modelos especializados de transcrição, síntese ou embeddings. Não baixar a lista inteira: a autorização atual cobre somente até 1,5 GB de modelos temporários de validação do P1.

Fontes: [Qwen 0.8B](https://huggingface.co/Qwen/Qwen3.5-0.8B), [Qwen 4B](https://huggingface.co/Qwen/Qwen3.5-4B), [Qwen 9B](https://huggingface.co/Qwen/Qwen3.5-9B), [Gemma 4](https://ai.google.dev/gemma/docs/core), [GPT-OSS](https://deploymentsafety.openai.com/gpt-oss/architecture).

### Memória e voz — próxima frente (antes do P2)

**Decisões de 19/11/2026:**

- **Memória longa automática:** após cada conversa, a IA extrai fatos e preferências e os guarda localmente. Há comandos explícitos ("lembre que…", "esqueça…") e um painel para ver, editar e apagar memórias. Cada memória tem uma marcação de privacidade, que começa como duvidosa e só deixa de ser assim por classificação ou por você no painel. Essa marcação é o que a regra de nuvem consulta.
- **Memória e nuvem:** referências (memórias, arquivos, conversas anteriores) podem acompanhar um envio à nuvem quando forem relevantes e não tiverem conteúdo sensível ou duvidoso. Elas aparecem listadas na revisão do envio, que continua obrigatória. Histórico e órgãos nunca vão em bloco. O `AGENTS.md` foi atualizado com essa regra.
- **Armazenamento:** SQLite embutido, com busca por palavras (FTS5) e vetores em Rust. O Qdrant fica para quando o volume exigir; o README continua como visão.
- **Benchmark autorizado:** até ~2,5 GB em `/var/tmp/ayla-benchmark`. Embeddings: EmbeddingGemma 300M e Qwen3-Embedding 0.6B. Transcrição: Whisper small e large-v3-turbo. Fala: vozes Piper pt-BR e Kokoro.

**Ordem proposta:** benchmark de embeddings → memória longa (esquema, extração, busca híbrida e uso no contexto local) → painel de memórias → referências na revisão de envio à nuvem → benchmark e integração de voz na conversa.

**Benchmark de embeddings (30 fatos sintéticos, 20 perguntas parafraseadas):**

| Modelo | Acerto no 1º / nos 3 primeiros | Consulta na CPU | RAM |
|---|---|---|---|
| EmbeddingGemma 300M QAT Q4 | 90% / 95% | 7 ms | 390 MB |
| Qwen3-Embedding 0.6B Q8 | 85–90% / 100% | 17–38 ms | 900 MB |

Limiares calibrados: busca a partir de 0,42 (a melhor nota de uma pergunta sem relação foi 0,41), mesma memória a partir de 0,88, memória relacionada a partir de 0,45.

**Decisões:** EmbeddingGemma como padrão (licença Gemma Terms of Use, baixado do ggml-org). Worker de memória na CPU ao lado do modelo de conversa (revisa a decisão do P1 de um worker ativo). Aviso de memória nova com nota discreta, desfazer e painel.

- [x] **Memória longa local.**
  - Banco na v2, com backup automático antes da migração. Tabelas de fatos e de vetores binários por modelo, índice FTS5 e cursor de extração por conversa. As notas antigas viram memórias manuais, com vetores recalculados.
  - Busca híbrida (vetores + FTS5, fusão por rank). Até 5 memórias entram no prompt de sistema, só em respostas locais.
  - A extração usa o modelo de conversa com JSON Schema, recebe as memórias relacionadas e decide entre lembrar, substituir ou esquecer. Roda após 45 s sem mensagens ou logo após "lembre que…"/"esqueça…". Uma nova mensagem cancela a extração sem descarregar o modelo.
  - A classificação automática nunca marca uma memória como pública.
  - Painel "O que a Ayla lembra" nas Configurações (adicionar, editar, privacidade, apagar, pausar a extração automática). Nota "Ayla lembrou: … · desfazer" na conversa e oferta de ativar a memória (0,3 GB).
  - **Aceite:** 33 testes Rust (migração com backup, fatos/FTS/vetores, parsing da extração, fusão, catálogo); TypeScript estrito, build, lint e cliente SSE. Teste com modelos reais (ignorado por padrão): extraiu 3 fatos duradouros e ignorou o pedido pontual; achou o pet em 14 ms; nada voltou para uma pergunta sem relação; "me mudei para Recife" substituiu Fortaleza; "esqueça a gata" removeu a memória; uma pergunta geral não gerou memória. No desktop, com dados em `/var/tmp/ayla-memory-smoke`, o usuário confirmou o fluxo completo: preparar, ativar memória, "lembre que", uso em outra conversa e painel. Primeiro trecho em 4,8 s na primeira mensagem (carga dos dois modelos) e 133 ms na seguinte.
  - **Limites:** a extração depende de um modelo de conversa local; sem ele, as conversas ficam pendentes. Uma falha de extração avança o cursor para não repetir em laço. O worker de memória usa o pacote Vulkan com `-ngl 0` quando só ele está instalado. Busca por força bruta, adequada até ~100 mil memórias. Referências à nuvem ainda não implementadas.
- [x] **Modelos padrão sem clique** (decisão do usuário em 19/11/2026, substitui a confirmação de download). Ao abrir, o núcleo prepara sozinho o que falta: o llama.cpp, o melhor modelo de conversa para o hardware e, com a memória ligada, o modelo de memória. A interface mostra o progresso, com interromper e tentar de novo, e trocar de tela não cancela. Nunca roda no modo somente nuvem nem quando nenhum modelo cabe no orçamento. Quando houver instalador, avaliar embutir ou baixar os modelos durante a instalação.
  - **Cache de desenvolvimento:** `AYLA_MODEL_CACHE=/var/tmp/ayla-benchmark` reaproveita arquivos por nome, só se tamanho e SHA-256 conferirem (com hardlink), e guarda downloads novos. O `sha2` compila otimizado no perfil dev.
  - **Aceite:** 34 testes Rust, incluindo o cache (válido, adulterado e alimentação). Instalação nova sem clique, confirmada pelo usuário: conversa pronta com memória ativa. O preparo a partir do cache levou ~5 s. Antes da otimização, o hash em debug deixava a barra parada por minutos, o que levou à correção do progresso.
- [ ] Referências (memórias) na revisão de envio à nuvem, conforme a regra do `AGENTS.md`.
- [x] **Benchmark de voz (19/11/2026, CPU).**
  - **Transcrição** (40 trechos reais do FLEURS pt_br, CC-BY-4.0, ~12 s cada): tiny teve 20% de erro de palavras (0,4 s, 177 MB); small, 6,9% (2,2 s, 751 MB); large-v3-turbo q5, 4,1% (7,1 s, 794 MB). Não há build oficial do whisper.cpp com GPU para Linux.
  - **Síntese** (4 frases; inteligibilidade medida pelo erro do Whisper small ao reouvir): Piper faber/cadu (CC0) tiveram 16%/12% de erro, 0,5 s por frase e 200 MB. Piper dii/miro têm licença CC BY-NC-SA (não comercial) e ficaram em 25–26%. Kokoro dora/alex/santa (Apache-2.0) tiveram 3–6%, 1,6 s por frase e 730 MB; o carregamento custa ~0,75 s fixo por chamada. Todas leem mal "R$" e datas.
- **Decisões de voz (19/11/2026):** voz padrão Kokoro Dora; transcrição padrão Whisper small. Nesta etapa, microfone e "ouvir" na conversa: a resposta a uma fala é lida frase a frase durante a geração, e o modo contínuo fica para depois. A voz é opcional: na primeira abertura a Ayla pergunta se a pessoa quer usar voz; se sim, baixa sozinha (~0,9 GB). Dá para ativar depois em Configurações.
- [x] **Integração de voz na conversa.**
  - A síntese roda sem Python, pelo binário do sherpa-onnx (Kokoro e VITS). Voz e transcrição usam processos e uma fila próprios e não descarregam o modelo de conversa.
  - Normalizador determinístico em PT-BR: números, R$, datas, horas e porcentagens por extenso; remove markdown e emojis.
  - Na conversa: microfone, resposta falada frase a frase durante a geração, "ouvir" e "parar". Pergunta inicial "Quer conversar por voz?" e opção em Configurações; o preparo automático instala o Whisper small, o Kokoro Dora e os motores.
  - O WebKitGTK negava o microfone: a mídia passou a ser liberada só para áudio, na janela da Ayla, via `webkit2gtk` 2.0.2 (mesma versão do Tauri; dependência direta só no Linux, aprovada pelo usuário). O MediaRecorder travava no WebKitGTK; a gravação passou a capturar PCM pela Web Audio API.
  - **Aceite:** 37 testes Rust (normalizador e catálogo de voz); 8 testes do cliente (frases, WAV, reamostragem). Teste real (ignorado por padrão): instalação via cache, fala em 1,5 s e transcrição de volta em 3,1 s, com o valor e a data corretos. No desktop, o usuário confirmou microfone, resposta falada e ativação automática da voz.
  - **Feedback do usuário:** a voz soa estranha; o ícone do microfone era ruim (trocado por SVG); o estilo geral do app ainda é feio; quer ver a transcrição aparecer enquanto fala.
  - **Limites:** cada frase pagava ~0,75 s de carga do Kokoro. A fala nas respostas saiu em 20/11/2026 (ver decisão abaixo).
- [ ] **Transcrição ao vivo** (pedido do usuário; decisão de 20/11/2026: o Whisper tiny, 78 MB, vem junto quando o usuário escolhe o modo voz). Durante a gravação, o tiny transcreve o áudio acumulado a cada ~1,5 s e mostra no campo de texto. O texto enviado continua vindo do Whisper small. Implementado sem commit. Teste real (ignorado por padrão, via cache) num trecho do FLEURS: small em 3,5 s e tiny em 0,6 s; o tiny erra nomes próprios ("esra eu" por "Israel"), aceitável para prévia. Falta a confirmação no desktop.
- **Decisão de 20/11/2026 (substitui a fala nas respostas):** nenhuma voz testada agradou; todas soam como inglês adaptado ao português. Por ora, a voz é só entrada: a Ayla ouve pelo microfone e responde somente em texto (no futuro, também com imagens e outros formatos). Ela deve poder enviar notificações pelo sistema.
- [x] **Fala nas respostas removida** (sem commit). Saíram Kokoro, sherpa-onnx, "ouvir", leitura frase a frase, síntese na nuvem, normalizador de fala, adaptador Python e a dependência `bzip2`; o histórico do Git preserva. Ativar o microfone baixa só a transcrição (Whisper small e tiny). Modelos de voz já baixados continuam no registro como "Voz de saída (sem uso)" para remoção em Opções avançadas; o motor `runtimes/speech` já baixado fica no disco.
  - **Aceite:** 36 testes Rust, 7 do cliente, 21 do backend, TypeScript estrito, build, lint, `cargo check --locked` e `git diff --check`. Teste real de transcrição acima. Falta conferir no desktop.
- [ ] **Notificações do sistema** (sem commit). Plugin oficial do Tauri (`tauri-plugin-notification` 2.4), enviado só pelo núcleo em Rust, sem dependência JS. Avisam com a janela fora de foco e nunca mostram conteúdo de conversa. Ligadas por padrão e ajustáveis em Configurações: resposta pronta e preparo concluído ou com erro (cancelamento não avisa). Preferências ganharam `notifications` sem mudar de versão; arquivos antigos recebem o padrão. Testes de decisão e de preferências passam; falta ver a notificação no desktop.
  - **Lembretes** ("me lembra às 15h") também avisarão, ligados por padrão. Exigem agendador e decisões próprias (como a Ayla entende o pedido, se roda em segundo plano com a janela fechada, início com o sistema).
- [x] **Microfone por padrão** (decisão de 20/11/2026). Vem ligado e entra no primeiro preparo, junto com conversa e memória (Whisper small e tiny); a pergunta inicial saiu. Arquivos antigos sem escolha valem como ligado; só quem desligou fica desligado. Teste de preferências cobre os três casos.
- [x] **Colar imagens** (Ctrl+V na caixa de mensagem). PNG/JPEG de até 20 MB viram um anexo local temporário em `work/pasted`, apagado depois do envio (e ao abrir o app, se sobrar). Anexos escolhidos pela pessoa nunca são apagados. Teste cobre validação e limpeza; falta conferir no desktop.
- [ ] **Visual do app.** Três direções propostas em 20/11/2026; o usuário escolheu **A · Foco**: escuro neutro com laranja como destaque, barra lateral de conversas, respostas sem balão, caixa de texto única com ícones e Configurações em listas com interruptores. Fonte do sistema, para cara de app nativo (decisão de 20/11/2026; nada de fonte embutida ou baixada).
  - **Implementado (sem commit):** tema neutro com laranja só de destaque e fonte do sistema. Na conversa: barra lateral recolhível com as conversas (no lugar da lista suspensa), cabeçalho com título e indicador Local/Nuvem, respostas sem balão com "Copiar" e "Pensando…", aviso de memória com "Desfazer" e rolagem que acompanha a resposta. A caixa de texto agora é única e cresce com o texto (Enter envia, Shift+Enter quebra linha), com ícones de anexar e de ajustes desta resposta, botões redondos de microfone/parar/enviar e um cronômetro "Ouvindo · 0:06" com a prévia da transcrição. "Nova conversa" só limpa a tela; a conversa nasce na primeira mensagem. A conversa continua aberta ao ir para Configurações, então uma resposta em andamento não é mais cancelada. Configurações: barra de seções, grupos em cartões, perfil em controle segmentado e interruptores que salvam na hora (microfone, notificações, memória). Limites técnicos, nuvem, dados e medições ficam em "Recursos, nuvem e dados".
  - **Validação:** TypeScript estrito, lint, build e 7 testes do cliente. Não houve revisão visual automatizada (o ambiente não tem navegador headless); falta a conferência do usuário no desktop.

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

Athena e outros órgãos → Biblioteca e retrospectivas. Mercúrio permanece como proposta de comparação de preços. Evolução de voz e multimodalidade será guiada pelos casos de uso e medições do P1. Instalador Linux e, posteriormente, port Windows terão escopo próprio.

## 5. Comandos de desenvolvimento e verificação

- Desktop Linux: na pasta `frontend`, `npm run tauri -- dev`. Para testar sem baixar modelos de novo: `AYLA_MODEL_CACHE=/var/tmp/ayla-benchmark AYLA_DATA_DIR=<pasta descartável>`. Requer Rust/Cargo (mínimo declarado 1.89 pelas dependências Linux; validado em 1.99.1) e WebKitGTK. A `.venv` é necessária para síntese local pelo adaptador auxiliar e para o backend legado; a tela inicial nativa não depende dela. Nenhuma IA precisa estar instalada. `AYLA_PYTHON` permite apontar outro interpretador.
- Backend: da raiz, `PYTHONPATH=backend .venv/bin/python -m unittest discover -s backend/tests -v`.
- Cliente/build: em `frontend`, `npm test`, `npm run build` e `npm run lint`.
- Tipos: em `frontend`, `tsc --noEmit --strict --jsx react-jsx --module esnext --moduleResolution bundler --target es2022 --lib es2022,dom --allowImportingTsExtensions --skipLibCheck src/App.tsx src/components/Chat.tsx src/services/Api.ts` (compilador disponível neste ambiente).
- Desktop: em `frontend/src-tauri`, `cargo check --locked`; teste de processo com banco descartável inexistente: `AYLA_DB_PATH=/tmp/ayla-lifecycle-test.db AYLA_LOCAL_ENABLED=false AYLA_CLOUD_ENABLED=false cargo test --locked --lib`.
- Backup: `python backend/backup.py`. Restauração: fechar a Ayla, preservar o banco atual e copiar o backup para o caminho do banco. `AYLA_DB_PATH` substitui o banco legado. O banco nativo fica no diretório de dados do Tauri; `AYLA_DATA_DIR` permite um diretório descartável em desenvolvimento. Use o backup nativo na tela de ajustes para os novos dados.

As variáveis documentadas em `backend/.env.example` são recursos de desenvolvimento, não a experiência de instalação do produto. O arquivo não é carregado automaticamente. Neste ambiente, o Cargo foi validado com cache temporário em `/tmp/ayla-p0-cargo`; isso não é requisito do projeto.
