# Projeto Ayla

> Uma plataforma pessoal offline-first que reúne dados, ferramentas, automações e inteligência artificial em um único ecossistema.

## Sobre este projeto

A **Ayla** nasceu da ideia de construir uma assistente pessoal que não seja apenas um chatbot.

Ela deve acompanhar diferentes partes da vida digital do usuário — música, jogos, finanças, conhecimento, arquivos, automações e comunicação — e, com o tempo, aprender a conectar essas informações de maneira útil.

A inteligência artificial é uma parte central dessa visão, mas **não é a fundação inteira do projeto**.

A Ayla deve continuar funcionando quando nenhum modelo estiver carregado. Seus módulos devem conseguir coletar, organizar e apresentar dados por conta própria; a IA entra como uma camada cognitiva capaz de interpretar esses dados, relacioná-los e executar tarefas mais complexas.

É por isso que o projeto é organizado em **órgãos**.

# Filosofia

A Ayla deve crescer conforme exista uma necessidade real.

**Nada deve ser criado sem propósito.**

**Nada deve ser separado sem necessidade.**

**A IA não deve ser usada onde código determinístico resolve melhor.**

**Os dados pertencem ao usuário e devem continuar úteis sem IA.**

A arquitetura deve respeitar:

- privacidade;
- desempenho;
- manutenção;
- modularidade;
- tempo;
- contexto;
- usuário.

---

# 1. Órgãos

Os **órgãos** são módulos especializados da Ayla. Cada um possui uma responsabilidade própria e pode operar independentemente da inteligência artificial.

A ideia é inspirada no funcionamento de um organismo: cada órgão resolve um problema específico, enquanto o conjunto forma algo maior.

Alguns órgãos são principalmente simbióticos, existindo para ampliar as capacidades da Ayla. Outros podem se tornar aplicações ou produtos independentes no futuro.

### Uma regra importante

> **Um órgão não deve precisar de um LLM para executar aquilo que código determinístico já consegue fazer.**

Por exemplo:

- a **Moneta** deve conseguir sincronizar e organizar finanças sem IA;
- a **Athena** deve conseguir criar e consultar notas sem IA;
- o **Pulse** deve conseguir registrar histórico musical sem IA;
- o **Hand** deve conseguir executar automações determinísticas sem IA.

A inteligência artificial pode então utilizar esses mesmos serviços quando houver uma tarefa que realmente se beneficie de linguagem, raciocínio ou contexto.

## 1.1 Pulse

O **Pulse** cuida da dimensão musical da Ayla.

- Integração com plataformas de música, atualmente Spotify
- Coleta de dados musicais
- Histórico e padrões de escuta
- Integração com a Biblioteca
- Futuras retrospectivas musicais

## 1.2 Arcade

O **Arcade** acompanha a dimensão de jogos e entretenimento interativo.

- Rastreamento de jogos
- Histórico de atividade
- Padrões de utilização
- Retrospectivas
- Integração com a Biblioteca

## 1.3 Hand

O **Hand** é a interface entre a Ayla e o computador.

- Execução de comandos
- Automação local
- Controle do sistema operacional
- Ações determinísticas
- Futuras ferramentas controladas pela IA

## 1.4 Hermes

O **Hermes** funciona como o mensageiro da Ayla.

- E-mails
- Eventos
- Notificações
- Parsers
- Agendamentos
- Comunicação com serviços externos

## 1.5 Iris

O **Iris** concentra capacidades relacionadas a imagens, documentos e contexto visual.

1. Análise de imagens, PDFs e arquivos referenciados pela Athena
2. Análise e organização de galerias em nuvem
3. Extração de contexto visual
4. Futuras capacidades multimodais

## 1.6 Moneta

A **Moneta** é o órgão financeiro da Ayla e um dos candidatos mais fortes a funcionar como produto independente.

Responsabilidades:

- Contas
- Instituições
- Transações
- Categorias
- Orçamentos
- Investimentos
- Patrimônio
- Importação de dados
- Sincronização bancária
- Limpeza e normalização
- Consultas e relatórios

### Pluggy

A integração com **Pluggy** é uma das primeiras partes que queremos desenvolver na Moneta.

