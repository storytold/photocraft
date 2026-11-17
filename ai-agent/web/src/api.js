const API_BASE = (import.meta.env.VITE_API_BASE || "/api").replace(/\/$/, "");

export class ApiError extends Error {
  constructor(message, status, detail) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.detail = detail;
  }
}

export function screenUrl(tick = 0) {
  return `${API_BASE}/screen?t=${tick}`;
}

export async function checkHealth(signal) {
  let response;
  try {
    response = await fetch(`${API_BASE}/health`, {
      signal,
      headers: { Accept: "application/json" },
    });
  } catch (error) {
    if (error.name === "AbortError") throw error;
    throw new ApiError("无法连接后端", 0);
  }
  if (!response.ok) throw new ApiError("后端未就绪", response.status);
  return response.json();
}

export async function getSettings() {
  return request("/settings");
}

export async function saveSettings(payload) {
  return request("/settings", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export async function testSettings() {
  return request("/settings/test", { method: "POST" });
}

export async function startApp() {
  const response = await fetch(`${API_BASE}/app/start`, { method: "POST" });
  const body = await response.json().catch(() => null);
  if (!response.ok) {
    throw new ApiError(
      (body && (body.detail || body.message)) || `HTTP ${response.status}`,
      response.status,
      body,
    );
  }
  return body;
}

export async function streamChat(message, { signal, onEvent }) {
  let response;
  try {
    response = await fetch(`${API_BASE}/chat/stream`, {
      method: "POST",
      signal,
      headers: {
        "Content-Type": "application/json",
        Accept: "text/event-stream",
      },
      body: JSON.stringify({ message }),
    });
  } catch (error) {
    if (error.name === "AbortError") throw error;
    throw new ApiError("无法连接后端", 0);
  }

  if (!response.ok) {
    const body = await response.json().catch(() => null);
    throw new ApiError(
      (body && (body.detail || body.message)) || `HTTP ${response.status}`,
      response.status,
      body,
    );
  }
  if (!response.body) throw new ApiError("浏览器不支持流式响应", 0);

  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });

    let index = buffer.indexOf("\n\n");
    while (index !== -1) {
      const block = buffer.slice(0, index);
      buffer = buffer.slice(index + 2);
      for (const line of block.split("\n")) {
        if (!line.startsWith("data:")) continue;
        const raw = line.slice(5).trim();
        if (!raw) continue;
        try {
          onEvent(JSON.parse(raw));
        } catch {
          // A malformed event must not kill the rest of the stream.
        }
      }
      index = buffer.indexOf("\n\n");
    }
  }
}
