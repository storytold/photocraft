# PhotoCraft Adapter v2 生命周期状态机与操作语义规范

> **规范状态**：评审草案（Review Draft 2.0.0-draft-review-1）  
> **注意**：本文件为 M2 阶段最小规范冻结评审包的一部分，未经过明确评审确认前**不是冻结标准（Not Frozen）**。

---

## 1. 模型资源就绪状态划分（Readiness Levels）

为了彻底杜绝“配置了假模型就宣称 NPU 就绪”的欺骗性实现，v2 规范严格区分五个不可逾越的就绪层级：

```text
[ declared ]清单声明 
      │
      ▼ (磁盘文件检查)
[ files_present ]文件存在
      │
      ▼ (全量 SHA-256 摘要比对)
[ files_verified ]校验完成
      │
      ▼ (加载运行时 DLL、构建计算图、分配内存会话)
[ session_created ]会话可创建
      │
      ▼ (送入真实输入张量、硬件 NPU 实际执行、校验输出数值)
[ inference_verified ]实际推理成功
```

| 状态名称 | 严格技术准入条件 | 是否允许建立 Session | 是否代表硬件推理成功 | 触发方式 |
|---|---|---|---|---|
| `declared` | 模型资源仅在清单或注册表中声明，尚未确认本地文件是否存在。 | **否** | **否** | 初始包发现 |
| `files_present` | 资源记录中声明的所有文件（模型、权重、配置、许可）均在本地磁盘存在且大小一致。 | **否** | **否** | `probe` |
| `files_verified` | 本地文件的 SHA-256 摘要与资源清单及 `manifest_digest` 逐字节比对完全一致。 | **否** | **否** | `probe` / `prepare` |
| `session_created` | 目标后端（如 QNN HTP / ONNX Runtime）已成功加载模型文件，通过图编译并预留设备执行上下文。 | **是** | **否**（尚未喂入真实数据） | `prepare` |
| `inference_verified` | 在授权设备上完成真实图执行，输出有效掩码，生成合规的 `execution` 执行报告。 | **是** | **是** | `run` |

---

## 2. 五大控制与执行动作语义（Actions Semantics）

### 2.1 动作语义矩阵

| 动作 (`action`) | 入参要求 | 语义定义与边界 | 成功返回状态 | 典型错误码 |
|---|---|---|---|---|
| `probe` | `resource`, `operation`, `operation_version`, `device_policy` | **只读探测**。检查运行环境、依赖项与模型文件是否存在及校验。**绝对不得**启动网络下载，不得初始化 NPU 计算图，不得分配常驻模型会话。 | `status: "ready"`<br>`readiness: "declared"` / `"files_present"` / `"files_verified"`<br>（**严禁返回 `session_created`**） | `resource_missing`<br>`resource_invalid`<br>`device_unavailable` |
| `prepare` | `resource`, `operation`, `operation_version`, `device_policy` | **会话预热**。执行文件完整性校验，并尝试在目标设备上初始化推理会话（如构建 QNN HTP Session）。不得执行网络下载。 | `status: "ready"`<br>`readiness: "files_verified"` 或 `"session_created"` | `resource_missing`<br>`resource_invalid`<br>`device_unavailable`<br>`policy_violation` |
| `listResources` | 可选 `resource`（若提供则过滤，若缺省则枚举全局） | **受限枚举**。枚举本地已安装且属于当前 Worker 范围的模型资源清单及各个资源的就绪状态。最多返回 256 条，超出报错。 | `status: "ready"`<br>`resources: [...]`（包含 identity、operation 和 readiness） | `unsupported_operation`<br>`internal_error` |
| `evict` | `resource`, `evict_scope: "sessions" \| "derived_cache"` | **显式释放**。释放指定的模型推理上下文（Session 句柄、图缓存）。**严禁删除**磁盘上的原始模型文件、许可文件或系统模型。 | `status: "ready"`（无输出载荷与执行报告） | `invalid_request`<br>`internal_error` |
| `run` | `resource`, `operation`, `operation_version`, `device_policy`, `params`, `inputs`, `sections` | **单次推理**。将输入图像切片并送入指定模型执行，返回对齐的掩码结果、物理分段数据及执行报告。 | `status: "ok"`<br>`outputs`, `sections`, `execution` | `unsupported_geometry`<br>`policy_violation`<br>`deadline_exceeded`<br>`device_unavailable` |

---

## 3. 设备策略与“零回退”强制保障（Strict Device Policy）

在 AI 图像处理应用中，若用户显式指定使用 NPU 加速，系统**绝对禁止在 NPU 失败时静默回退到 CPU 并谎称成功**。

### 3.1 设备策略模式 (`device_policy.mode`)

1. `mode: "exact"`（精确指定）：
   - 必须提供 `target_device`（如 `"qualcomm.qnn.htp.0"`）。
   - **强约束**：`allow_graph_fallback` 必须为 `false`（Draft 2020-12 Schema 层面强校验）。
   - 行为：若指定设备无法分配会话或初始化失败，立即报错 `device_unavailable`，禁止尝试其他设备。
2. `mode: "require_kind"`（按类别约束）：
   - 必须提供 `target_kind`（`"npu"` / `"gpu"` / `"cpu"`）。
   - **强约束**：`allow_graph_fallback` 必须为 `false`。
   - 行为：若指定类型的设备不可用，立即报错 `device_unavailable`。
