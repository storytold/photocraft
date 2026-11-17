import { useEffect, useRef, useState } from "react";
import LogTimeline from "./LogTimeline";
import SettingsModal from "./SettingsModal";

const SUGGESTIONS = [
  "新建 1080x1920 画布，填充蓝色",
  "新建 1920x1080 画布，填充白色",
  "新建 800x800 画布",
];

function SendIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M4 11.5 20 4l-4.2 16-4.1-6.2L4 11.5Z" />
      <path d="m11.7 13.8 4.1-5.1" />
    </svg>
  );
}

function WarningIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M12 4 3.5 19h17L12 4Z" />
      <path d="M12 9v4.5M12 16.5v.5" />
    </svg>
  );
}

function Spinner({ small = false }) {
  return <span className={`spinner ${small ? "spinner-small" : ""}`} aria-hidden="true" />;
}

function Welcome({ text }) {
  return (
    <div className="msg msg-agent">
      <div className="avatar">AI</div>
      <div className="agent-content">
        <div className="agent-name">PhotoCraftAI</div>
        <div className="welcome-bubble">{text}</div>
      </div>
    </div>
  );
}

function AssistantMessage({ message, liveStatus, waitingText, elapsed }) {
  const logs = message.logs || [];
  const isWaiting = message.phase === "waiting";
  const isPlaying = message.phase === "playing";
  const isDone = message.phase === "done";
  const isError = message.phase === "error";

  return (
    <div className="msg msg-agent">
      <div className="avatar">AI</div>
      <div className="agent-content">
        <div className="agent-name">
          PhotoCraftAI
          {isPlaying || isWaiting ? <span className="agent-working">执行中</span> : null}
        </div>

        {logs.length > 0 ? <LogTimeline logs={logs} /> : null}

        {isWaiting ? (
          <div className="thinking">
            <Spinner />
            <span>{waitingText}</span>
            <span className="elapsed">{elapsed}s</span>
          </div>
        ) : null}

        {isPlaying ? (
          <div className="thinking thinking-compact">
            <Spinner small />
            <span>{liveStatus || "正在继续…"}</span>
          </div>
        ) : null}

        {isDone ? (
          <>
            <div className="reply-bubble">{message.reply}</div>
            <div className="reply-meta">
              <span className={`provider-badge ${message.provider === "mock" ? "is-mock" : ""}`}>
                {message.provider === "mock" ? "mock 规划器" : message.model || message.provider || "LLM"}
              </span>
              <span className={`steps-badge ${message.allStepsOk ? "ok" : "bad"}`}>
                {message.allStepsOk ? "全部步骤成功" : "有步骤失败"}
              </span>
              <span className="steps-count">{(message.logs || []).length} 步</span>
            </div>
            {message.mockWarning ? (
              <div className="mock-note">
                当前没有配置 API Key，后端使用了离线 mock 规划器。工具调用是真实的，但规划不是大模型。
              </div>
            ) : null}
            {message.warnings?.length ? (
              <div className="mock-note">{message.warnings.join("；")}</div>
            ) : null}
          </>
        ) : null}

        {isError ? (
          <div className="chat-error">
            <div className="chat-error-head">
              <WarningIcon />
              这次没有跑完
            </div>
            <p>{message.error}</p>
          </div>
        ) : null}
      </div>
    </div>
  );
}

export default function ChatPanel({
  messages,
  busy,
  health,
  liveStatus,
  waitingText,
  elapsed,
  onSend,
  onRetryHealth,
}) {
  const [input, setInput] = useState("");
  const [settingsOpen, setSettingsOpen] = useState(false);
  const scrollRef = useRef(null);
  const textareaRef = useRef(null);
  const composingRef = useRef(false);

  useEffect(() => {
    const element = scrollRef.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [messages, liveStatus, waitingText]);

  useEffect(() => {
    const element = textareaRef.current;
    if (!element) return;
    element.style.height = "auto";
    element.style.height = `${Math.min(element.scrollHeight, 132)}px`;
  }, [input]);

  const submit = () => {
    const text = input.trim();
    if (!text || busy) return;
    setInput("");
    onSend(text);
  };

  const handleKeyDown = (event) => {
    if (event.key !== "Enter" || event.shiftKey) return;
    if (composingRef.current || event.nativeEvent.isComposing) return;
    event.preventDefault();
    submit();
  };

  const hasUserMessage = messages.some((message) => message.role === "user");

  return (
    <aside className="chat-pane" aria-label="聊天栏">
      <div className="chat-head">
        <div>
          <h2>PhotoCraftAI</h2>
          <p>一句话，逐步执行</p>
        </div>
        <div className="chat-head-actions">
          <button type="button" className="chat-settings" onClick={() => setSettingsOpen(true)}>
            API Key
          </button>
          <span className={`chat-health health-${health.state}`}>
            <span className="health-dot" />
            {health.state === "online" ? "已连接" : health.state === "checking" ? "连接中" : "未连接"}
          </span>
        </div>
      </div>

      {health.state === "offline" ? (
        <div className="offline-banner" role="alert">
          <div className="offline-title">
            <WarningIcon />
            后端没启动，请先跑
          </div>
          <code>uvicorn main:app --host 127.0.0.1 --port 8000</code>
          <button type="button" onClick={onRetryHealth}>
            重新检测
          </button>
        </div>
      ) : null}

      <div className="chat-scroll" ref={scrollRef}>
        {messages.map((message) =>
          message.role === "user" ? (
            <div className="msg msg-user" key={message.id}>
              <div className="user-bubble">{message.text}</div>
            </div>
          ) : message.phase === "welcome" ? (
            <Welcome key={message.id} text={message.reply} />
          ) : (
            <AssistantMessage
              key={message.id}
              message={message}
              liveStatus={liveStatus}
              waitingText={waitingText}
              elapsed={elapsed}
            />
          ),
        )}
      </div>

      {!hasUserMessage ? (
        <div className="suggestions">
          {SUGGESTIONS.map((text) => (
            <button type="button" key={text} onClick={() => onSend(text)} disabled={busy}>
              {text}
            </button>
          ))}
        </div>
      ) : null}

      <div className="composer">
        <div className={`composer-box ${busy ? "is-busy" : ""}`}>
          <textarea
            ref={textareaRef}
            rows={1}
            value={input}
            placeholder="例如：新建 1080x1920 画布，填充蓝色"
            aria-label="输入你的需求"
            onChange={(event) => setInput(event.target.value)}
            onKeyDown={handleKeyDown}
            onCompositionStart={() => {
              composingRef.current = true;
            }}
            onCompositionEnd={() => {
              composingRef.current = false;
            }}
          />
          <button
            type="button"
            className="send-button"
            onClick={submit}
            disabled={busy || !input.trim()}
            aria-label="发送"
          >
            {busy ? <Spinner small /> : <SendIcon />}
          </button>
        </div>
        <div className="composer-hint">
          <span>Enter 发送 · Shift + Enter 换行</span>
          <span>{busy ? "Agent 正在执行，请稍候" : "支持中文输入"}</span>
        </div>
      </div>
      <SettingsModal open={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </aside>
  );
}
