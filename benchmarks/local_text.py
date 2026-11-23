"""Benchmark offline de texto para os GGUF da Ayla, usando llama-server b12531."""
import argparse
import json
import os
import re
from pathlib import Path
import secrets
import sqlite3
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from contextlib import contextmanager

CONTEXT = 8192
OUTPUT = 256
CASES = [
    [
        ("Controle de tarefas. A12 está aberta, B34 concluída e C56 aberta. "
         "Retorne um objeto JSON com abertas (lista de IDs na ordem citada) e total (quantidade de abertas).",
         {"abertas": ["A12", "C56"], "total": 2}),
        ("Correção confirmada: C56 foi concluída e B34 foi reaberta. "
         "A ideia de fechar A12 foi rejeitada. Atualize o mesmo JSON.", {"abertas": ["A12", "B34"], "total": 2}),
    ],
    [
        ("Relatório em centavos. Há duas compras confirmadas de 1290 e 750, e uma compra pendente de 500. "
         "Informe soma, quantidade e moeda em um objeto JSON, apenas para compras confirmadas. A moeda é BRL.",
         {"soma": 2040, "quantidade": 2, "moeda": "BRL"}),
        ("Os 750 estavam incorretos: substitua por 705. A compra de 500 acaba de ser confirmada. "
         "Recalcule no mesmo formato.", {"soma": 2495, "quantidade": 3, "moeda": "BRL"}),
    ],
]


# Conexões do benchmark seguem diretamente para o worker local.
LOCAL = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def post(base, key, path, payload):
    data = json.dumps(payload).encode()
    request = urllib.request.Request(base + path, data, headers={
        "Content-Type": "application/json", "Authorization": "Bearer " + key,
    })
    return LOCAL.open(request, timeout=900)


@contextmanager
def server(binary, model, threads, device, reserve_mib, extra=()):
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    key = secrets.token_hex(32)
    env = dict(os.environ, LLAMA_API_KEY=key)
    env["LD_LIBRARY_PATH"] = str(binary.parent) + os.pathsep + env.get("LD_LIBRARY_PATH", "")
    flags = ["--offline", "--no-webui", "--no-warmup", "--no-slots", "--parallel", "1",
             "--host", "127.0.0.1", "--port", str(port), "-c", str(CONTEXT),
             "-t", str(threads), "-tb", str(threads)]
    flags += list(extra)
    if device:
        flags += ["--device", device, "-ngl", "auto", "--split-mode", "none", "--fit-target", str(reserve_mib)]
    else:
        flags += ["-ngl", "0"]
    started = time.monotonic()
    with tempfile.TemporaryFile() as log, subprocess.Popen([binary, "-m", model, *flags], env=env,
            stdout=log, stderr=subprocess.STDOUT) as proc:
        try:
            while True:
                if proc.poll() is not None or time.monotonic() - started > 180:
                    log.seek(0)
                    detail = log.read().decode(errors="replace")[-1600:].replace(key, "[oculto]")
                    raise RuntimeError(f"Falha ao iniciar {model.name} (status {proc.poll()}): {detail}")
                try:
                    with LOCAL.open(base + "/health", timeout=2) as response:
                        if response.status == 200:
                            break
                except (urllib.error.URLError, TimeoutError):
                    pass
                time.sleep(0.1)
            yield base, key, proc.pid, round(time.monotonic() - started, 3)
        finally:
            proc.terminate()
            try:
                proc.wait(timeout=8)
            except subprocess.TimeoutExpired:
                proc.kill()


def completion(base, key, prompt, reuse):
    payload = dict(prompt=prompt, n_predict=OUTPUT, stream=True, ignore_eos=True, seed=42,
                   temperature=0, cache_prompt=reuse, return_tokens=True, id_slot=0)
    started = time.monotonic()
    first = None
    tokens = []
    timings = None
    with post(base, key, "/completion", payload) as response:
        for line in response:
            if not line.startswith(b"data:") or line.strip().endswith(b"[DONE]"):
                continue
            event = json.loads(line[5:])
            if "error" in event:
                raise RuntimeError(event["error"])
            if event.get("tokens"):
                if first is None:
                    first = time.monotonic()
                tokens.extend(event["tokens"])
            if event.get("stop"):
                timings = event["timings"]
    if first is None or not timings or len(tokens) != OUTPUT or timings["predicted_n"] != OUTPUT:
        raise RuntimeError("Resposta incompleta ou incompatível com o runtime do benchmark")
    return tokens, dict(input_tokens=len(prompt), processed=timings["prompt_n"],
                        cache_reported=timings.get("cache_n"), output_tokens=len(tokens),
                        ttft_s=round(first - started, 3), wall_s=round(time.monotonic() - started, 3),
                        prefill_tps=round(timings["prompt_per_second"], 2),
                        decode_tps=round(timings["predicted_per_second"], 2))


