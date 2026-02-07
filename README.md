# 🌤️ Projeto Ayla

> Uma IA pessoal, local-first, construída com cuidado, identidade e memória.

---

## ✨ O que é o Projeto Ayla?

O **Projeto Ayla** é uma iniciativa pessoal para criar uma **IA assistente própria**, executável no computador, com foco em:

* 🧠 **Consciência de contexto** (memória)
* 🔐 **Privacidade** (local-first)
* 🏗️ **Arquitetura sólida** (MVC)
* 🌱 **Evolução incremental** (sem pressa, sem atalhos)

A Ayla não é apenas um chatbot. Ela é pensada como um **sistema vivo**, que cresce junto com seu criador, acumulando conhecimento, preferências e decisões ao longo do tempo.

---

## 🎯 Objetivo inicial (MVP)

A primeira versão da Ayla deve ser capaz de:

* Abrir como um **aplicativo desktop**
* Receber perguntas em texto
* Responder de forma coerente e contextual
* Manter memória de curto prazo
* Ter uma **identidade clara e consistente**

Nada além disso é obrigatório no começo.

> **Princípio fundamental:**
> Antes de ser poderosa, a Ayla precisa ser **bem estruturada**.

---

## 🧭 Princípios do Projeto

1. **Local-first**
   Sempre que possível, dados e processamento permanecem na máquina do usuário.

2. **Privacidade como regra**
   Nenhuma informação pessoal sai do sistema sem decisão explícita.

3. **Arquitetura antes de features**
   Código organizado é prioridade sobre funcionalidades chamativas.

4. **Evolução consciente**
   Cada nova capacidade deve ter um propósito claro.

5. **Identidade > Treinamento**
   A personalidade da Ayla é definida por regras, memória e contexto — não por re-treinamento pesado de modelos.

---

## 🏛️ Arquitetura Geral

O projeto segue uma separação clara entre **mente** e **corpo**:

* **Backend (Python)** → lógica, IA, memória, regras
* **Frontend (JavaScript)** → interface, interação, visual

A comunicação entre eles acontece via **API local**.

Estrutura base:

```
project-ayla/
├── backend/    # Cérebro (Python)
├── frontend/   # Corpo (JavaScript)
├── docs/       # Visão, decisões e registros
└── README.md
```

---

## 🧠 IA e Modelos de Linguagem

O Projeto Ayla **não depende de um único modelo**.

A arquitetura permite:

* Uso de **modelos locais** (ex: LLaMA, Mistral)
* Uso de **modelos em nuvem**, quando desejado
* Estratégia **híbrida**, onde a Ayla decide qual usar

> O modelo é uma ferramenta.
> A Ayla é o sistema que decide como usá-lo.

---

## 🎭 Identidade da Ayla

A personalidade da Ayla é definida de forma **explícita e versionada**.

Ela inclui:

* Forma de falar
* Tom emocional
* Limites éticos
* Estilo de interação

Essa identidade vive no código e evolui junto com o projeto.

---

## 🧱 Estado Atual do Projeto

📍 **Fase:** Fundação

* [x] Definição de visão
* [x] Criação do repositório
* [ ] Estrutura base do backend
* [ ] API local funcional
* [ ] Interface inicial

---

## 🛣️ Roadmap (alto nível)

1. Fundação (estrutura + visão)
2. Backend funcional (API + respostas mockadas)
3. Interface desktop
4. Integração com modelo de linguagem
5. Memória persistente
6. Estratégia híbrida (local + nuvem)
7. Voz, plugins e automações

---

## 🧡 Nota do Criador

Este projeto não é sobre criar a IA mais poderosa.

É sobre criar **a IA certa**.

Uma que respeita limites, cresce com o tempo e reflete quem a constrói.

---

> "Algumas coisas não nascem prontas.
> Elas nascem bem cuidadas."