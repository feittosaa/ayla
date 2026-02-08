import "./Chat.css";

export default function Chat() {
  return (
    <div className="chat-container">
      <div className="messages">
        <div className="message ayla">
          <span>Oi… ☀️ eu sou a Ayla.</span>
        </div>
      </div>

      <div className="input-area">
        <input placeholder="Fale comigo…" />
        <button>➤</button>
      </div>
    </div>
  );
}
