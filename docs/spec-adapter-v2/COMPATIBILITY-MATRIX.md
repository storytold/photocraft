# PhotoCraft Adapter v1 / v2 Codec 契约与版本兼容矩阵

> **规范状态**：评审草案（Review Draft 2.0.0-draft-review-1）  
> **注意**：本文件为 M2 阶段最小规范冻结评审包的一部分，未经过明确评审确认前**不是冻结标准（Not Frozen）**。

---

## 1. 三种版本的严格分离（Three-Version Separation）

在 Adapter 架构演进中，必须严格区分以下三类版本，禁止在单一版本号中混淆生命周期、线协议和操作语义：

| 版本维度 | 字段位置 | 规范责任范围 | 典型值 | 演进规则 |
|---|---|---|---|---|
| **包清单版本**<br>`package manifest_version` | `adapter.json` 根级 `manifest_version` | 描述适配器包结构、可执行文件路径、运行时架构、资源库发现规则及安全/内存预算配额 | `1` (v1 单一包),<br>`2` (v2 资源隔离包) | 当包元数据结构、清单清单模式或安全配额模型变更时递增。 |
| **线传输协议版本**<br>`wire protocol version` | 帧头魔数 (`magic`) 及 JSON 头 `protocol` | 进程间通信（IPC）物理帧边界：魔数、头部长度、Section 寻址表、对齐规则与二进制 Payload 解包 | `1` (`PCADREQ1`/`PCADRES1`),<br>`2` (`PCADREQ2`/`PCADRES2`) | 帧格式、分段寻址或底层编码机制变更时递增；必须与数据载荷语义解耦。 |
| **操作语义版本**<br>`operation semantic version` | 操作条目 `operations[].version` 与请求中 `operation_version` | 具体图像处理能力的语义契约：输入输出端口集合、几何映射策略、色彩模式与深度、参数 Schema | `0.1.0`, `1.0.0`,<br>`2.0.0` (SemVer) | 遵循语义化版本控制（SemVer）。新增端口/参数且向后兼容递增 Minor；破坏性端口或语义变更递增 Major。 |

---

## 2. v1 与 v2 物理帧结构对比与严格解码契约

### 2.1 物理帧结构对比

```text
v1 线协议结构 (Wire v1):
+--------------------+------------------+------------------------------+---------------------------+
| Magic (8 Bytes)    | HeaderLen (4B LE)| JSON Header (HeaderLen)      | Raw f32 Payload (Samples) |
| "PCADREQ1"/"RES1"  | u32le (<= 1 MiB) | UTF-8 JSON (deny_unknown)    | IEEE754 f32 LE Interleaved|
+--------------------+------------------+------------------------------+---------------------------+

v2 线协议结构 (Wire v2):
+--------------------+------------------+------------------------------+--------------------+-------------------------+---------------------------+
| Magic (8 Bytes)    | HeaderLen (4B LE)| JSON Header (HeaderLen)      | SecCount (4B LE)   | Section Table (Sec*16B) | Payload Area (Sec Data)   |
| "PCADREQ2"/"RES2"  | u32le (<= 1 MiB) | UTF-8 JSON (Draft 2020-12)   | u32le (0..=64)     | [kind,res,offset,len]   | 4-byte aligned f32/ICC    |
+--------------------+------------------+------------------------------+--------------------+-------------------------+---------------------------+
```

### 2.2 严格解码器契约（Strict Decoder Contract）

1. **v1 Codec 绝对不变性**：
   - Rust 宿主现有 v1 解码器（`crates/plugins/src/worker.rs`）严格标注 `#[serde(deny_unknown_fields)]`。
   - **禁止行为**：严禁在 v1 的 `ControlHeader` 或 `RunHeader` 中塞入任何 v2 的字段（如 `request_id`, `sections`, `execution` 等）。
   - **禁止行为**：严禁放宽 v1 解码器为宽松解析（lenient / ignore unknown fields）以掩盖协议兼容错误。
2. **v2 Codec 独立性**：
   - v2 必须由完全独立的解码函数（如 `read_request_v2` / `read_response_v2`）处理，其第一道关卡为严格校验 8 字节魔数：必须精确匹配 `b"PCADREQ2"` 或 `b"PCADRES2"`。
   - v2 解码器对于非 v2 帧直接报错，禁止尝试猜测或“自动降级解析”。
