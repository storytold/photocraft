# PhotoCraft 通用兼容适配器与 QNN 后端：Flash Agent 实施计划书

- 日期：2026-10-21
- 文档性质：**设计与交接计划，不是实施完成报告，也不是继续自动修改的授权。**
- 目标执行者：Flash agent；执行前由用户明确授权启动。
- 当前决定：停止本轮实现、构建、提交、推送；仅编写本计划。
- 核心结论：现有架构可以保留，但尚未满足完整的开放模型接口和 PS 神经插件兼容目标。需要先修互通缺陷，再冻结最小通用规范，随后实现普通 ARM64 QNN Worker 与验证链路。

## 0. 给 Flash 的执行摘要

### 用户真正要的东西

1. PhotoCraft 主程序继续是原生 Rust 应用，不为了 NPU 改成 MSIX 应用、不内嵌 Photoshop、不依赖 Photoshop 自动化。
2. 新增**普通 Windows ARM64 外部 Worker 的 QNN 后端**，通过 ONNX Runtime QNN EP 调用 Snapdragon Hexagon NPU；不要求安装 Windows AI 系统模型 MSIX 伴随包。
3. 接口不是只为“选择主体”或某一家模型服务：未来其他合法部署的模型、第三方过滤器桥接、其他宿主应用都能够复用同一协议。
4. “能够共享模型计算”和“能直接运行 Photoshop 插件/Adobe Neural Filters”不是一件事。不得承诺尚无实际加载器、许可或测试的兼容性。
5. **所有编译、测试运行、打包在 GitHub Actions 上完成。**本地允许阅读、编辑、格式化和 Git 操作；不得本地 cargo/dotnet 编译来替代远端验证。

### 本轮明确不做

- 不继续修代码，不启动新构建，不提交/推送当前试写改动。
- 不把未完成 QNN 草稿包装成可用插件。
- 不因接口支持 `resource` 字符串就宣称支持任何 `.onnx`、`.8bf` 或 Adobe 受保护资源。
- 不以 CPU 假输出、mock、设备描述符或 CI 绿色替代真实 NPU 推理证据。

### 推荐里程碑

| 里程碑 | 交付物 | 允许的结论 |
| --- | --- | --- |
| M0 | 工作区清点、差异审查、旧版本可复现 | 知道哪些代码已提交，哪些只是草稿 |
| M1 | v1 真实跨语言互通修复及回归测试 | C# 与 Rust 协议互通；不代表 NPU 可用 |
| M2 | 最小通用 v2 规范、schema、兼容矩阵、参考包与测试向量 | 接口可以独立实现；不等于所有模型已适配 |
| M3 | QNN 普通 ARM64 Worker、完整运行时 ZIP、配置与模型包机制 | 后端真实接入并可部署；尚需指定模型和硬件验收 |
| M4 | 宿主集成、模型/设备选择、严格回退与精度/取消/撤销 | PhotoCraft 端完整受控接入 |
| M5 | GitHub 构建与协议/宿主测试通过，指定 Snapdragon 实测通过 | 对**指定模型、版本、设备**宣称 NPU 支持 |
| M6 | Photoshop 路线的独立 PoC 与资源兼容矩阵 | 仅对经过实现和测试的桥接/插件作兼容声明 |

---

## 1. 交接基线：不要把当前目录当作干净的成品

### 1.1 已提交的已知基线

- 当前分支：`compat/host-neutral-adapters-v1-20261020`。
- 本次只读检查 HEAD：`0835a19b`。
- 旧 CI：<https://github.com/kettly1260/photocraft/actions/runs/39593924793>。
- 旧 CI 的成功只覆盖当时提交和工作流。它**没有**证明本轮草稿通过，也没有证明 C# 响应和 Rust 严格解码实际互通，更没有证明用户机器 NPU 推理成功。
- 上游 Draft PR：<https://github.com/storytold/photocraft/pull/7995>。不要在未验证时更新为“已全面兼容/已实测”。

### 1.2 当前未提交改动（交接时只读检查结果）

已跟踪但修改：

- [Windows AI 工作流](../.github/workflows/companion-windows-ai.yml)
- [通用适配器引擎命令](../crates/engine/src/adapter_cmds.rs)
- [原生适配器包管理](../crates/engine/src/adapter_packages.rs)
- [主体选择路由](../crates/engine/src/smartselect_cmds.rs)
- [兼容适配器文档](compatibility-adapters.md)
- [Windows AI 项目](../examples/npu_subject_worker/PhotoCraft.WindowsAI.csproj)
- [Windows AI 入口](../examples/npu_subject_worker/Program.cs)
- [Windows AI 协议实现](../examples/npu_subject_worker/Protocol.cs)
- [Windows AI 说明](../examples/npu_subject_worker/README.md)

未跟踪：

- [旧下载产物目录](../companion-artifact/)：不是源码，不得顺手加入提交。
- [跨语言测试草稿](../crates/plugins/tests/worker_companion_contract.rs)
- [接口审查备忘](adapter-evolution.md)：辅助设计材料，不是已实施规范。
- [Windows AI 协议测试目录](../examples/npu_subject_worker/ProtocolTests/)
- [QNN Worker 草稿目录](../examples/onnx_qnn_worker/)

**处理原则：**上述修改由上一轮越界实施产生，没有编译/测试通过结论，没有提交/推送。本计划不要求 Flash 全盘保留，也不要求直接删除。先逐文件审查，形成“保留/重做/待用户决定”表；如需回退，只回退确认属于本轮的内容，不覆盖用户或其他 agent 的工作。禁止无差别 `git reset --hard` / `git clean -fd`。

