export type Connection = { url: string; token: string };

// SSE interno: dados JSON preservam espaços, novas linhas e Unicode.
export async function readChatStream(body: ReadableStream<Uint8Array>, onToken: (token: string) => void) {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let complete = false;

  function consume(frame: string) {
    let event = "message";
    const data: string[] = [];
    for (const line of frame.split(/\r?\n/)) {
      if (line.startsWith("event:")) event = line.slice(6).trim();
      if (line.startsWith("data:")) data.push(line.slice(5).replace(/^ /, ""));
    }
    if (!data.length) return;
    const payload = JSON.parse(data.join("\n"));
    if (event === "error") throw new Error(payload.message || "Resposta interrompida.");
    if (event === "done") complete = true;
    if (event === "message" && typeof payload.token === "string") onToken(payload.token);
  }

  try {
    while (!complete) {
      const { value, done } = await reader.read();
      buffer += decoder.decode(value, { stream: !done });
      let boundary: RegExpExecArray | null;
      while ((boundary = /\r?\n\r?\n/.exec(buffer)) !== null) {
        consume(buffer.slice(0, boundary.index));
        buffer = buffer.slice(boundary.index + boundary[0].length);
        if (complete) break;
      }
      if (done) break;
    }
    if (!complete) throw new Error("A conexão terminou antes de concluir a resposta.");
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}

export async function streamChat(
  connection: Connection,
  message: string,
  onToken: (token: string) => void,
  signal?: AbortSignal,
) {
  const response = await fetch(`${connection.url}/chat`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Accept: "text/event-stream",
      "X-Ayla-Token": connection.token,
    },
    body: JSON.stringify({ message }),
    signal,
  });
  if (!response.ok) {
    throw new Error(response.status === 401 ? "A conexão com a Ayla precisa ser reiniciada." : "Não foi possível enviar a mensagem.");
  }
  if (!response.body || !response.headers.get("content-type")?.includes("text/event-stream")) {
    throw new Error("A Ayla retornou uma resposta inválida.");
  }
  await readChatStream(response.body, onToken);
}