3. **分段表与数据区安全校验**：
   - 在分配任何内存缓冲区之前，解码器必须先校验：
     1. 文件长度 $\ge 8 + 4 + \text{HeaderLen} + 4 + \text{SecCount} \times 16 + \sum \text{SecLen}$；
     2. 每个分段偏移 $\text{offset}$ 必须 4 字节对齐，且 $\text{offset} + \text{len} \le \text{PayloadAreaLen}$；
     3. 分段之间严禁重叠（no overlapping）、严禁未引用分段（no unreferenced orphan sections）、严禁多端口引用同一分段（no aliased sections）。

---

## 3. Host / Worker 版本兼容矩阵

PhotoCraft 宿主支持插件体系的渐进平滑升级，系统应按照如下矩阵处理不同版本的通信：

| 场景 | 宿主版本 (Host) | 工作进程版本 (Worker) | 协商与调用机制 | 行为表现与要求 |
|---|---|---|---|---|
| **A** | **Host v1** | **Worker v1** | 宿主读取 `adapter.json`（`manifest_version: 1`），使用 CLI 参数 `--action <probe\|prepare\|run> --request <req.bin> --response <resp.bin>`。 | 现有基准模式，通信协议为 `PCADREQ1`/`PCADRES1`。完全保持现状。 |
| **B** | **Host v1** | **Worker v2 独占** | 宿主识别到 `manifest_version: 2` 或 Worker 仅声明支持 wire v2。 | **在分发前拒绝（Fail Fast）**：宿主 UI 明确提示“插件需要 PhotoCraft v2 协议支持，请升级主程序”，不启动外部进程，不发送错误报文。 |
| **C** | **Host v2** | **Worker v1 独占** | 宿主读取包清单识别为 `manifest_version: 1`，或者根据 descriptor 探测确认仅支持 v1。 | **显式向下兼容**：Host 切换到内置的纯 v1 独立 Codec 进行通信。严禁向 v1 Worker 发送 `PCADREQ2` 帧；严禁发送包含 `request_id` 或 Section Table 的请求。 |
| **D** | **Host v2** | **双协议 Worker (v1+v2)** | Worker 在 manifest 的 `supported_wire_versions` 中同时声明 `[1, 2]`。 | **优先协商 v2**：Host 启动 Worker 并发送 `type: hello`（`PCADREQ2`）。若 Worker 成功返回 `status: negotiated` 且选择版本 2，后续全部走 v2；若协商失败，干净终止并根据策略决定是否允许用户显式选择以 v1 退化模式启动新进程。 |
| **E** | **Host v2** | **Worker v2 独占** | 双方均支持 v2。 | **完整 v2 协议**：支持完整的 `hello` 握手、细粒度生命周期（`probe`/`prepare`/`listResources`/`evict`/`run`）、多 Section 数据帧与严格无回退设备策略。 |
| **F** | **任意** | **协议版本不匹配** | 握手返回 `error/unsupported_version` 或魔数无法识别。 | **安全中断**：立即终止该进程，销毁临时通信文件，记录结构化错误日志，绝不静默重试或执行未知二进制。 |

---

## 4. 握手与能力协商契约（Handshake & Capability Negotiation）

在 v2 模式下，Worker 进程启动后，首个请求必须是 `type: hello`：

```json
// Hello Request Header
{
  "protocol": 2,
  "request_id": "req-init-001",
  "type": "hello",
  "negotiation": {
    "wire_versions": [2],
    "supported_features": ["image-mask-v2", "same-geometry", "resource-control"],
    "required_features": ["image-mask-v2", "same-geometry", "resource-control"]
  },
  "sections": []
}
```

- **最小基础能力集**：
  1. `image-mask-v2`：支持端口化 image 输入与 mask 输出规范。
  2. `same-geometry`：输出掩码尺寸严格等于输入图像尺寸。
  3. `resource-control`：支持独立模型资源标识与生命周期状态查询。
- **未知能力处置规则**：
  - 如果 Host 提出的 `required_features` 中包含 Worker 不支持的特性，Worker **必须**返回 `status: "error"`，错误码为 `unsupported_feature`。
  - 对于可选特性（在 `supported_features` 中但不在 `required_features` 中），若未达成一致，必须在当前会话中**保持禁用状态（Disabled）**。
  - 本阶段**明确拒绝**的高级/远期能力（作为未实现特性拦截）：
    - `streaming-frames`（流式帧管道）
    - `daemon-service`（常驻多租户服务）
    - `dynamic-geometry`（动态自由缩放几何）
    - `photoshop-plugin-host`（Photoshop 滤镜桥接）
