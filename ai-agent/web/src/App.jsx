import { useCallback, useEffect, useRef, useState } from "react";
import ChatPanel from "./components/ChatPanel";
import LiveScreen from "./components/LiveScreen";
import { ApiError, checkHealth, startApp, streamChat } from "./api";

const WELCOME = {
  id: "welcome",
  role: "assistant",
  phase: "welcome",
  reply:
    "你好，我是 PhotoCraftAI。我会真的去点原版 PhotoCraft 的菜单、选工具、建图层。试试“新建 1080x1920 画布，填充蓝色”，每一步都会在左边发生。",
};

const WAITING = "正在连接桌面版 PhotoCraft…";

function uid() {
  if (typeof crypto !== "undefined" && crypto.randomUUID) return crypto.randomUUID();
  return `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

export default function App() {
  const [messages, setMessages] = useState([WELCOME]);
  const [busy, setBusy] = useState(false);
  const [health, setHealth] = useState({ state: "checking" });
  const [liveStatus, setLiveStatus] = useState("");
  const [elapsed, setElapsed] = useState(0);
  const [cursor, setCursor] = useState(null);
  const [windowSize, setWindowSize] = useState({ width: 1440, height: 851 });
  const [screenRefresh, setScreenRefresh] = useState(0);
  const [aiActive, setAiActive] = useState(false);
  const [startingApp, setStartingApp] = useState(false);
  const autoStartRef = useRef(false);
  const abortRef = useRef(null);
  const busyRef = useRef(false);
  const aiTimerRef = useRef(null);

  const probeHealth = useCallback(async () => {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 5000);
    try {
      const data = await checkHealth(controller.signal);
      setHealth({ state: "online", ...data });
      if (data.window?.width) setWindowSize(data.window);
    } catch {
      setHealth({ state: "offline" });
    } finally {
      clearTimeout(timer);
    }
  }, []);

  useEffect(() => {
    probeHealth();
    const timer = setInterval(probeHealth, 12000);
    return () => clearInterval(timer);
  }, [probeHealth]);

  useEffect(() => {
    if (!busy) return undefined;
    const started = Date.now();
    setElapsed(0);
    const timer = setInterval(() => setElapsed(Math.floor((Date.now() - started) / 1000)), 1000);
    return () => clearInterval(timer);
  }, [busy]);

  useEffect(
    () => () => {
      if (aiTimerRef.current) clearTimeout(aiTimerRef.current);
      if (abortRef.current) abortRef.current.abort();
    },
    [],
  );

  const handleStartApp = useCallback(async () => {
    if (startingApp) return;
    setStartingApp(true);
    try {
      await startApp();
      await probeHealth();
      setScreenRefresh((value) => value + 1);
    } catch (error) {
      console.error(error);
    } finally {
      setStartingApp(false);
    }
  }, [probeHealth, startingApp]);

  useEffect(() => {
    if (autoStartRef.current) return undefined;
    if (health.state === "online" && health.app_connected === false) {
      autoStartRef.current = true;
      const timer = setTimeout(handleStartApp, 1200);
      return () => clearTimeout(timer);
    }
    return undefined;
  }, [health.state, health.app_connected, handleStartApp]);

  const patchMessage = useCallback((id, patch) => {
    setMessages((prev) =>
      prev.map((message) => {
        if (message.id !== id) return message;
        return typeof patch === "function" ? patch(message) : { ...message, ...patch };
      }),
    );
  }, []);

  const handleSend = useCallback(
    async (raw) => {
      const text = String(raw || "").trim();
      if (!text || busyRef.current) return;
      busyRef.current = true;
      if (aiTimerRef.current) clearTimeout(aiTimerRef.current);

      const userId = uid();
      const agentId = uid();
      setMessages((prev) => [
        ...prev,
        { id: userId, role: "user", text },
        {
          id: agentId,
          role: "assistant",
          phase: "waiting",
          logs: [],
          reply: "",
          startedAt: Date.now(),
        },
      ]);
      setBusy(true);
      setAiActive(true);
      setLiveStatus(WAITING);
      setElapsed(0);
      setCursor(null);

      const controller = new AbortController();
      abortRef.current = controller;
      const timeout = setTimeout(() => controller.abort(), 180000);
      let hadError = false;

      try {
        await streamChat(text, {
          signal: controller.signal,
          onEvent: (event) => {
            if (event.type === "hello") {
              if (event.window?.width) setWindowSize(event.window);
              return;
            }
            if (event.type === "status") {
              setLiveStatus(event.text || "");
              setAiActive(true);
              return;
            }
            if (event.type === "step") {
              patchMessage(agentId, (message) => ({
                ...message,
                phase: "playing",
                logs: [...message.logs, event],
              }));
              if (event.cursor) setCursor(event.cursor);
              setAiActive(true);
              setScreenRefresh((value) => value + 1);
              return;
            }
            if (event.type === "error") {
              hadError = true;
              patchMessage(agentId, { phase: "error", error: event.error || "执行失败" });
              setAiActive(false);
              return;
            }
            if (event.type === "done" && !hadError) {
              patchMessage(agentId, {
                phase: "done",
                reply: event.reply || "完成。",
                allStepsOk: true,
                provider: "bridge",
                model: "Bridge",
                mode: "bridge",
                document: event.document,
              });
              setLiveStatus("完成");
              setScreenRefresh((value) => value + 1);
              if (aiTimerRef.current) clearTimeout(aiTimerRef.current);
              aiTimerRef.current = setTimeout(() => setAiActive(false), 1400);
            }
          },
        });
      } catch (error) {
        hadError = true;
        let message = "请求失败，请稍后重试。";
        if (error.name === "AbortError") {
          message = "请求超时或被取消（180 秒）。请检查桌面 App 和后端是否还在运行。";
        } else if (error instanceof ApiError) {
          if (error.status === 0) {
            message = "无法连接后端。请确认 uvicorn 已经启动。";
            setHealth({ state: "offline" });
          } else {
            message = `后端返回错误（${error.status}）：${error.detail || error.message}`;
          }
        } else if (error instanceof Error) {
          message = error.message;
        }
        patchMessage(agentId, { phase: "error", error: message });
        setAiActive(false);
      } finally {
        busyRef.current = false;
        clearTimeout(timeout);
        setBusy(false);
        abortRef.current = null;
      }
    },
    [patchMessage],
  );

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand">
          <div className="brand-mark" aria-hidden="true">
            <svg viewBox="0 0 24 24" role="img">
              <path d="M4 17.5 16.8 4.7l2.5 2.5L6.5 20H4v-2.5Z" />
              <path d="M14.5 7 17 9.5" />
            </svg>
          </div>
          <div className="brand-text">
            <div className="brand-name">PhotoCraftAI</div>
            <div className="brand-sub">PhotoCraft · Bridge</div>
          </div>
        </div>

        <div className="topbar-meta">
          <span className="endpoint">127.0.0.1:7878</span>
          <span className={`health-pill health-${health.state}`}>
            <span className="health-dot" />
            {health.state === "online"
              ? health.app_connected
                ? "桌面 App 已连接"
                : "桌面 App 未连接"
              : health.state === "checking"
                ? "正在连接…"
                : "后端离线"}
          </span>
        </div>
      </header>

      <main className="workspace">
        <section className="canvas-pane" aria-label="PhotoCraft 原版界面">
          <LiveScreen
            active={busy}
            refreshKey={screenRefresh}
            cursor={cursor}
            aiActive={aiActive}
            windowSize={windowSize}
            health={health}
            onStartApp={handleStartApp}
            starting={startingApp}
          />
        </section>

        <ChatPanel
          messages={messages}
          busy={busy}
          health={health}
          liveStatus={liveStatus}
          waitingText={liveStatus || WAITING}
          elapsed={elapsed}
          onSend={handleSend}
          onRetryHealth={probeHealth}
        />
      </main>
    </div>
  );
}
