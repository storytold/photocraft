# PhotoCraftAI Backend

这是 PhotoCraftAI 的后端与工具目录。完整介绍、截图和架构见仓库根目录的
[`README-AI.md`](../README-AI.md)。

## 目录

| 路径 | 作用 |
| --- | --- |
| `main.py` | FastAPI：`/chat/stream` SSE、`/settings`、`/screen`、`/app/start` |
| `visual.py` | 新建画布/填充的可视化流程（File 菜单、New 对话框、Paint Bucket、图层） |
| `bridge_agent.py` | 通用 LLM Bridge Agent：任意编辑请求，一次一个真实工具 |
| `bridge.py` | `photocraft-cli mcp --bridge` 长连接与 UI 控制原语 |
| `planner.py` | 模型意图解析（尺寸/颜色） |
| `settings_store.py` | 服务商/API Key/模型/Base URL，本地 0600 保存 |
| `list_tools.py` | 第 1 步的工具清单脚本 |
| `web/` | 可选的 React 预览界面 |
| `runtime/` | token 与 API Key 配置（gitignore） |
| `workspace/` | Bridge 读写根目录（gitignore） |

## 启动

```bash
cd ai-agent
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r requirements.txt
uvicorn main:app --host 127.0.0.1 --port 8000
```

默认会自动启动 `../target/release/photocraft`。关闭自动启动：

```bash
PHOTOCRAFT_AUTO_START_APP=0 uvicorn main:app --host 127.0.0.1 --port 8000
```

## API Key

在 PhotoCraft 右侧 **PhotoCraftAI → API Key** 里填写；或写 `ai-agent/.env`：

```dotenv
LLM_PROVIDER=deepseek
DEEPSEEK_API_KEY=...
```

Key 保存在 `runtime/ai_settings.json`（0600，已 gitignore）。