def reply(base, key, history):
    started = time.monotonic()
    with post(base, key, "/v1/chat/completions", dict(messages=history, temperature=0, seed=42,
              max_tokens=OUTPUT, chat_template_kwargs={"enable_thinking": False})) as response:
        body = json.load(response)
    choice = body["choices"][0]
    stats = {"wall_s": round(time.monotonic() - started, 3), "output_tokens": body["usage"]["completion_tokens"]}
    return choice["message"]["content"] or "", choice["finish_reason"], stats


def quality(base, key):
    results = []
    for turns in CASES:
        history = [{"role": "system", "content": "Responda em português. Entregue apenas o JSON solicitado."}]
        for question, expected in turns:
            history.append({"role": "user", "content": question})
            answer, finish, stats = reply(base, key, history)
            try:
                value = json.loads(answer)
            except ValueError:
                value = None
            # JSON canônico distingue booleanos de inteiros. Ordem de chaves não altera a nota.
            passed = json.dumps(value, sort_keys=True) == json.dumps(expected, sort_keys=True)
            results.append(dict(answer=answer, passed=passed, finish=finish, **stats))
            history.append({"role": "assistant", "content": answer})
    return results


# Cada conjunto é um banco independente. Só os prompts chegam ao modelo.
SQL_FIXTURES = [
    [],
    [("B", "pago", 100), ("B", "pendente", 90), ("A", "pago", 200), ("A", "pago", 50)],
    [("A", "pendente", 300), ("B", "cancelado", 10)],
    [("C", "pago", -40), ("B", "pago", 10), ("A", "pago", -10), ("B", "pago", -10),
     ("A", "pago", -10), ("D", "pendente", 800), ("E", "cancelado", 1)],
]


def sql_passes(answer, rows, include_empty):
    sql = answer.strip()
    if sql.startswith(('```sql\n', '```sqlite\n', '```\n')) and sql.endswith('```'):
        sql = sql.split('\n', 1)[1][:-3].strip()
    expected = {}
    for account, state, amount in rows:
        if state == "pago":
            expected[account] = expected.get(account, 0) + amount
        elif include_empty:
            expected.setdefault(account, 0)
    db = sqlite3.connect(":memory:")
    try:
        db.execute("CREATE TABLE movimentos(conta TEXT, estado TEXT, centavos INTEGER)")
        db.executemany("INSERT INTO movimentos VALUES(?,?,?)", rows)
        db.execute("PRAGMA query_only=ON")
        def authorize(action, arg, name, database, source):
            allowed = action in (sqlite3.SQLITE_SELECT, sqlite3.SQLITE_READ) or (
                action == sqlite3.SQLITE_FUNCTION and name in ("sum", "coalesce", "ifnull", "total", "iif", "count"))
            return sqlite3.SQLITE_OK if allowed else sqlite3.SQLITE_DENY
        db.set_authorizer(authorize)
        deadline = time.monotonic() + 0.3
        db.set_progress_handler(lambda: int(time.monotonic() > deadline), 200)
        cur = db.execute(sql)
        return len(cur.description or ()) == 2 and cur.fetchmany(len(expected) + 1) == sorted(expected.items())
    except (sqlite3.Error, ValueError):
        return False
    finally:
        db.close()


def sql_canary(base, key):
    history = [{"role": "system", "content": "Responda somente com uma consulta SQLite de leitura."}]
    prompts = [
        "Há uma tabela movimentos(conta TEXT, estado TEXT, centavos INTEGER). Retorne duas colunas: conta e soma "
        "dos centavos pagos por conta (estado = 'pago'). Liste só contas com ao menos um item pago. "
        "Inclua valores negativos e ordene por conta crescente.",
        "Mudança de requisito: inclua também as contas que aparecem na tabela mas não têm nenhum item pago. "
        "Para essas, o total deve ser zero. Mantenha todos os demais requisitos.",
    ]
    results = []
    for index, prompt in enumerate(prompts):
        history.append({"role": "user", "content": prompt})
        answer, finish, stats = reply(base, key, history)
        checks = [sql_passes(answer, rows, include_empty=bool(index)) for rows in SQL_FIXTURES]
        results.append(dict(answer=answer, checks=checks, finish=finish, **stats))
        history.append({"role": "assistant", "content": answer})
    return results


