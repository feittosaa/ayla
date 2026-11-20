# Benchmark local de texto da Ayla

A rodada de 20/11/2026 compara os quatro GGUF de texto que já estavam no cache de desenvolvimento. O objetivo é verificar o custo da primeira resposta, o ganho de continuar uma conversa e o cumprimento de requisitos em português. A referência de Akita compara quantizações do Qwen3.8 Flash-Next com outros motores e hardware. Aqui comparamos os modelos candidatos da Ayla sob o mesmo llama.cpp. [Artigo de referência](https://akitaonrails.com/2026/10/27/llm-benchmark-melhores-llms-open-source-hardware-caseiro/)

## Resultados de 20/11/2026

Fedora 43, Ryzen 7 7800X3D, 30,5 GiB de RAM visível ao Linux, llama.cpp b12531 (`e4b620af1`). O runtime Vulkan não detectou a RX 9060 XT nesta sessão. Todas as medições abaixo foram feitas em CPU, com os pesos já disponíveis em disco. RSS é a memória residente do worker, incluindo as páginas residentes do arquivo do modelo.

| Modelo e quantização | Carga | 1º token, 4.096 | 1º token, 7.168 | 1º token, continuação | Leitura a 7.168 (tok/s) | Geração a 7.168 (tok/s) | Pico de RSS |
|---|---:|---:|---:|---:|---:|---:|---:|
| Qwen3.5 0.8B Q4_0 | 0,81 s | 5,48 s | 10,91 s | 0,10 s | 657,2 | 49,5 | 1,39 GiB |
| Qwen3.5 4B Q4_K_M | 3,12 s | 35,91 s | 61,58 s | 0,38 s | 116,4 | 9,8 | 5,75 GiB |
| Gemma 4 E4B QAT Q4_0 | 3,63 s | 19,47 s | 82,26 s | 0,33 s | 87,1 | 4,6 | 7,52 GiB |
| Qwen3.5 9B Q4_K_M | 4,83 s | 45,43 s | 84,86 s | 0,59 s | 84,5 | 5,0 | 9,28 GiB |

Nos quatro modelos, a continuação reaproveitou 7.423 dos 7.440 tokens enviados. O motor precisou processar 17 para começar a responder. O resultado depende de manter o mesmo worker e o mesmo prefixo de tokens. [Dados de velocidade e JSON](2026-11-20-cpu.jsonl)

| Modelo | JSON, turnos corretos | SQL inicial, bases corretas | SQL após a mudança, bases corretas | Tempo das duas respostas SQL |
|---|---:|---:|---:|---:|
| Qwen3.5 0.8B | 0/4 | 0/4 | 0/4 | 7,0 s |
| Qwen3.5 4B | 2/4 | 4/4 | 3/4 | 12,6 s |
| Gemma 4 E4B | 0/4 | 4/4 | 2/4 | 14,6 s |
| Qwen3.5 9B | 4/4 | 3/4 | 3/4 | 25,4 s |

- **Qwen 0.8B** perdeu os prefixos dos IDs e omitiu o agrupamento por conta no SQL. No segundo pedido de SQL, repetiu a consulta até atingir o limite de saída.
- **Qwen 4B** acertou os dois pedidos iniciais em JSON. Após as correções, trocou os IDs e devolveu 2.995 centavos quando o total era 2.495. No segundo SQL, uma conta com movimentos pagos e pendentes apareceu duas vezes.
- **Gemma E4B** acertou as duas listas de tarefas, mas envolveu todas as respostas em blocos Markdown. Mesmo removendo esses blocos, as duas somas em centavos estão erradas. O segundo SQL multiplicou valores ao juntar vários movimentos da mesma conta.
- **Qwen 9B** preservou os IDs e recalculou os centavos nos dois turnos. No primeiro SQL, descartou contas com soma negativa ou zero, apesar de haver pagamentos. No segundo, duplicou uma conta que tinha movimentos pagos e pendentes.

O 9B merece nova avaliação para extração estruturada. O 4B acertou mais bases de SQL usando menos RAM. Nenhum resolveu a mudança de requisito em todas as bases. Para avaliar o perfil principal da Ayla, a próxima comparação é repetir texto e SQL na RX 9060 XT com Vulkan, com mais de uma passagem por modelo, e confrontar os resultados com os testes anteriores de conversa e visão. [Consultas produzidas e verificações por base](2026-11-20-cpu-sql.jsonl)

## Protocolo

- Um worker por modelo, uma requisição de cada vez, contexto de 8.192 tokens, sete threads, sem warmup e sem projetor de visão. A carga mede o intervalo até o worker responder ao health check. “Frio” na tabela significa que o worker já carregou o modelo e precisa ler o prompt inteiro. O cache de arquivos do Linux é compartilhado entre processos.
- O teste de velocidade tokeniza um texto sintético em português e envia os primeiros 4.096 ou 7.168 IDs ao endpoint de completions. Cada chamada fria desativa o reuso do prompt e gera exatamente 256 tokens. O terceiro pedido contém o prefixo de 7.168 tokens, a saída real anterior e 16 tokens adicionais, agora com cache habilitado. O resultado registra quantos tokens o servidor realmente processou.
- TTFT é o intervalo entre iniciar a requisição e receber o primeiro evento com token no stream local. Prefill e geração usam as contagens e os tempos informados pelo motor. O pico de RSS vem do processo do worker. O teste de velocidade força o comprimento da saída e mede o motor diretamente.
- A qualidade usa a API de chat, o template de cada modelo, raciocínio desativado, temperatura zero e até 256 tokens por resposta. Duas conversas sintéticas têm dois turnos cada. Uma exige preservar IDs de tarefas e aplicar uma correção. A outra exige calcular em centavos e rever um lançamento. Cada turno vale um acerto se o conteúdo inteiro for o JSON esperado. A ordem das chaves é livre. O segundo turno recebe a resposta real do modelo ao primeiro.
- O canário de programação pede uma consulta SQLite e depois muda um requisito. Cada versão é executada em quatro bancos sintéticos independentes, incluindo contas sem pagamentos, valores negativos e um banco vazio. O modelo só recebe o esquema e os requisitos. A correção compara as duas colunas, os valores e a ordem das linhas. O banco em memória aceita leitura e agregações, com limite de tempo.
- Uma passagem por modelo e cenário. O artigo mede 42.400 tokens no cenário longo. Usamos 7.168 para acomodar a resposta e a continuação na janela padrão da Ayla. Os testes de qualidade usam histórico curto. Recuperação de fatos em contexto longo, energia, temperatura, visão e latência da interface Tauri ficaram fora desta rodada.

## Reproduzir no Fedora

Execute na raiz do repositório. Os modelos e o runtime já devem estar instalados. Cada arquivo de saída é criado uma única vez.

```bash
cache=/var/tmp/ayla-benchmark
motor="$HOME/.local/share/dev.feit.ayla/runtimes/llama-vulkan/llama-b12531/llama-server"
modelos=(
  "$cache/Qwen3.5-0.8B-Q4_0.gguf"
  "$cache/gemma-4-E4B_q4_0-it.gguf"
  "$cache/Qwen3.5-4B-Q4_K_M.gguf"
  "$cache/Qwen3.5-9B-Q4_K_M.gguf"
)
LD_LIBRARY_PATH="$(dirname "$motor")" "$motor" --list-devices
```

Para medir a RX 9060 XT, use o identificador que a linha dessa placa informar. O exemplo reserva 8 GiB para o sistema. Execute as duas suítes em sequência.

```bash
(
  set -e
  saida=$(mktemp -d)
  echo "Resultados em $saida"
  for suite in text sql; do
    python3 benchmarks/local_text.py --server "$motor" --device Vulkan0 --reserve-mib 8192 \
      --threads 7 --suite "$suite" --out "$saida/$suite.jsonl" "${modelos[@]}"
  done
)
```

Para repetir a medição em CPU, aponte `motor` para o `llama-server` do pacote `llama-cpu` b12531 e omita `--device Vulkan0 --reserve-mib 8192`. O arquivo JSONL contém uma linha por modelo concluído.
