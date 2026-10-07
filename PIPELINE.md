# Ayla — Pipeline de Desenvolvimento

Este documento acompanha a implementação da Ayla e funciona como um mapa de prioridades.

A intenção é evitar que o projeto cresça apenas pela quantidade de funcionalidades. Cada etapa deve fortalecer a fundação para que os próximos módulos possam ser adicionados sem criar dependências desnecessárias.

> **Regra geral:** primeiro dados e infraestrutura, depois serviços, depois inteligência.

---

# Direção arquitetural

- A Ayla funciona sem LLM.
- Os órgãos funcionam sem depender da IA.
- A IA utiliza serviços e dados através de interfaces bem definidas.
- Os dados pertencem ao usuário.
- A infraestrutura pode ser reproduzida com Docker.
- Serviços pesados podem ser iniciados e parados independentemente.
- Performance é medida, não presumida.
- Não criar abstrações antes de existir uma necessidade real.

---

# — Separar a aplicação da IA

## Objetivo

Fazer a Ayla iniciar e executar suas funções básicas sem nenhum modelo carregado.

- [ ] Criar camada de serviços.
- [ ] Remover dependências desnecessárias de Ollama.
- [ ] Separar lógica de negócio da lógica de conversa.
- [ ] Definir interfaces para os órgãos.
- [ ] Permitir que a UI acesse serviços diretamente.
- [ ] Permitir que a IA utilize os mesmos serviços posteriormente.

```text
              ┌── UI
Service ──────┼── IA
              └── Automação
```

---

# — Banco compartilhado

- [ ] Definir banco principal.
- [ ] Decidir o papel de SQLite e PostgreSQL.
- [ ] Criar migrations.
- [ ] Criar camada de acesso a dados.
- [ ] Definir IDs e relacionamentos.
- [ ] Separar tabelas por domínio.
- [ ] Definir eventos internos.
- [ ] Definir estratégia de backup.

---

# — Docker / Infraestrutura

## Primeira composição possível

```text
docker compose
├── ayla-backend
├── ayla-frontend
├── postgres
├── redis
└── qdrant
```

Depois:

- [ ] Healthchecks
- [ ] Volumes persistentes
- [ ] `.env`
- [ ] Secrets
- [ ] Logs
- [ ] Backups
- [ ] Compose de desenvolvimento
- [ ] Compose de produção

Não containerizar tudo apenas porque é possível. GPU, áudio, hardware e integração profunda com o sistema operacional podem permanecer fora do Docker.

---

# — Moneta

## Domínio

- [ ] Contas
- [ ] Instituições
- [ ] Transações
- [ ] Categorias
- [ ] Orçamentos
- [ ] Investimentos
- [ ] Patrimônio

## Importação

- [ ] CSV
- [ ] OFX
- [ ] Outros formatos relevantes

## Pluggy