### 1.3 QNN 草稿的具体状态

已有项目、Core、Runtime 和入口文件，但交接目录没有完整测试入口、打包脚本、模型示例配置、README 或独立 QNN 工作流。测试工程配置为可执行程序，不能因存在 csproj 就推断测试可运行。

[QNN 后端草稿](../examples/onnx_qnn_worker/Runtime/QnnBackend.cs)包含真实 ORT QNN 调用和禁 CPU 回退设置，但这只是静态代码。其输出绑定、完整 native/DSP 载荷、模型限制、异常行为、API 编译兼容、许可与设备行为均须重新审查。

**不要直接在草稿上补一个上传步骤就宣布完成。**

---

## 2. 现有接口是否满足目的：审查结论

| 目标 | 当前满足程度 | 主要限制 |
| --- | --- | --- |
| 不绑定 Windows AI / 某一家模型 | 基础满足 | 核心没有 WinRT/ONNX/QNN 类型，但部分文档和示例仍带专用假设 |
| 单图滤镜、分割、去噪等复用 | 基础满足 | 一张主图＋可选选区，单结果，同尺寸，具体模型仍需 Worker |
| 多模型管理 | 不完整 | 一个 opaque resource 字符串；probe/prepare 无法指定模型，缺少资源清单、版本与缓存身份 |
| 参考图、多结果、超分辨率 | 未满足 | 没有命名输入/输出与输出几何策略；宿主限制同尺寸 |
| 跨语言/跨宿主接口 | 只有基础 | Rust trait 不是稳定二进制 ABI；需语言无关规范、schema、测试向量、独立 client 示例 |
| 普通 EXE 调用 QNN NPU | 已知可行，尚未交付验证 | 当前已提交 Windows AI Worker 不是 QNN；新草稿未构建/验证 |
| PS 插件共同使用同一计算服务 | 架构可支持，未实现桥接 | 每个宿主需要自己的像素/选区/事务桥接 |
| PhotoCraft 直接运行 `.8bf` | 未实现 | 需要真正的 Photoshop-compatible host suites、回调、参数/UI、架构适配 |
| 直接使用 Adobe 内置 Neural Filters 资源 | 未建立可行性或兼容性 | 可能依赖专有运行时、授权、保护格式、云端；没有通用独立加载器 |
| 真实 NPU 执行证明 | 没有 | 设备字段是适配器报告，不是硬件证明；没有指定模型的目标机实测证据 |

相关源码：

- [宿主中立数据契约](../crates/plugins/src/adapter.rs)
- [原生 Worker 传输](../crates/plugins/src/worker.rs)
- [通用写回与事务](../crates/engine/src/adapter_cmds.rs)
- [原生包安装](../crates/engine/src/adapter_packages.rs)

**设计判断：保留核心分层，不推倒重来；但不能把当前 v1 原样冻结为对外“万能模型/PS 兼容接口”。**

---

## 3. 目标架构与边界

```text
PhotoCraft 原生 Rust UI / CLI / Agent 命令
                  │
                  ▼
宿主桥接：快照、色彩/深度、选区、事务、撤销、权限
                  │
                  ▼
版本化、语言无关的 Adapter Effect API
                  │
        ┌─────────┼───────────────────┐
        ▼         ▼                   ▼
QNN Worker    Windows AI Worker   其他模型/过滤器 Worker
普通 ARM64 EXE 现有 MSIX 系统API    独立实现的合法加载器
        │
        ▼
ORT QNN EP → QnnHtp → Hexagon NPU
```

其他应用或 Photoshop 内的合法桥接也可以访问同一 Worker API，但它们有各自的宿主桥接。**PhotoCraft 不需要启动或自动化 Photoshop。**

### 必须保持

- 引擎不识别/解密厂商模型格式，不直接加载任意 DLL。
- 推理运行时与原生 ABI 留在外部 Worker；WASM 插件 ABI 不改成原生插件后门。
- 核心继续 Rust，禁止新增 unsafe/panic 路径或 Tauri/Electron/webview UI。
- 当前外部 C# Worker 是独立工具而非主程序内嵌依赖。其是否继续存放在 examples、是否拆到独立仓库，应按维护者对 Rust-only 规则的要求确认，不能默认为已获长期维护许可。
- 原生 Worker 有用户权限，进程分离是可靠性隔离，**不是 OS 沙箱**。
- 模型、Worker、宿主桥接分开版本化与验收；以后新增模型不应要求改 PhotoCraft 引擎。

### 暂不引入

- 通用任意张量 RPC、完整 Photoshop API 代理、远程推理服务器。
- 为一个模型提前设计所有生成式/视频/训练能力。
- 没有实测需要的零拷贝 GPU 共享、常驻服务或无限模型缓存。

---

## 4. P0：先做工作区审查，不继续扩大未验证 diff

### 工作

1. 读取 [AGENTS.md](../AGENTS.md)、架构/开发/贡献规范；确认兄弟 craftrules 是否存在。不存在就记录，勿假装遵守未读取私有规则。
2. 重新检查 HEAD、分支、未跟踪文件、其他 agent 状态；不要假设本计划的目录清点永远最新。
3. 审查本轮草稿，按独立主题拆分：协议互通、宿主路由、文件数限制、QNN、文档。
4. 不把旧下载 MSIX、CER、缓存、私有模型、证书私钥或临时文件加入源码。
5. 创建可复现基线/安全备份后再编辑，保留用户已有修改；不先批量恢复再询问。
6. 向用户确认实施范围：先 M1–M5，PS 二进制桥接作为独立后续任务，不暗中扩大为全 Adobe 兼容。

