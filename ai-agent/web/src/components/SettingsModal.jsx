import { useEffect, useState } from "react";
import { getSettings, saveSettings, testSettings } from "../api";

const PROVIDERS = [
  {
    id: "deepseek",
    label: "DeepSeek",
    model: "deepseek-chat",
    base_url: "https://api.deepseek.com",
  },
  {
    id: "qwen",
    label: "Qwen / 通义千问",
    model: "qwen-plus",
    base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
  },
  {
    id: "openai",
    label: "OpenAI",
    model: "gpt-4o-mini",
    base_url: "https://api.openai.com/v1",
  },
];

export default function SettingsModal({ open, onClose }) {
  const [provider, setProvider] = useState("deepseek");
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState("deepseek-chat");
  const [baseUrl, setBaseUrl] = useState("https://api.deepseek.com");
  const [hasKey, setHasKey] = useState(false);
  const [maskedKey, setMaskedKey] = useState("");
  const [status, setStatus] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setStatus("正在读取已保存的配置…");
    getSettings()
      .then((data) => {
        if (cancelled) return;
        setProvider(data.provider || "deepseek");
        setModel(data.model || "deepseek-chat");
        setBaseUrl(data.base_url || "https://api.deepseek.com");
        setHasKey(Boolean(data.has_key));
        setMaskedKey(data.masked_key || "");
        setApiKey("");
        setStatus(data.has_key ? `已保存 Key：${data.masked_key}` : "还没有配置 API Key");
      })
      .catch((error) => {
        if (!cancelled) setStatus(`读取失败：${error.message}`);
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  if (!open) return null;

  const changeProvider = (id) => {
    const found = PROVIDERS.find((item) => item.id === id);
    setProvider(id);
    if (found) {
      setModel(found.model);
      setBaseUrl(found.base_url);
    }
  };

  const submit = async () => {
    if (busy) return;
    setBusy(true);
    setStatus("正在保存并测试连接…");
    try {
      await saveSettings({
        provider,
        api_key: apiKey.trim(),
        model: model.trim(),
        base_url: baseUrl.trim(),
      });
      const result = await testSettings();
      setHasKey(true);
      setStatus(`连接成功：${result.provider || provider} / ${result.model || model}`);
      setApiKey("");
    } catch (error) {
      setStatus(`连接失败：${error.message}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-overlay" role="dialog" aria-modal="true" onMouseDown={onClose}>
      <div className="settings-card" onMouseDown={(event) => event.stopPropagation()}>
        <div className="settings-head">
          <div>
            <h2>PhotoCraftAI 设置</h2>
            <p>API Key 只保存在本机，不会上传到别处。</p>
          </div>
          <button type="button" className="settings-close" onClick={onClose} aria-label="关闭">
            ×
          </button>
        </div>

        <label className="settings-field">
          <span>服务商</span>
          <select value={provider} onChange={(event) => changeProvider(event.target.value)}>
            {PROVIDERS.map((item) => (
              <option key={item.id} value={item.id}>
                {item.label}
              </option>
            ))}
          </select>
        </label>

        <label className="settings-field">
          <span>API Key</span>
          <input
            type="password"
            value={apiKey}
            placeholder={hasKey ? `已保存：${maskedKey}（留空保持不变）` : "粘贴 API Key"}
            onChange={(event) => setApiKey(event.target.value)}
            autoComplete="off"
          />
        </label>

        <label className="settings-field">
          <span>模型</span>
          <input value={model} onChange={(event) => setModel(event.target.value)} />
        </label>

        <label className="settings-field">
          <span>Base URL</span>
          <input value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} />
        </label>

        <div className={`settings-status ${status.includes("失败") ? "is-error" : ""}`}>{status}</div>

        <div className="settings-actions">
          <button type="button" className="settings-secondary" onClick={onClose}>
            关闭
          </button>
          <button type="button" className="settings-primary" onClick={submit} disabled={busy}>
            {busy ? "测试中…" : "保存并测试"}
          </button>
        </div>
      </div>
    </div>
  );
}
