import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { streamChat, type Connection } from "../services/Api";
import "./Chat.css";

type Message = { role: "user" | "assistant"; content: string };

export default function Chat() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [input, setInput] = useState("");
  const [loading, setLoading] = useState(false);
  const [connection, setConnection] = useState<Connection | null>(null);
  const [status, setStatus] = useState("Iniciando a Ayla…");
  const [error, setError] = useState("");
  const abortRef = useRef<AbortController | null>(null);
  const busyRef = useRef(false);

  useEffect(() => {
    const controller = new AbortController();
    async function connect() {
      if (!isTauri()) {
        setStatus("Abra a Ayla pelo aplicativo desktop.");
        return;
      }
      try {
        const info = await invoke<Connection>("backend_connection");
        for (let attempt = 0; attempt < 40; attempt++) {
          if (controller.signal.aborted) return;
          try {
            const response = await fetch(`${info.url}/health`, {
              headers: { "X-Ayla-Token": info.token },
              signal: AbortSignal.any([controller.signal, AbortSignal.timeout(1000)]),
            });
            if (response.ok) {
              const health = await response.json();
              setConnection(info);
              setStatus(health.ai_configured ? "Ayla pronta." : "Ayla pronta. IA ainda não configurada.");
              return;
            }
          } catch {
            if (controller.signal.aborted) return;
          }
          await new Promise((resolve) => setTimeout(resolve, 250));
        }
        throw new Error("Não foi possível iniciar a Ayla. Reabra o aplicativo.");
      } catch {
        if (!controller.signal.aborted) setStatus("Não foi possível iniciar a Ayla. Reabra o aplicativo.");
      }
    }
    void connect();
    return () => { controller.abort(); abortRef.current?.abort(); };
  }, []);

  async function handleSend() {
    if (!input.trim() || busyRef.current || !connection) return;
    busyRef.current = true;
    const text = input.trim();
    setMessages((prev) => [...prev, { role: "user", content: text }, { role: "assistant", content: "" }]);
    setInput("");
    setError("");
    setLoading(true);
    const controller = new AbortController();
    abortRef.current = controller;
    try {
      await streamChat(connection, text, (token) => {
        setMessages((prev) => {
          const updated = [...prev];
          const last = updated[updated.length - 1];
          updated[updated.length - 1] = { ...last, content: last.content + token };
          return updated;
        });
      }, controller.signal);
    } catch (err) {
      setError(controller.signal.aborted ? "Resposta interrompida." : err instanceof Error ? err.message : "Não foi possível concluir a resposta.");
      // Preservar texto já recebido, inclusive quando o usuário interrompe.
    } finally {
      setMessages((prev) => prev.filter((message) => message.content !== ""));
      setLoading(false);
      abortRef.current = null;
      busyRef.current = false;
    }
  }

  return (
    <div className="chat-container">
      <p className="connection-status" role="status">{status}</p>
      <div className="messages" aria-live="polite">
        {messages.map((m, i) => <div key={i} className={`message ${m.role}`}>{m.content}</div>)}
        {loading && <div className="message assistant">Respondendo…</div>}
      </div>
      {error && <p className="chat-error" role="alert">{error}</p>}
      <form className="input-bar" onSubmit={(event) => { event.preventDefault(); void handleSend(); }}>
        <input aria-label="Mensagem" value={input} onChange={(e) => setInput(e.target.value)} placeholder="Fale com a Ayla…" disabled={loading || !connection} />
        {loading
          ? <button type="button" onClick={() => abortRef.current?.abort()}>Interromper</button>
          : <button type="submit" disabled={!connection || !input.trim()}>Enviar</button>}
      </form>
    </div>
  );
}