Estudar o [Securo](https://github.com/securo-finance/securo) como referência para:

- [ ] autenticação;
- [ ] criação de conexão;
- [ ] atualização;
- [ ] contas;
- [ ] transações;
- [ ] investimentos;
- [ ] tratamento de erros;
- [ ] sincronização incremental.

Depois:

- [ ] Implementar integração própria na Moneta.
- [ ] Testar sincronização real.
- [ ] Tratar contas e investimentos separadamente.
- [ ] Criar mecanismo de atualização.

## Independência

A Moneta deve funcionar assim:

```text
Pluggy
   ↓
Moneta
   ↓
Banco
   ↓
Dashboard
```

sem LLM.

**Resultado esperado:** uma primeira versão da Moneta capaz de sincronizar dados do Pluggy, armazená-los e apresentá-los de maneira útil sem IA.

---

# — Services / Tools

- [ ] Criar interfaces de serviços.
- [ ] Definir operações seguras.
- [ ] Criar ferramentas para a IA.
- [ ] Permitir que UI e IA chamem os mesmos serviços.
- [ ] Implementar permissões.
- [ ] Adicionar confirmação para ações sensíveis.

```text
UI ────────────────┐
                   ├──> MonetaService
IA ──> Tool ───────┘
```

---

# — Performance da IA

Comparar:

- Ollama
- llama.cpp
- outros runtimes locais relevantes

Critérios:

- latência;
- VRAM;
- RAM;
- CPU;
- qualidade;
- streaming;
- AMD/Vulkan;
- gerenciamento de modelos;
- descarregamento de modelos.

---

# — Pipeline de IA

Estudar os princípios usados no [VoiceAssistant](https://github.com/diegormirhan/voice-assistant):

- [ ] `asyncio`
- [ ] streaming
- [ ] processos separados
- [ ] lazy startup
- [ ] VAD
- [ ] interrupção
- [ ] métricas
- [ ] `whisper.cpp`
- [ ] `llama.cpp`
- [ ] Vulkan

A referência trabalha com uma meta de menos de 2 segundos entre o fim da fala e a primeira sílaba da resposta.

O objetivo é aprender com esses princípios, não copiar a implementação.

---

# — Pipeline mensurável

```text
Entrada
  ↓
VAD / captura
  ↓
STT
  ↓
Intent / classificação
  ↓
Memória curta
  ↓
RAG / memória longa
  ↓
Seleção de ferramentas
  ↓
LLM
  ↓
Streaming
  ↓
TTS
```

Cada etapa deve possuir, quando aplicável:

- timestamp;
- duração;
- erro;
- consumo aproximado;
- possibilidade de cancelamento.

---

# — Orçamento de performance

Como ponto de partida:

| Etapa | Meta inicial |
|---|---:|
| VAD | < 100 ms |
| STT | < 600 ms |
| classificação | < 100 ms |
| RAG | < 150 ms |
| primeiro token | < 500 ms |
| TTS inicial | < 300 ms |

Os benchmarks reais da Ayla devem determinar os valores finais.

---

# — Memória

- [ ] Memória curta
- [ ] Memória longa
- [ ] Qdrant
- [ ] Embeddings
- [ ] Semantic cache
- [ ] Classificação paralela
- [ ] Deduplicação
- [ ] Atualização/expiração de memórias

```text
                 ┌── Memória curta
Input → Router ──┼── Memória longa
                 ├── RAG
                 └── Contexto externo
```

---

# — IA ↔ Órgãos

Começar por:

- [ ] Moneta
- [ ] Athena

Depois:

- [ ] Pulse
- [ ] Arcade
- [ ] Hand
- [ ] Hermes
- [ ] Iris
- [ ] Odin

Exemplo:

```text
"Quanto gastei com jogos este mês?"

              ↓
           Intent
              ↓
       Moneta + Arcade
              ↓
        Dados reais
              ↓
             LLM
              ↓
           Resposta
```

A IA interpreta os dados. Ela não deve ser a fonte dos dados.

---

# — Voz

- [ ] STT local
- [ ] VAD
- [ ] TTS local
- [ ] Streaming
- [ ] Barge-in
- [ ] Cancelamento
- [ ] Controle de recursos

A voz deve ser uma interface da Ayla, não uma dependência de toda a aplicação.

---

# — Multimodal

- [ ] Screenshots
- [ ] Iris
- [ ] Visão
- [ ] PDFs
- [ ] Imagens
- [ ] Contexto visual opcional

Sempre com controles claros de privacidade.

---

# — Órgãos secundários

Após Moneta + Athena + Core:

- [ ] Pulse
- [ ] Arcade
- [ ] Hand
- [ ] Hermes
- [ ] Iris
- [ ] Odin

A prioridade deve ser baseada em utilidade real, e não na quantidade de módulos.

---

# — Biblioteca

Integrar:

- [ ] Música
- [ ] Jogos
- [ ] Filmes e séries
- [ ] Livros e mangás
- [ ] Outras mídias

Criar conceitos comuns para itens, pessoas/artistas, datas, eventos, histórico e relações.

---

# — Retrospectivas

- [ ] Retrospectiva mensal
- [ ] Retrospectiva anual
- [ ] Retrospectiva cultural
- [ ] Retrospectiva financeira
- [ ] Retrospectiva de projetos
- [ ] Retrospectiva técnica

---

# — Qualidade e observabilidade

- [ ] Logs estruturados
- [ ] Métricas
- [ ] Tracing básico
- [ ] Testes unitários
- [ ] Testes de integração
- [ ] Testes dos órgãos sem IA
- [ ] Testes do pipeline de IA
- [ ] Benchmark de modelos
- [ ] Benchmark de memória
- [ ] Benchmark de VRAM/RAM

---

# — Segurança e privacidade

- [ ] Secrets fora do Git
- [ ] Permissões por ferramenta
- [ ] Confirmação para ações destrutivas
- [ ] Logs sem dados sensíveis
- [ ] Backup seguro
- [ ] Controle explícito de acesso à rede
- [ ] Modo completamente offline para funcionalidades locais

---

# Critério de pronto

Uma funcionalidade só deve ser considerada pronta quando:

1. Funciona sem IA quando não precisa de IA.
2. Possui testes básicos.
3. Tem persistência definida.
4. Pode ser chamada pela UI.
5. Pode ser chamada pela IA quando fizer sentido.
6. Não bloqueia o restante da aplicação.
7. Possui tratamento de erros.
8. Possui documentação mínima.

---

# Próximo sprint — Retomar a Ayla

## Objetivo

Voltar ao projeto sem tentar reconstruí-lo inteiro de uma vez.

1. [ ] Abrir o código atual.
2. [ ] Mapear a arquitetura real.
3. [ ] Rodar o projeto.
4. [ ] Confirmar backend.
5. [ ] Confirmar frontend/Tauri.
6. [ ] Confirmar SQLite e memória.
7. [ ] Identificar onde Ollama é obrigatório.
8. [ ] Criar a primeira camada de serviços independente da IA.
9. [ ] Definir o esqueleto da Moneta.
10. [ ] Avaliar Docker.
11. [ ] Comparar a arquitetura real com o README.
12. [ ] Atualizar a documentação conforme as decisões reais.

### Primeiro alvo concreto

> **A Ayla inicia e funciona normalmente sem nenhum modelo de IA carregado.**

Depois:

> **A Moneta consegue existir e operar dentro da mesma aplicação e infraestrutura, também sem IA.**

E somente então:

> **A IA ganha acesso à Moneta através de ferramentas bem definidas.**

---

# Ordem macro

```text
FUNDAÇÃO
   ↓
BANCO
   ↓
DOCKER / INFRA
   ↓
MONETA
   ↓
SERVICES / TOOLS
   ↓
PERFORMANCE DA IA
   ↓
MEMÓRIA / RAG
   ↓
IA ↔ ÓRGÃOS
   ↓
VOZ
   ↓
MULTIMODAL
   ↓
BIBLIOTECA
   ↓
RETROSPECTIVAS
```

---

# Regra de ouro

> **A Ayla deve ser útil sem ser inteligente.**
>
> **E deve ser muito mais poderosa quando fica inteligente.**
