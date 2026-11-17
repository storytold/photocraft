import { useEffect, useState } from "react";
import { screenUrl } from "../api";

function MouseGlyph({ className = "" }) {
  return (
    <svg className={className} viewBox="0 0 24 24" aria-hidden="true">
      <rect x="7" y="2.5" width="10" height="16" rx="5" />
      <path d="M12 2.5v6" />
      <path d="M12 21v1" />
    </svg>
  );
}

export default function LiveScreen({
  active,
  refreshKey,
  cursor,
  aiActive,
  windowSize,
  health,
  onStartApp,
  starting,
}) {
  const [tick, setTick] = useState(0);
  const [failed, setFailed] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [dims, setDims] = useState(windowSize || { width: 1440, height: 851 });

  useEffect(() => {
    if (windowSize?.width && windowSize?.height) setDims(windowSize);
  }, [windowSize]);

  useEffect(() => {
    setTick((value) => value + 1);
  }, [refreshKey]);

  useEffect(() => {
    if (!active) return undefined;
    const timer = setInterval(() => setTick((value) => value + 1), 720);
    return () => clearInterval(timer);
  }, [active]);

  const width = Number(dims?.width) || 1440;
  const height = Number(dims?.height) || 851;
  const cursorX = cursor ? Math.min(100, Math.max(0, (cursor.x / width) * 100)) : 0;
  const cursorY = cursor ? Math.min(100, Math.max(0, (cursor.y / height) * 100)) : 0;
  const connected = health?.mode === "bridge" && health?.app_connected !== false;

  return (
    <div className="live-pane">
      <div className="live-head">
        <div className="live-title">
          <span className="pane-kicker">原版界面</span>
          <h1>PhotoCraft</h1>
          <span className="live-endpoint">Bridge · 127.0.0.1:7878</span>
        </div>
        <div className={`live-state ${active ? "is-active" : ""}`}>
          <span className="live-dot" />
          {active ? "AI 操作中" : connected ? "已连接桌面 App" : "等待桌面 App"}
        </div>
      </div>

      <div className="live-body">
        <div className="screen-grid" aria-hidden="true" />
        <div className="screen-shell">
          <div
            className="screen-frame"
            style={{ aspectRatio: `${width} / ${height}` }}
          >
            <img
              key="desktop"
              className={`screen-img ${loaded ? "is-loaded" : ""}`}
              src={screenUrl(tick)}
              alt="PhotoCraft 桌面窗口实时画面"
              onLoad={(event) => {
                const img = event.currentTarget;
                setLoaded(true);
                setFailed(false);
                if (!windowSize?.width && img.naturalWidth) {
                  setDims({ width: img.naturalWidth / 2, height: img.naturalHeight / 2 });
                }
              }}
              onError={() => {
                setFailed(true);
                setLoaded(false);
              }}
            />

            {!loaded && !failed ? (
              <div className="screen-loading">
                <span className="spinner" />
                正在接收 PhotoCraft 画面…
              </div>
            ) : null}

            {failed ? (
              <div className="screen-error">
                <strong>没有收到 PhotoCraft 窗口</strong>
                <p>请先启动桌面 App，或点下面的按钮。</p>
                <code>photocraft --control 7878 --in-window-menus …</code>
                <button type="button" className="screen-start" onClick={onStartApp} disabled={starting}>
                  {starting ? "正在启动…" : "启动 PhotoCraft"}
                </button>
              </div>
            ) : null}

            <div
              className={`ai-cursor ${aiActive ? "is-on" : ""}`}
              style={{ left: `${cursorX}%`, top: `${cursorY}%` }}
              aria-hidden="true"
            >
              <svg viewBox="0 0 24 24">
                <path d="M5 3.5 18.5 12l-6 1.2L9.5 19 5 3.5Z" />
              </svg>
              <span className="ai-cursor-label">AI</span>
            </div>

            <div className={`ai-float ${aiActive ? "is-on" : ""}`}>
              <span className="ai-float-icon">
                <MouseGlyph />
              </span>
              <span>AI 正在操作</span>
              <span className="ai-float-dots">
                <i />
                <i />
                <i />
              </span>
            </div>

            <div className="screen-corner">LIVE</div>
          </div>
        </div>
      </div>

      <div className="live-foot">
        <span>左键/菜单/图层全程可见</span>
        <span className="live-foot-sep">·</span>
        <span>截图来自原版 App 窗口</span>
        {cursor ? (
          <>
            <span className="live-foot-sep">·</span>
            <span className="mono">
              cursor {Math.round(cursor.x)}, {Math.round(cursor.y)}
            </span>
          </>
        ) : null}
      </div>
    </div>
  );
}
