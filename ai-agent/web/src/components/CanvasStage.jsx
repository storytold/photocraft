import { colorName } from "../lib/steps";

function EmptyArt() {
  return (
    <div className="empty-art">
      <div className="empty-frame">
        <svg viewBox="0 0 64 64" aria-hidden="true">
          <rect x="8" y="12" width="48" height="40" rx="3" />
          <path d="M14 44 26 31l8 8 6-6 10 11" />
          <circle cx="42" cy="23" r="4" />
        </svg>
      </div>
      <h2>画布会出现在这里</h2>
      <p>在右侧输入一句话，例如“新建 1080x1920 画布，填充蓝色”。</p>
    </div>
  );
}

function LayerChip({ layer }) {
  const kind = String(layer.kind || "Layer");
  return (
    <span className="layer-chip" title={`${kind} · ${layer.name || ""}`}>
      <span className={`layer-kind kind-${kind.toLowerCase()}`}>{kind === "Fill" ? "填充" : kind === "Pixel" ? "像素" : kind}</span>
      <span className="layer-name">{layer.name || "未命名图层"}</span>
    </span>
  );
}

export default function CanvasStage({ canvas, fillColor, busy }) {
  const width = Number(canvas?.width) || 1080;
  const height = Number(canvas?.height) || 1920;
  const layers = Array.isArray(canvas?.layers) ? canvas.layers : [];
  const portrait = height > width;
  const fill = fillColor || "#ffffff";

  const artStyle = {
    aspectRatio: `${width} / ${height}`,
    background: fill,
    ...(portrait
      ? { height: "min(62vh, 680px)", width: "auto" }
      : { width: "min(100%, 900px)", height: "auto" }),
    maxWidth: "100%",
    maxHeight: "62vh",
  };

  return (
    <div className="canvas-stage">
      <div className="pane-head">
        <div className="pane-title">
          <span className="pane-kicker">画布</span>
          <h1>{canvas?.name || "未命名画布"}</h1>
          <span className="pane-dims">
            {canvas ? `${width} × ${height} px` : "等待第一条指令"}
          </span>
        </div>
        <span className={`live-badge ${busy ? "is-live" : ""}`}>
          <span className="live-dot" />
          {busy ? "Agent 编辑中" : "MCP 待命"}
        </span>
      </div>

      <div className="canvas-area">
        <div className="canvas-grid" aria-hidden="true" />
        {canvas ? (
          <div className="canvas-wrap">
            <div className={`canvas-art ${busy ? "is-busy" : ""}`} style={artStyle}>
              <span className="canvas-corner top-left" />
              <span className="canvas-corner bottom-right" />
            </div>
            <div className="canvas-caption">
              <span className="caption-created">画布已创建</span>
              <span className="caption-sep">·</span>
              <span>{colorName(fill)}</span>
              <span className="caption-sep">·</span>
              <span>
                {width} × {height}
              </span>
              <span className="caption-sep">·</span>
              <span>{layers.length} 个图层</span>
            </div>
          </div>
        ) : (
          <EmptyArt />
        )}
      </div>

      <div className="canvas-foot">
        <div className="layer-strip">
          {layers.length ? (
            <>
              {layers.slice(0, 4).map((layer, index) => (
                <LayerChip key={`${layer.id ?? index}-${layer.name ?? ""}`} layer={layer} />
              ))}
              {layers.length > 4 ? <span className="layer-more">+{layers.length - 4}</span> : null}
            </>
          ) : (
            <span className="layer-empty">还没有图层</span>
          )}
        </div>
        <div className="canvas-stats">
          <span>{canvas?.mode ? String(canvas.mode).toUpperCase() : "RGB"}</span>
          <span>{canvas?.depth ? `${canvas.depth} bit` : "8 bit"}</span>
          <span>headless MCP</span>
        </div>
      </div>
    </div>
  );
}