def measure(args, model):
    with server(args.server, model, args.threads, args.device, args.reserve_mib, args.server_arg) as (base, key, pid, load):
        if args.suite == "sql":
            return dict(model=model.name, sql=sql_canary(base, key))
        text = "".join(f"Registro {i}: a equipe revisou o documento e confirmou a entrega.\n" for i in range(1000))
        with post(base, key, "/tokenize", {"content": text, "add_special": True}) as response:
            ids = json.load(response)["tokens"]
        if len(ids) < 7184:
            raise ValueError("O texto sintético não produziu tokens suficientes para a continuação")
        speeds = {}
        for name, count in [("cold_4k", 4096), ("cold_7k", 7168)]:
            generated, speeds[name] = completion(base, key, ids[:count], reuse=False)
            if speeds[name]["processed"] != count:
                raise RuntimeError("O servidor não processou a entrada fria completa")
        # Envia exatamente o prefixo anterior, a saída real e mais 16 tokens.
        _, speeds["append"] = completion(base, key, ids[:7168] + generated + ids[7168:7184], reuse=True)
        answers = quality(base, key)
        peak = next(line for line in Path(f"/proc/{pid}/status").read_text().splitlines() if line.startswith("VmHWM:"))
        return dict(model=model.name, bytes=model.stat().st_size, load_s=load, peak_rss_kib=int(peak.split()[1]),
                    speed=speeds, quality=answers)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", required=True, type=Path)
    parser.add_argument("--suite", choices=["text", "sql"], default="text")
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--threads", type=int, default=max(1, len(os.sched_getaffinity(0)) - 1))
    parser.add_argument("--device", help="Dispositivo informado por llama-server --list-devices, ex.: Vulkan0")
    parser.add_argument("--reserve-mib", type=int, default=8192, help="Reserva de memória no modo GPU")
    parser.add_argument("--server-arg", action="append", default=[],
                        help="Argumento extra do llama-server (repetível), ex.: --server-arg=-fa --server-arg=on")
    parser.add_argument("models", nargs="+", type=Path)
    args = parser.parse_args()
    args.server = args.server.resolve(strict=True)
    models = [path.resolve(strict=True) for path in args.models]
    cpu = next(line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines()
               if line.startswith("model name"))
    env = dict(os.environ, LD_LIBRARY_PATH=str(args.server.parent) + os.pathsep + os.getenv("LD_LIBRARY_PATH", ""))
    def runtime_info(flag):
        return subprocess.check_output([args.server, flag], env=env, stderr=subprocess.STDOUT, text=True)
    if args.threads <= 0 or args.reserve_mib < 0:
        parser.error("Use uma contagem de threads positiva e uma reserva de memória não negativa")
    device_info = None
    if args.device:
        devices = runtime_info("--list-devices")
        device_info = next((line.strip() for line in devices.splitlines()
                            if line.strip().startswith(args.device + ":")), None)
        if not device_info:
            parser.error(f"Dispositivo {args.device} ausente no runtime: {devices.strip()}")
    version = re.search(r"^version: (.+)$", runtime_info("--version"), re.MULTILINE)
    report = dict(cpu=cpu, gpu_requested=bool(args.device), device=device_info, threads=args.threads,
                  reserve_mib=args.reserve_mib if args.device else None, server_args=args.server_arg, context=CONTEXT, seed=42,
                  runtime=version.group(1) if version else "unknown")
    # Uma linha por modelo permite acompanhar a rodada e preservar resultados parciais.
    with args.out.open("x") as output:
        for model in models:
            print(f"{model.name}: iniciando {args.suite}", flush=True)
            result = measure(args, model)
            json.dump({**report, **result}, output, ensure_ascii=False)
            output.write("\n")
            output.flush()
            print(f"{model.name}: {args.suite} concluído", flush=True)


if __name__ == "__main__":
    main()
