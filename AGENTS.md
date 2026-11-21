# Guia dos agentes — Ayla

## Autoridade e documentos

- Este `AGENTS.md` orienta todos os agentes de IA, incluindo o Codex.
- `pipeline.md` registra o que vamos fazer, estamos fazendo e já validamos, com prioridades, dependências e pendências.
- `README.md` é público. Preserve seu conteúdo atual; só o altere mediante pedido explícito do usuário.
- Leia este guia e a pipeline antes de implementar. Confirme o estado do código e do Git; documentação e memória não provam funcionamento.

## Decisões com o usuário

- Sempre explicite e pergunte antes de tomar uma decisão de projeto: arquitetura, escopo, prioridade, experiência do produto, política de dados/privacidade, infraestrutura, dependências externas ou distribuição.
- Apresente o problema, as opções, sua recomendação e o impacto de cada escolha em linguagem direta. Aguarde a resposta antes de implementar o que depender dela.
- Continue correções e verificações independentes enquanto aguarda. Não transforme silêncio ou passagem de tempo em aprovação.
- Escolhas rotineiras de implementação que preservam decisões existentes não exigem nova consulta. Na dúvida sobre impacto no destino do projeto, pergunte.
- Registre decisões confirmadas na pipeline; não apresente uma proposta como decisão tomada.

## Direção

- Ayla é offline-first; IA é opcional. Serviços determinísticos devem funcionar sem modelos.
- UI, automações e IA devem usar os mesmos serviços quando aplicável.
- Moneta e outros projetos separados entram por contratos explícitos; não presumir API, estado ou propriedade dos dados.
- Órgãos abertos dentro da Ayla seguem os fundos, superfícies, texto e fontes da Ayla, por coerência de design. A camada de compatibilidade de cada órgão mapeia as variáveis dele para as da Ayla (`organs/<órgão>/tema.css`); o destaque e os gráficos podem continuar os do órgão.
- Não adicionar infraestrutura ou abstrações sem necessidade da entrega em andamento.
- Desempenho é requisito de arquitetura. Rust é uma das linguagens principais; núcleo em Rust e Python apenas em adaptadores auxiliares necessários, com medições de latência, CPU, RAM e uso de GPU quando aplicável.
- Experiência principal: abrir diretamente na conversa, sem aba Início nem criação manual obrigatória de conversa. Perfil Ayla automático é o padrão. Preferência de uso e personalidade ficam visíveis em Configurações; modelos, componentes e limites técnicos ficam em Opções avançadas. Não exigir conhecimento técnico no fluxo principal.
- Personalidade é uma configuração simples e independente de modelos, editável e restaurável ao padrão; jamais altera controles de privacidade. Para cloud, incluir a personalidade na revisão explícita do payload.
- Linux é o primeiro alvo; portar para Windows depois. Minimizar atrito técnico para o usuário final.
- Não fixar um modelo único. A direção é selecionar modelos por pedido, com tipos opcionais escolhidos na instalação e perfis leve, moderado e máximo para diferentes hardwares. Importação livre desde P1; shortlist é sugestão, não restrição. Perfis são orçamentos ajustáveis de RAM (25%, 50%, 75%), com reserva para o sistema.
- Runtime local gerenciado: llama.cpp para texto/raciocínio/visão/embeddings, whisper.cpp para transcrição. A Ayla responde em texto (sem síntese de voz) e pode avisar por notificações do sistema. Modelos são sempre opcionais. Instalação Linux primeiro; Windows depois.
- Cloud: modos configuráveis; dados sensíveis ou duvidosos ficam locais em todos os modos. Envio exige revisão do conteúdo público e consentimento aplicável; sugestões da IA não autorizam envio. Referências (memórias, arquivos ou conversas anteriores) só acompanham um envio quando forem relevantes ao pedido, não tiverem conteúdo sensível ou duvidoso e aparecerem listadas na revisão do envio; nunca enviar histórico ou órgãos em bloco.
- Não enviar dados, disparar chamadas cloud pagas ou conectar contas reais durante validações sem autorização específica.

## Execução e preservação

- Trabalhe no escopo autorizado e preserve alterações preexistentes.
- Não criar branches adicionais, commits, pushes, merges, limpezas ou publicações sem autorização explícita. Não implementar diretamente em `main` ou `development`.
- Preserve bancos e arquivos do usuário; faça backup antes de migrações. Nunca exponha conteúdo pessoal, tokens ou secrets nos logs, testes e documentação.
- Use dados temporários e provedores simulados nos testes; não importe a aplicação contra o banco real para testar.
- Não delegar a outros agentes sem solicitação do usuário ou instrução aplicável explícita.

## Validação e andamento

- Execute verificações proporcionais: testes focados para mudanças pontuais; backend, build da interface e verificações desktop para alterações que atravessam essas camadas.
- Testes devem cobrir contratos e regressões relevantes, não apenas reproduzir a implementação.
- Separe testes automatizados, inspeção estática, validação da interface, inicialização desktop e uso de provedores reais.
- Só marque um item como concluído quando seu aceite tiver evidência. Registre bloqueios e verificações pendentes sem declarar a etapa inteira pronta.
- Mantenha a pipeline resumida e atualizada. Ao concluir, informe mudanças, validações e limitações materiais de forma concisa.