### 验收

输出基线 commit、差异归属表、保留/重做理由；无未归属文件被删除，无本地编译，无构建/推送在授权前发生。

---

## 5. P1：修复协议互通，并建立双向契约测试

### 已确认的问题

旧 C# `ResponseHeader` 含 `ready_state`、`descriptor`，包括值为 null 时也序列化。Rust 使用 `deny_unknown_fields`，只接受 `protocol/status/message/output/execution`，因此旧 C# 响应无法直接通过严格解码。

本轮草稿移除了额外字段并写了测试，但仍未经过 GitHub 编译/运行。此问题不能用旧 CI 绿色掩盖。

### 修改范围

- [C# 协议](../examples/npu_subject_worker/Protocol.cs)与[入口](../examples/npu_subject_worker/Program.cs)：v1 返回严格字段，状态细节进入有界 message。
- [C# 协议测试](../examples/npu_subject_worker/ProtocolTests/)：真实序列化产生 frames。
- [Rust 跨语言测试](../crates/plugins/tests/worker_companion_contract.rs)：生产 decoder 读取真实 C# frames。
- [Windows AI CI](../.github/workflows/companion-windows-ai.yml)：单独运行无硬件契约测试与 MSIX 打包，明确不是 NPU 测试。
- 增加反向 Rust writer → C# reader 测试，不只 C# 自造请求再自读。

### 测试矩阵

- ready / needs_model / unavailable / error / ok+mask。
- null 与字段缺失策略、错误版本、重复/未知字段、超深 JSON。
- magic、LE 长度、f32 字节序、负坐标、8/16/32 位 provenance。
- 截断、尾随字节、尺寸/样本乘法溢出、NaN/Inf、越界 mask。
- ICC、直通 alpha、选择 seed 的字段与数据长度。
- 显式运行 fixture 测试却未提供输入目录时必须失败，不能 skip 后计成功。

### 验收

1. 旧非法响应 fixture 被拒绝；修正后的真实 C# 帧被 Rust 接收。
2. Rust 产生的请求在 C# 侧解析为一致的数据。
3. 全程不调用 NPU，不把协议测试描述为推理验证。
4. v1 保持严格，不用“忽略所有未知字段”绕过错误。

---

## 6. P2：冻结最小通用 v2 规范，再开放给外部实现者

**本节字段是设计需求，不是已经存在的 API。**先完成 RFC、schema 和 golden vectors，再实施 host/worker，避免 Flash 自行添加互不兼容的字段。

### 6.1 三种版本分开

| 版本 | 管什么 |
| --- | --- |
| package manifest version | 安装、文件、架构、运行时部署与信任 |
| wire protocol version | framing、能力协商、二进制段、长度限制 |
| effect/operation semantic version | 输入输出、参数语义、几何、色彩、模型含义 |

- 显式协商能力和可接受版本；不兼容必须可解释地拒绝。
- v1/v2 独立 codec，不把旧 magic 中的字段偷偷解释成新语义。
- 不依赖“对端总是最新版本”；包含新 host/旧 worker、旧 host/新 worker、未知必需特性等矩阵。
- 协商不得执行不受信任程序；只有已安装、授权的 Worker 才能进行握手/探测。

### 6.2 模型资源与 Worker 包分离【必须】

建立不可变的模型资源记录，而不是把任意路径塞给 QNN：

- resource ID、格式、模型版本、manifest digest。
- 具名组成文件、文件大小、SHA-256 或明确选定的统一内容摘要。
- 相对路径、许可/来源、导出版本、runtime/architecture 约束。
- 参数/算子/输入输出 binding profile、预后处理规则。
- 需要的外部 ONNX 数据、tokenizer、配置、可选权重必须显式列出。

模型文件存储与可执行运行时包存储分开；不通过把所有安装上限调到数 GB 来解决。安装 Worker 不代表授权下载模型；文档也不能自行指定可执行代码。

第一版只支持用户显式安装的本地资源包，默认不实现网络下载。若后续增加下载，必须有独立来源、摘要、许可及单次/持久授权设计。

### 6.3 资源生命周期控制【必须】

probe / prepare / listResources / evict 要能指定 resource ID、operation 与 device policy。

- probe：检查明确范围的可用性，不下载、不做推理；避免每次 UI 刷新全量哈希大模型。
- prepare：显式动作，可校验资源并创建 runtime session；下载若存在须单独许可。
- run：验证实际请求与资源绑定，返回明确结果和执行信息。
- evict：移除派生缓存或模型时区分二者，不能卸载 Worker 时顺手删除用户原始文件/OS 模型。
- 状态分层：声明支持、文件存在/已校验、runtime session 可创建、实际推理已成功，不能统称 Ready 后夸大。

v1 只有 `{"protocol":1}`，无法做这些定向控制；不要在 v1 中硬塞未声明字段。过渡版如保留 package/default-model probe，必须在 UI/文档标出范围，不能当作所有模型都 ready。

### 6.4 操作描述与参数【必须】

从 provider 级单个 opaque params_schema 扩展到每 operation 的 schema ID、版本和约定 dialect。

每操作声明：

- 接受的色彩模式、源深度、alpha、HDR/range、输入尺寸和内存预算。
- 具名输入端口，最小 source + optional selection；reference、prompt 等只在确有 adapter 使用时启用。
- 具名输出端口和类型 image/mask/depth；required/optional、数量与主输出角色。
- 输出几何策略：same_geometry、scaled 或 explicit_transform；host 不支持的策略应在执行前拒绝。
- 可用设备/后端、取消/进度/tiling/session 能力。

v2 初次实现可以仅启用同尺寸图像/掩码和一个可选参考图；**协议描述可扩展不等于宿主已经支持超分/多层写回。**更复杂特性以协商 flag 关闭，拒绝而不是静默丢弃端口。

参数由宿主做结构/预算与规范化 schema 校验，Worker 再做语义校验；不能只展示表单却不拒绝无效类型/范围。

### 6.5 色彩、深度与二进制段【必须】

- 保持 native f32 样本与 source_depth provenance，不默认为 u8/sRGB。
- ICC 放到有界二进制段；避免 JSON 字节数组吞掉 1 MiB header。
- 分别限制 header、binary section、aggregate bytes、图像样本、ICC、资源清单。
- 所有长度、offset、乘法、索引和坐标先验证再分配；不可信数据不得 panic。
- alpha 明确 straight/premultiplied、是否 composite 到某底色，由模型 profile 明确说明。
- mask 是覆盖率，不是颜色；depth 需要单位/近远范围/无效值约定，缺少语义时拒绝 materialize。
- 任何模型要求 SDR/RGB 而不支持当前色彩时，转换必须明确且色彩管理正确，或报 unsupported；禁止偷偷夹掉 HDR/替换 ICC。

### 6.6 设备策略与执行报告【必须】

策略区分 auto、exact device、require kind=NPU、allowed backends、allow graph fallback。

执行报告至少可表达：

- 请求策略与实际后端、运行时版本、模型/配置 digest。
- 使用的声明设备、是否图分区/混合、fallback reason。
- 可获得时的 runtime graph-placement/profiling 摘要。
- preprocessing/postprocessing CPU 时间与模型图执行区分。

报告本身仍是 Worker/runtime 声明，非硬件远程证明。禁止“没传 device 且 allowFallback=false”被解释为“必定 NPU”；当前 v1 没有这样的含义。

### 6.7 可供其他开发者使用的交付物【必须】

- 语言无关规范、schema、状态/错误码表、版本兼容表。
- 独立 client/worker 最小样例，不依赖 PhotoCraft 文档模型。
- golden request/response fixtures 与 conformance runner。
- 示例中清楚说明共享的是 effect API，不是 Rust trait/ABI/厂商私有模型格式。
- 暂不为“看起来独立”强拆新 crate；只有真实跨项目复用需要时再抽协议 crate，且登记层级、保持 no unsafe/wasm。

---

## 7. P3：实现真正的普通 ARM64 QNN Worker

### 7.1 技术选择

- 独立 `win-arm64` 文件夹 EXE，优先复用现有请求/响应桥接，不引入主程序 WinRT/CLR 依赖。
- ONNX Runtime QNN package：调研基线 `Microsoft.ML.OnnxRuntime.QNN 1.24.4`，实施时重新核验版本/安全状态并锁定，不用浮动版本。
- 不混用另一个 CPU-only ORT 包的同名 native DLL。
- self-contained、关闭 trimming/single-file，优先可审计的完整载荷，不用打包技巧掩盖缺依赖。
- Windows AI MSIX Worker 保留为另一后端，不删除其合法包身份检查来假装实现 QNN。

### 7.2 严格 NPU 配置

C# 调研确认的调用方式（**参考，不是已编译承诺**）：

```csharp
using var options = new SessionOptions();
options.AddSessionConfigEntry("session.disable_cpu_ep_fallback", "1");
options.AppendExecutionProvider("QNN", new Dictionary<string, string>
{
    ["backend_path"] = Path.Combine(AppContext.BaseDirectory, "QnnHtp.dll"),
    ["offload_graph_io_quantization"] = "0",
});
using var session = new InferenceSession(verifiedModelPath, options);
```

- C# provider 名为 `QNN`，不要复制 Python 的 `QNNExecutionProvider` 写法。
- backend 必须是受信任载荷中的绝对 `QnnHtp.dll`，不得从请求任意指定 DLL。
- offload_graph_io_quantization 的默认行为可把图 I/O Q/DQ 交给 CPU；与严格策略一起测试。
- 失败时不得重建一个 CPU/DML session 后返回“NPU 成功”。
- 预处理/后处理在 CPU 上不等于违反模型图无 CPU fallback；报告必须区分。
- 首批选择实际可验证的固定形状 QDQ 模型，不宣称支持任意动态图/多输入模型。现代 HTP 有 FP16 选项，不要笼统写成“所有浮点都不能跑”。

### 7.3 模型 profile

先支持一个明确的分割 profile 类型，不硬编码某一网络：

- allowlisted resource ID → 已验证模型资源。
- 输入/输出名称、dtype、shape、NCHW/NHWC、通道顺序。
- 归一化公式、mean/scale/std、单位；处理已含预处理的图，避免二次归一化。
- resize/crop/letterbox、插值、padding、反变换对齐规则。
- alpha 处理、色彩约束、输出 probability/sigmoid/softmax 与前景类别。
- 数值容差、输出通道和 tensor 元素上限。

将 ORT 实际 InputMetadata/OutputMetadata 与 profile 对照；不按“第一个 output”猜模型语义。不支持的模式要清晰拒绝。

首版若只有 scalar probability/sigmoid，不得声称兼容返回二分类 logits 的 SINet。可后续增加有测试的 softmax+foreground_class，而不是错用 channel0。

### 7.4 模型来源与法律边界

候选 SINet、MediaPipe Selfie 的上游许可已调研为 MIT / Apache-2.0，但都是人像/人体分割，**不是通用对象主体模型**。具体导出产物、包装代码、转换工具、校准数据和 runtime 有各自许可。

实施者必须：

1. 选择具体模型/用途（人像还是通用主体），向用户说明区别。
2. 固定来源版本、产物 SHA、输入输出元信息和许可文本。
3. 验证导出/QDQ/设备转换，不能只下载任意 `.onnx`。
4. 不擅自下载/复用用户机器上的 Adobe/Affinity 受保护包。
5. 如果没有满足许可/质量/HTP 支持的模型，交付明确的 `needs_model` 和阻塞表，不交付假模型来伪装成功。

### 7.5 模型安全边界

- 拒绝 traversal、绝对/UNC/ADS、重解析逃逸、重复文件、超大小和 digest 不匹配。
- ONNX 可能引用外部 tensors：首版拒绝 external-data，或完整验证所有 sidecar；仅哈希主 `.onnx` 不够。
- 不用自写半套 protobuf parser 冒充完整 ONNX 安全验证；若保留草稿 parser，需有规范依据、畸形输入与边界测试。它只能缩小支持范围，不能成为沙箱保证。
- 明确 custom ops、EPContext、自定义库是否支持；首版默认拒绝未授权代码加载。
- 避免验证文件后再无约束打开另一文件；记录 TOCTOU 局限，不夸大为安全加载器。

### 7.6 完整运行时打包

逐项审计还原包与 publish 输出：

- onnxruntime / providers_shared / providers_qnn。
- QNN HTP/System/Prepare 等相关 DLL。
- 匹配架构的 HTP Stub、DSP skeleton `.so` 与 `.cat`；**Windows 包也可能需要这些文件**。
- 完整 .NET 自包含依赖，不删除看似多余的框架文件凑数量。
- 包根目录的 LICENSE、ThirdPartyNotices、Privacy、`Qualcomm_LICENSE.pdf` 等真实法律文件。
- native CRT/驱动要求单列；self-contained .NET 不等于消灭所有系统依赖。

当前包管理 `files` 是 `string[]` 相对路径，不是 `{path,hash,size}` 对象列表。host 安装记录用 **BLAKE3**；模型内容 SHA-256 与其是不同用途，文档不能写混。

基线文件数上限128，本轮草稿改512；是否保留须有实际 publish 清单与边界测试依据。每文件256 MiB、总512 MiB暂不提高。大型模型走独立 store，不塞进 runtime cap。

**许可门禁：**QNN NuGet 顶层 MIT 不代表所有 Qualcomm native 二进制也是 MIT。检查所选包中的 PDF，完整保留 notice，必要时取得许可审查结论后再发布含 runtime 的 ZIP。

---

## 8. P4：宿主通用集成与已发现缺陷

### 8.1 多后端路由与用户选择

涉及[主体选择](../crates/engine/src/smartselect_cmds.rs)、[包状态](../crates/engine/src/adapter_packages.rs)、[管理 UI](../crates/ui-egui/src/adapter_ui.rs)。

目标：

- 可明确选择 provider / model / operation / device，并可设置优先项；所有选择能通过命令驱动。
- automatic routing 匹配实际输入能力，不按字典序盲选。
- 动态就绪与静态声明分离；不在菜单绘制/空闲 frame 中启动 Worker 或下载。
- 显式指定 provider/device/require NPU 失败时 fail-closed；自动模式的经典回退由明确策略控制，结果报告实际走了什么。
- 缓存 Ready 不等于永远可用；资源/运行时/驱动变更失效，run 仍须验证。

草稿已经过滤部分能力与缓存状态，但未完成 preferred-provider、per-model readiness、参数默认值/UI，更未测试。**尤其注意：空 ICC 的图像若 Worker 要求 `colorPolicy=assume-srgb`，普通菜单调用 `{}` 不能凭空满足。应设计明确的用户色彩策略/默认配置，不得让选择主体按钮始终报错，也不得偷偷把空 ICC 当 sRGB。**

### 8.2 保留 F32 mask

当前主体路径把 F32 mask 转为 u8 Region，即使 enhanceEdge=false；通用 adapter selection 写回则保留 F32。

- 无传统边缘增强时保留浮点覆盖率，使用 float-aware combine。
- 需要 legacy u8 refinement 时只能显式走该分支并说明精度损失。
- 对 0.5001/0.5002、小于1/255的非零值测试，覆盖 replace/add/subtract/intersect、8/16/32位文档和 undo。
- 保留“主体未找到不清除旧选区”的产品语义，不能把它泛化到所有模型 mask；通用 authoritative mask 可以合法全零。

### 8.3 包校验与超时/取消

当前 verify 整文件读入 Vec，并有 status 前台校验＋job中重复校验，worker deadline 又从之后开始。

- 改为流式 hash、固定 scratch、取消/预算检查。
- 大 I/O 放进后台 job，去掉重复完整校验。
- 明确定义 timeout 覆盖文件校验、资源加载、启动、复制、推理、解码；不是只杀子进程。
- 启用/恢复时若必要的大校验不能直接异步，采用显式 preparing/validating 状态，而不是阻塞 UI。
- 完整性异常的禁用/隔离持久化；不要只在内存改 enabled。
- 清点 package directory 时先过滤/排序合法条目再施加数量上限，避免临时目录藏掉正常包。
- ExecutionAlias 路线如果要信任指定发行者，应核验 package family/publisher/version/arch，而非只信一个可变别名。

### 8.4 写回与端口

保留单事务、快照、stale-result rejection、锁、选区只混合一次、alpha正确混合。新输出类型不得绕过这些规则。

多输出/新图层/变尺寸属于后续 host capability，先定义后实现；不支持就执行前拒绝。不要用一个灰度表面伪装已经支持 depthLayer 或 super-resolution。

### 8.5 UI、翻译与视觉验证

- 所有用户可见行为有 engine command；UI只做薄封装。
- 展示模型缺失、runtime缺失、设备不可用、需要许可/准备、严格NPU失败等可操作状态。
- 新标签补真实翻译，不重复采用“移除 tl! 宏绕过翻译覆盖测试”作为长期方案。
- UI 布局通过 GitHub 生成 offscreen snapshot，实际看 PNG 后才验收；主题字体延迟生效测试要正确初始化。
- 保持默认 automation 禁止安装/准备/执行原生 adapter，直到有专用 capability policy；不可为QNN全局放开。

---

## 9. P5：测试、GitHub 构建与发布门禁

### 9.1 测试层级必须分别报告

| 层级 | 环境 | 证明什么 | 不证明什么 |
| --- | --- | --- | --- |
| T1 协议/配置单测 | GitHub Windows x64 / Linux适用部分 | bounds、字段、profile、预后处理、错误路径 | NPU或模型质量 |
| T2 C# ↔ Rust真实frames | GitHub Actions | 真实生产codec互通 | 子进程/硬件成功 |
| T3 真实Worker进程 | GitHub x64/ARM64适用环境 | 启动/退出、错误码、无模型状态、超时、取消、损坏响应 | HTP可用 |
| T4 ARM64完整publish与启动 | GitHub ARM64 runner（若可用） | 架构、载荷、程序可启动 | 云VM有Qualcomm NPU |
| T5 真实Snapdragon推理 | 用户授权目标机/受控self-hosted | 指定模型严格QNN session、推理、数值与性能 | 任意模型/全部PS插件兼容 |

### 9.2 建议工作流拆分

1. 通用 contract workflow：schema、golden、双向跨语言、host协议测试。
2. QNN workflow（建议新增 `.github/workflows/companion-qnn.yml`）：pure tests、win-arm64 publish、inventory/license门禁、无模型smoke、artifact上传。
3. Windows AI workflow：独立协议/包构建，不让MSIX要求污染QNN部署。
4. 主CI：格式、Clippy、layer、wasm、engine/ui/automation、panic_hunt、相关corpus。
5. 可选受控hardware workflow：显式dispatch、受信代码与固定模型；不得把任意PR代码送上用户NPU机器。

### 9.3 主仓库远端检查

以下命令由 GitHub runner 执行；按依赖与平台拆 job，不在本地跑：

```sh
cargo fmt --all -- --check
cargo test -p photocraft-plugins --lib --tests
cargo test -p photocraft-engine --lib --tests
cargo clippy -p photocraft-plugins -p photocraft-engine --all-targets -- -D warnings
cargo xtask layers
cargo xtask wasm
cargo test -p photocraft-engine --test panic_hunt -- --ignored
```

- UI修改再跑相关UI/字体/翻译测试及snapshot。
- 新增/调整命令按仓库要求运行parity，提交实际生成结果，不手改假计数。
- 涉及相关crate或门禁要求时跑corpus/scorecard；不得仅因模型功能就修改parity floor或宣称“补齐PS神经滤镜”。
- conformance fixture tests需显式传真实输入，缺文件失败。
- QNN pure tests中fake backend只用来验证调度/错误处理，测试名和报告标“no hardware inference”。

### 9.4 进程/异常测试

- 无模型、缺DLL、错误架构、损坏配置、hash错、外部tensor、错误shape/dtype。
- Worker非零退出、无response、格式错、超长输出、超时、取消、子进程清理。
- 文档运行中变化、撤销、锁、关闭文档，结果不误写。
- 多provider错误/未就绪在前、可用在后；显式指定失败不偷偷改provider。
- Header/ICC极限、总sample上限、路径/深JSON、非有限值。

### 9.5 产物内容

- ARM64普通Worker ZIP，版本与commit。
- runtime完整文件清单与摘要、实际Native/DSP/法律文件。
- adapter manifest、model schema与可核验示例（占位示例必须明显不可运行）。
- 安装/启用/准备/选择模型/设备/卸载说明，错误排查。
- test evidence：run URL、job、scope、fixture、日志、缺失硬件项。
- 不包含私有模型、证书私钥、用户照片、未获准再分发的SDK或权重。

---

## 10. P6：目标 Snapdragon 设备验收

本节是未来验收步骤，**不是要求本轮现在操作用户机器**。

前提：用户同意运行指定artifact、runtime和模型许可审查完成、记录设备/OS/驱动版本。

1. 下载与commit匹配的普通ARM64 ZIP到新目录，不先覆盖当前可用PhotoCraft/旧伴随包。
2. 验证摘要、native载荷、许可文件；确认没有MSIX安装步骤。
3. 显式安装模型资源包，记录模型/配置digest和预后处理版本。
4. probe无下载/无推理；prepare明确目标resource/device并创建严格session。
5. 严格NPU run实际产生mask；同时记录provider配置、执行报告和可获得的runtime placement/profiling。任务管理器NPU图可辅助，不能单独当完整证据。
6. 与可信CPU参考结果比较（仅测试oracle，不是生产静默回退）；报告误差/质量而非要求量化模型逐位相同。
7. 用许可明确的公开测试图覆盖人像、复杂边缘、非人主体、透明、尺寸/色彩/深度；人像模型对非人失败是已知能力限制。
8. 错误driver、不支持graph、模型被改动、取消/超时：错误清晰，文档无部分写回，不返回“NPU成功”。
9. 冷启动/热启动、模型加载、预处理/图执行/后处理/IPC、内存峰值分别记录；涉及24–36MP时用release并有端到端预算。

验收输出必须是“模型A、artifact版本B、设备C、驱动D、严格策略E通过”，不能写成“所有骁龙/模型/PS插件支持”。

---

## 11. P7：Photoshop兼容路线，独立立项而非混入QNN

### A. 模型计算共同使用【首选长期方向】

PhotoCraft与其他宿主实现各自bridge，使用同一effect/resource协议和worker。若在Photoshop内部使用，可另做合法UXP/native插件桥接，不使PhotoCraft依赖Photoshop运行。

验收：同一输入/模型/profile/digest在两个宿主得到可比较结果，正确处理各自色彩、alpha、16位编码、选区与事务。未经实现不得称已有PS bridge。

### B. PhotoCraft直接运行第三方 `.8bf`【另一个兼容项目】

需要真实loader与suite/callback实现、参数持久化/对话框、tile/buffer/channel/range转换、架构匹配、selection compositing、SDK许可与逐插件兼容矩阵。

首个PoC选择许可和行为明确的简单插件，不从复杂受保护Neural Filters开始。外部x64桥接可隔离架构差异，但不是自动实现；主进程不新增unsafe加载任意库。

### C. Adobe内置Neural Filters/离线资源【先调研，未承诺】

先确认具体资源格式、实际runtime、授权/独立使用条件、是否云端依赖，再决定是否有合法可实现的loader。

- 没有loader就标unsupported。
- 不通过改扩展名、移除保护、复制Adobe私有SDK/模型把资源“变成ONNX”。
- 接口开放不提供或暗示解密/授权绕过能力。
- 若无法独立使用，可用授权允许的替代模型实现相似功能；必须说明“功能替代”而非原插件兼容。

---

## 12. 文件级实施地图与任务拆分

| 工作包 | 主要位置 | 交付/边界 |
| --- | --- | --- |
| 协议v1修复 | [Windows AI实现](../examples/npu_subject_worker/)、[Rust互通测试](../crates/plugins/tests/worker_companion_contract.rs) | 只修互通，不夹带宽松decoder |
| 通用v2规范 | [现有接口文档](compatibility-adapters.md)、建议新schema/spec目录 | RFC先于代码；不把设计写成已有功能 |
| Core契约与wire | [adapter.rs](../crates/plugins/src/adapter.rs)、[worker.rs](../crates/plugins/src/worker.rs) | 版本、ports、budgets、errors、control；独立测试 |
| 资源管理/包 | [adapter_packages.rs](../crates/engine/src/adapter_packages.rs)、建议独立model-resource模块 | 运行时包与不可变模型store分开；新命令注册/授权 |
| QNN Worker | [QNN草稿目录](../examples/onnx_qnn_worker/) | 重审草稿，真实runtime、profile、测试、打包、README |
| 通用/subject写回 | [adapter_cmds.rs](../crates/engine/src/adapter_cmds.rs)、[smartselect_cmds.rs](../crates/engine/src/smartselect_cmds.rs) | 能力路由、F32、事务、strict policy |
| UI | [adapter_ui.rs](../crates/ui-egui/src/adapter_ui.rs)和对应状态/翻译 | 命令驱动、model/device/operation状态，无idle副作用 |
| 安全/automation | [automation工作区](../crates/automation/src/workspace.rs)等实际授权入口 | 默认deny不倒退；专项capability测试 |
| CI | [现有WindowsAI工作流](../.github/workflows/companion-windows-ai.yml)、建议新增QNN/contract工作流 | 编译只在GitHub；证据按测试层级分类 |

### 推荐提交粒度

1. v1互通fix + 真实fixture回归。
2. 协议v2规范/schema/vectors（评审后冻结）。
3. v2 core codecs/control + conformance。
4. 模型资源store/命令/授权 + tests。
5. 独立QNN后端与pure tests + runtime打包门禁。
6. host路由/浮点mask/取消预算 + tests。
7. UI/翻译/snapshot。
8. 目标硬件验收资料与文档更新。

每个提交尽量可独立验证；不要一次提交协议、runtime、UI、SDK与未验证模型数千行后才第一次跑CI。

### 并行工作规则

- 可并行：规范/schema、QNN独立目录、契约测试；先统一已冻结的wire草案。
- 不可并行盲改：shared commands/state/manifest/worker codec。
- 每agent明确文件owner；只有主agent做共享注册表的小改动。
- 已启动background job必须收集结果或明确取消，不反复sleep轮询。
- 读-only研究子agent不能临时当实现agent使用；另起有明确写权限任务。

---

## 13. 风险、阻塞与需要用户/维护者确认

| 风险/问题 | 处理原则 | 是否阻塞所有工作 |
| --- | --- | --- |
| 没有已验证的通用主体分割QNN模型 | 先确认人像/通用主体目标，再选artifact；不以人像模型冒充通用主体 | 不阻塞协议/后端；阻塞“通用主体可用”验收 |
| Qualcomm runtime再分发条款 | 检查实际选定包PDF，保留notices；必要时分离用户自行获取组件 | 不阻塞源码；阻塞含该runtime的公开二进制发布 |
| 无可用NPU runner | GitHub先做无硬件测试，最终用户授权设备验收 | 不阻塞编译；阻塞真实NPU支持声明 |
| v2范围过大 | 必需：resource/control/version/device/budgets；高级ports/geometry/常驻按capability分期 | 不允许用“大而全”拖住基本互通 |
| C# companion与Rust-only维护政策 | 保持外部独立；询问维护者目录/仓库归属 | 不应把CLR塞主程序作为绕行 |
| PS资源格式/许可未知 | 单独兼容性调研，未通过就unsupported | 不阻塞普通开放模型和QNN |
| 现有草稿不完整 | 先M0审查；允许按规范重写而不是维护错误假设 | 不应作为成品发布 |

先交可执行的接口/后端里程碑，再谈具体PS资源，不需要用户先解决所有远期问题才能推进。

---

## 14. 最终完成定义（DoD）

### 通用接口

- [ ] 不依赖单一系统AI/模型/宿主类型。
- [ ] 版本/schema/错误/二进制限制清楚，可由另一语言独立实现。
- [ ] 资源、operation、device能显式选择与定向probe/prepare。
- [ ] 多模型配置无须改引擎，内容摘要/许可/运行时约束可追溯。
- [ ] 不支持的ports/geometry/runtime在执行前解释性拒绝。
- [ ] automation、native trust、WASM隔离不退化。

### QNN

- [ ] 普通ARM64 EXE，无MSIX必需条件；完整native/DSP与法律文件。
- [ ] 真实QNN HTP路径，严格CPU图回退策略，失败不悄悄换后端。
- [ ] 至少一个指定合法模型artifact完成profile与实际推理验收。
- [ ] 不把mock/probe/provider列表算作NPU执行成功。
- [ ] 色彩/alpha/HDR/shape/pre-postprocess受控并有测试。

### 宿主与验证

- [ ] 真实双向跨语言协议测试、进程异常测试、核心CI通过。
- [ ] 多provider/资源路由可解释、显式选择严格、浮点mask不无故降精度。
- [ ] 超时/取消含大I/O，UI无隐式启动/下载，文档写回事务安全。
- [ ] 每项结论附commit、run URL、测试scope；硬件验证单列。
- [ ] 文档/PR没有“支持任意Adobe模型/PS神经插件”或“永不崩溃/100%全兼容”的无证承诺。
- [ ] 用户知道当前可用什么、未完成什么、下一步是否需其设备/模型授权。

---

## 15. 可直接复制给 Flash 的启动提示词

> 请按《PhotoCraft 通用兼容适配器与 QNN 后端：Flash Agent 实施计划书》执行。先读取AGENTS.md并完成M0工作区审查，报告当前HEAD和未提交草稿的保留/重做清单，再从P1真实C#↔Rust互通开始。现有未提交改动没有通过验证，不能直接当基线成品；不要删除用户文件或提交旧下载产物。保留PhotoCraft原生Rust/WASM安全边界，所有cargo/dotnet编译、测试、打包仅在GitHub Actions运行。本地只读/编辑/格式化。QNN采用普通ARM64外部Worker，不以移除WindowsAI的MSIX检查冒充新后端；严格HTP、完整native/DSP载荷、资源摘要与许可。冻结最小通用v2规范后再扩展资源生命周期/设备策略；不承诺任意ONNX或Adobe Neural Filters兼容。每个阶段完成提供文件diff、CI证据、可验收结论和仍未验证的项，未经用户授权不扩展到PS二进制桥接或用户机器操作。

---

## 16. 参考资料（实施前核验固定版本）

- [ONNX Runtime QNN EP](https://onnxruntime.ai/docs/execution-providers/QNN-ExecutionProvider.html)
- [QNN NuGet](https://www.nuget.org/packages/Microsoft.ML.OnnxRuntime.QNN/1.24.4)
- [固定版本C# SessionOptions源](https://github.com/microsoft/onnxruntime/blob/v1.24.4/csharp/src/Microsoft.ML.OnnxRuntime/SessionOptions.shared.cs)
- [固定版本native NuGet打包源](https://github.com/microsoft/onnxruntime/blob/v1.24.4/tools/nuget/generate_nuspec_for_native_nuget.py)
- [Windows ImageForegroundExtractor及其MSIX要求](https://learn.microsoft.com/en-us/windows/ai/apis/image-foreground-extractor)
- [Qualcomm SINet模型说明](https://aihub.qualcomm.com/models/sinet)
- [SINet上游许可](https://github.com/clovaai/ext_portrait_segmentation/blob/master/LICENSE)
- [Qualcomm MediaPipe Selfie说明](https://aihub.qualcomm.com/models/mediapipe_selfie)
- [MediaPipe上游许可](https://github.com/google-ai-edge/mediapipe/blob/master/LICENSE)

这些链接支持技术路线与调研结论，不代替具体下载artifact的许可/哈希/shape/真实设备验证。