A experiência prática com o [Securo](https://github.com/securo-finance/securo) mostrou uma implementação open-source interessante de um gerenciador financeiro self-hosted, com integração bancária e uma infraestrutura baseada em FastAPI, React/Vite, PostgreSQL, Redis/Celery e Docker Compose.

A intenção **não é copiar o Securo**, mas estudar e adaptar as ideias que fazem sentido para a arquitetura da Ayla.

A Moneta deverá continuar útil mesmo sem IA:

```text
Pluggy
   ↓
Moneta
   ↓
Banco de dados
   ↓
Interface / relatórios
```

## 1.7 Athena

A **Athena** é a camada de conhecimento pessoal da Ayla.

- Integração com Obsidian
- Arquivos Markdown
- RPG
- Journaling
- Projetos pessoais
- Conhecimento
- Organização intelectual
- Documentos e referências

## 1.8 Odin

O **Odin** será responsável por pesquisa e coleta de informações externas.

- Busca profunda na internet
- Coleta de dados
- Estruturação de resultados
- Pesquisa
- Integração posterior com a Ayla

---

# 2. Como as partes se relacionam

Os órgãos possuem responsabilidades próprias, mas compartilham uma infraestrutura comum.

```text
                         AYLA
                           │
          ┌────────────────┴────────────────┐
          │                                 │
     ÓRGÃOS / SERVIÇOS                CAMADA DE IA
          │                                 │
   ┌──────┼──────┬──────┐             ┌────┴─────┐
   │      │      │      │             │          │
 Moneta Athena Pulse Arcade          Memória    LLM
   │      │      │      │             RAG       STT/TTS
   │      │      │      │             Tools     Vision
   └──────┴──────┴──────┘                 │
                 │                         │
                 └──────────┬──────────────┘
                            │
                     DADOS / STORAGE
```

A diferença fundamental é que **a camada de IA não possui os órgãos. Ela os utiliza.**

Isso permite que a mesma Moneta, por exemplo, seja usada pela interface gráfica, por automações e pela IA.

---

# 3. Banco de dados compartilhado

Um dos objetivos da arquitetura é possuir uma **base de dados unificada da Ayla**, sem transformar todos os módulos em um único domínio.

Cada órgão mantém seu próprio domínio, enquanto a infraestrutura de dados pode ser compartilhada quando fizer sentido.

```text
                     AYLA DATABASE
                          │
         ┌────────────────┼────────────────┐
         │                │                │
      Moneta           Athena            Pulse
         │                │                │
    transações         notas           músicas
    contas             projetos       artistas
    investimentos      RPG             histórico
         │                │                │
         └────────────────┼────────────────┘
                          │
                     IA / MEMÓRIA
```

Isso permite que a Ayla faça conexões como:

> "Você gastou mais com jogos neste mês e também passou mais horas jogando."

sem que Moneta ou Arcade precisem conhecer a IA.

---

# 4. Arquitetura

A arquitetura é pensada em quatro grandes grupos:

- **Aplicação:** interface, configurações, autenticação e permissões.
- **Órgãos e serviços:** módulos responsáveis pelos diferentes domínios.
- **Inteligência:** memória, RAG, classificação, LLM, STT, TTS, visão e orquestração.
- **Infraestrutura:** banco, cache, filas, armazenamento e modelos.

```text
┌──────────────────────────────────────────────────────────┐
│                       AYLA UI                            │
│             Desktop / Web / futuramente móvel           │
└──────────────────────────┬───────────────────────────────┘
                           │
┌──────────────────────────▼───────────────────────────────┐
│                       CORE                               │
│     configurações · eventos · permissões · serviços     │
└──────────────┬─────────────────────────────┬─────────────┘
               │                             │
       ┌───────▼────────┐           ┌────────▼────────┐
       │ ÓRGÃOS         │           │ INTELIGÊNCIA    │
       │ Moneta         │           │ Memória         │
       │ Athena         │           │ RAG             │
       │ Pulse          │           │ Classificação   │
       │ Arcade         │           │ LLM             │
       │ Hand           │           │ STT / TTS       │
       │ Hermes         │           │ Vision          │
       │ Iris           │           │ Tools           │
       │ Odin           │           │                 │
       └───────┬────────┘           └────────┬────────┘
               │                             │
               └──────────────┬──────────────┘
                              │
                    ┌─────────▼─────────┐
                    │ STORAGE / INFRA   │
                    │ PostgreSQL/SQLite │
                    │ Qdrant / Redis    │
                    │ arquivos / cache  │
                    └───────────────────┘
```

---

# 5. Inteligência Artificial

A IA continua sendo uma das partes mais importantes do projeto, mas é tratada explicitamente como uma **camada cognitiva**.

Ela é responsável por tarefas como:

- entendimento de linguagem;
- classificação de intenção;
- memória;
- RAG;
- raciocínio;
- seleção de ferramentas;
- orquestração entre órgãos;
- respostas por voz;
- STT / TTS;
- análise multimodal;
- explicação de resultados.

A aplicação não deve depender de um LLM para executar operações básicas.

## 5.1 Memória

A arquitetura planejada possui quatro camadas:

1. **Intent**
2. **Memória curta**
3. **Memória longa**
4. **Contexto externo**

A busca semântica deve utilizar **Qdrant**, com embeddings, cache semântico e classificação em paralelo sempre que possível.

---

# 6. Performance da IA

A experiência com modelos locais mostrou que simplesmente colocar um LLM atrás de uma API não é suficiente.

A Ayla deve tratar **latência e consumo de recursos como requisitos de arquitetura**.

Um projeto especialmente interessante para estudar é o [VoiceAssistant, de diegormirhan](https://github.com/diegormirhan/voice-assistant).

O projeto é um assistente de voz local que utiliza `whisper.cpp` e `llama.cpp` com Vulkan, streaming, VAD, `asyncio`, TTS por streaming, barge-in e uma arquitetura que separa a interface do pipeline. O projeto estabelece como meta menos de 2 segundos entre o fim da fala e a primeira sílaba da resposta.

### O que queremos trazer para a Ayla

Não a implementação inteira, mas os princípios:

- GPU-first quando apropriado
- Streaming de tokens
- Pipeline assíncrono
- Processos/serviços separados para gargalos
- Lazy loading
- Modelos descarregáveis
- Controle de VRAM/RAM
- VAD e interrupção de fala
- Métricas de latência por etapa
- Orçamento explícito de performance
- Não bloquear a aplicação principal por causa do LLM

A Ayla deve conseguir funcionar mesmo quando o modelo estiver desligado, carregando a camada cognitiva apenas quando necessária.

---

# 7. Docker e infraestrutura

A experiência com o Securo mostrou que **Docker Compose é uma opção interessante para organizar a infraestrutura da Ayla**.

Uma possível composição:

```text
docker compose
├── ayla-backend
├── ayla-frontend
├── postgres
├── redis
├── qdrant
└── workers / serviços auxiliares
```

Isso não significa que tudo precise rodar em containers.

Componentes que dependam fortemente de GPU, áudio, hardware ou integração profunda com o sistema operacional podem continuar como processos locais quando isso for mais adequado.

O objetivo é tornar a infraestrutura:

- reproduzível;
- fácil de instalar;
- fácil de atualizar;
- fácil de fazer backup;
- previsível durante desenvolvimento.

---

# 8. Biblioteca

A **Biblioteca** não é um órgão individual.

Ela é uma camada que emerge da integração dos diferentes módulos culturais da Ayla.

Pode reunir:

- músicas;
- jogos;
- filmes e séries;
- livros e mangás;
- outras mídias.

Ela permite criar:

- histórico cultural;
- relações entre diferentes tipos de conteúdo;
- retrospectivas;
- descoberta de padrões;
- registros da identidade pessoal ao longo do tempo.

---

# 9. Retrospectivas

Conforme os órgãos acumularem dados, a Ayla deverá conseguir produzir retrospectivas:

- mensais;
- anuais;
- culturais;
- financeiras;
- técnicas;
- de projetos;
- de diferentes fases da vida.

A IA pode ajudar a transformar os dados em narrativas e análises, mas os dados fundamentais devem vir dos órgãos e dos registros reais.

---

# 10. Futuro

## Audiovisual

- Streaming
- Filmes
- Séries
- Histórico de consumo
- Integração com a Biblioteca

## GitHub

- Commits
- Projetos
- Linguagens
- Atividade
- Retrospectivas técnicas

---

# 11. Referências e agradecimentos

A Ayla é um projeto independente, mas alguns projetos open-source têm sido especialmente importantes como referência.

### [Securo](https://github.com/securo-finance/securo)

Projeto open-source e self-hosted de gerenciamento financeiro.

A principal inspiração está na infraestrutura financeira e na experiência de desenvolvimento, especialmente:

- Docker/Compose
- integração com Pluggy
- FastAPI
- React/Vite
- PostgreSQL
- Redis/Celery
- separação de serviços
- possibilidade de recursos de IA serem opcionais

**Agradecimento:** obrigado ao pessoal do Securo por disponibilizar um projeto tão útil como referência para a comunidade open-source.

### [VoiceAssistant — diegormirhan](https://github.com/diegormirhan/voice-assistant)

Projeto de assistente de voz local com foco em baixa latência, execução no dispositivo, processamento assíncrono, Vulkan, `whisper.cpp` e `llama.cpp`.

A principal inspiração para a Ayla está na engenharia do pipeline de IA e na preocupação explícita com performance.

**Agradecimento:** obrigado ao Diego por disponibilizar o projeto e documentar uma abordagem tão prática para construir uma pipeline local de voz e IA.

> **As referências acima são utilizadas como inspiração técnica e arquitetural. A Ayla não pretende reproduzir esses projetos nem reivindica autoria sobre suas implementações.**

---

# 12. Estado atual

A infraestrutura inicial da Ayla já possui:

- Backend FastAPI
- Endpoint `/chat`
- Streaming local via Ollama
- Memória curta
- SQLite
- Interface desktop baseada em React + Vite + Tauri
- Planejamento de Qdrant para RAG
- Separação conceitual entre órgãos e Core
- Primeiros testes com provedores cloud

Os próximos passos devem priorizar **fundação arquitetural e performance**, antes de adicionar dezenas de novas capacidades.

Consulte [`PIPELINE.md`](PIPELINE.md) para o planejamento de implementação.

---

# 13. Licença

A licença da Ayla ainda deve ser definida.

---

## Regra de ouro

> **A Ayla deve ser útil sem ser inteligente.**
>
> **E deve ser muito mais poderosa quando fica inteligente.**