3. `mode: "auto"`（自动协商）：
   - 仅允许在声明的 `allowed_backends` 名单内选择最佳设备。
   - 若 `allow_graph_fallback: false`，则一旦选定后端发生编译或图执行失败，必须报错，不得隐式回退。

### 3.2 真实执行报告（Execution Report）

`run` 成功返回时必须包含结构化 `execution` 对象，供宿主审计：
- `requested_policy`：回显请求策略。
- `devices`：实际参与计算的硬件设备列表（包含 `id` 与 `kind`）。
- `backend`：实际执行后端标识（必须包含在 `allowed_backends` 中）。
- `graph_partitioned`：是否发生算子切分。
- `graph_fallback`：是否发生降级回退（若策略要求 `false` 则此处必须为 `false`，否则报错 `policy_violation`）。
- `timing`：耗时统计（`cpu_preprocess_ms`, `graph_execution_ms`, `cpu_postprocess_ms`, `total_ms`），满足串行一致性。

---

## 4. 端口约束、色彩策略与内存预算

### 4.1 图像端口与色彩契约

- **输入主图像 (`source`)**：
  - 格式必须为 `kind: "image"`。
  - 颜色模式 `color_mode`：`"rgb"`, `"gray"`, `"cmyk"`, `"lab"`。
  - 色彩编码 `color_encoding`：
    - `"icc"`：必须指定对应的 ICC 数据分段（`icc_section_index`），且 ICC 分段长度 $\le 4 \text{ MiB}$。
    - `"srgb"` / `"linear-srgb"`：仅适用于 RGB 模式，禁止附加 ICC 分段。
    - `"untagged"`：必须在操作参数中具有明确的转换策略，禁止宿主未经验证隐式假定为 sRGB。
  - Alpha 通道：若存在必须为直线透明度（straight alpha），取值范围 $[0.0, 1.0]$。
- **输出掩码 (`mask`)**：
  - 格式必须为 `kind: "mask"`。
  - 单通道（`channels: 1`），单色灰度（`color_mode: "gray"`），浮点深度（`source_depth: 32`）。
  - 无 Alpha 通道（`alpha: false`），无 ICC 配置文件。
  - 采样值范围严格落在 $[0.0, 1.0]$，禁止出现 NaN 或 Infinity。

### 4.2 尺寸与内存预算硬上限（Hard Budget Limits）

| 配额维度 | 规范硬上限 | 违规处理 |
|---|---|---|
| **JSON 头大小** | $\le 1,048,576 \text{ 字节 (1 MiB)}$ | 解码器直接拒绝，返回 `invalid_request` |
| **Section 分段数** | $0 \le N \le 64$ | 超出拒绝，返回 `invalid_request` |
| **ICC 单分段大小** | $\le 4,194,304 \text{ 字节 (4 MiB)}$ | 超出拒绝，返回 `policy_violation` |
| **ICC 聚合大小** | $\le 16,777,216 \text{ 字节 (16 MiB)}$ | 超出拒绝，返回 `policy_violation` |
| **单个张量样本总数** | $\le 67,108,864 \text{ 浮点样本 (256 MiB)}$ | 超出拒绝，返回 `unsupported_geometry` |
| **单帧 Payload 总大小** | $\le 268,435,456 \text{ 字节 (256 MiB)}$ | 超出拒绝，返回 `policy_violation` |
| **单图最大像素数** | $16,384 \times 16,384$ 像素 | 超出拒绝，返回 `unsupported_geometry` |
| **单模型包聚合大小** | $\le 34,359,738,368 \text{ 字节 (32 GiB)}$ | 校验拒绝，返回 `resource_invalid` |

---

## 5. 错误码与失败隔离

成功响应与失败响应严格互斥：
- **成功状态**（`status: "ok"` 或 `status: "ready"`）：
  - 必须包含对应的成功字段（如 `outputs`, `execution` 或 `readiness`）。
  - **绝对不得**出现 `error` 字段。
- **失败状态**（`status` 取值不为 `ok`/`ready`/`negotiated`）：
  - 必须包含 `error: { "code": string, "retryable": boolean }`。
  - **绝对不得**出现 `outputs`, `execution`, `resources` 字段。
  - 二进制分段数 `section_count` 必须严格为 `0`，Payload 大小为 `0`。

### 规范标准错误码清单

```json
[
  "unsupported_version",    // 协议或操作版本不受支持
  "unsupported_feature",    // 协商中请求了未实现的能力
  "invalid_request",        // 报文结构、JSON 语法或分段表损坏
  "unsupported_operation",  // 操作标识不存在或已废弃
  "unsupported_geometry",   // 尺寸越界、非 same_geometry 或超出最大像素限制
  "resource_missing",       // 请求的模型资源不存在或文件丢失
  "resource_invalid",       // 资源清单摘要不匹配或权重文件损坏
  "permission_required",    // 缺少必要的操作权限或本地用户授权
  "device_unavailable",     // 指定的硬件（NPU/GPU）不存在、离线或不可访问
  "policy_violation",       // 违反策略（如禁止 CPU 回退却尝试降级、超出内存配额）
  "cancelled",              // 请求在完成前被显式取消
  "deadline_exceeded",      // 超过声明或协商的超时时限
  "internal_error"          // Worker 内部未捕获的严重异常
]
```
