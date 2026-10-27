# PhotoCraft Adapter v2 待冻结决策清单（Freeze Decision Points）

> **规范状态**：评审草案（Review Draft 2.0.0-draft-review-1）  
> **注意**：本文件为 M2 阶段待冻结决策清单，需经用户与架构维护者正式评审确认后方可合入主线并标记为 Frozen。**当前绝不写成 Frozen**。

---

## 决策项 1：物理帧分段描述符表（Section Descriptor Entry）格式

- **背景**：在 `PCADREQ2` / `PCADRES2` 物理帧中，紧随 UTF-8 JSON 头部之后为分段数 `section_count: u32le` 以及分段寻址表。
- **当前设计方案（推荐）**：
  - 采用固定 16 字节对齐条目：
    ```text
    struct SectionEntry {
        kind: u32le,      // 1 = icc, 2 = f32
        reserved: u32le,  // 必须全 0，预留用于 64位寻址扩展或对齐标志
        offset: u32le,    // 相对 Payload 区域起始地址的字节偏移量（4字节对齐）
        length: u32le,    // 数据区字节长度
    }
    ```
- **备选方案**：变长编码（如 Protobuf varint 或 MessagePack 分段）。
- **权衡与影响**：
  - 固定 16 字节条目具有 $O(1)$ 的内存映射和常数时间边界计算能力，便于在无 GC 的 Rust、C# 及 C++ 原生代码中以确定性指针算术操作，杜绝变长解析可能带来的整数溢出攻击。
  - 最大分段数限制为 64，总表长度至多 1,024 字节，空间浪费几乎为零。
- **提议结论**：**建议冻结固定 16 字节结构**。

---

## 决策项 2：掩码数值域与边界约束（Mask Value Range Contract）

- **背景**：在主体分割任务中，模型输出可能为未归一化的 Logits、Sigmoid 后的概率浮点数、或 $[0, 255]$ 区间整数。
- **当前设计方案（推荐）**：
  - 掩码输出端口（`kind: "mask"`）严格定义为单通道 32 位浮点数（IEEE 754 LE），数值区间必须标准化为 $[0.0, 1.0]$：
    - `0.0`：绝对背景（完全不选中）；
    - `1.0`：绝对主体（完全选中）；
    - `(0.0, 1.0)`：半透明边缘/发丝羽化值。
  - **Worker 责任**：模型本身的后处理（Sigmoid、阈值截断或除以 255）由 Worker 内部完成后写入 Payload，禁止向宿主吐出原始 Logits。
  - **宿主责任**：宿主收到 F32 浮点数组后，当 `enhanceEdge: false` 时直接保留高精度 F32 掩码，若需要栅格化为 8 位选区时使用严谨的向下取整或四舍五入。
- **提议结论**：**建议冻结归一化 $[0.0, 1.0]$ IEEE 754 浮点数值域**。

---

## 决策项 3：生命周期会话预热与超时边界（Pre-warming & Timeout Model）

- **背景**：高通 NPU（QNN HTP）在初次加载模型上下文（`.bin` 权重加载、HTP 图编译分配硬件资源）时耗时可能长达数秒，若放在用户点击“选择主体”时执行会导致界面卡顿。
- **当前设计方案（推荐）**：
  - 将 `probe` 与 `prepare` 的职责严格正交化：
    1. `probe` 保持无副作用的只读探测（仅做文件和设备存在性核验，毫秒级响应，**严禁分配会话**，就绪状态最高只返回 `files_verified`）；
    2. `prepare` 显式承担耗时的会话预热职责（返回 `session_created`），宿主可在插件启用或进入编辑模式时异步触发；
    3. 若宿主未调用 `prepare` 直接发起 `run`，Worker 必须支持懒加载（Lazy Initialization），但此时受 `run` 请求自身的执行超时预算约束。
- **超时边界**：
  - `probe`：固定硬上限 5,000 ms；
  - `prepare`：允许较长会话构建时限，默认上限 30,000 ms；
  - `run`：由请求中的 `execution_limits.max_idle_ms` / `timeout` 显式控制（1,000 ms .. 60,000 ms）。
- **提议结论**：**建议冻结此 probe/prepare/run 职责切分与超时模型**。

---

## 决策项 4：远期特性的能力协商与显式拦截（Capability Rejection Policy）

- **背景**：规范需要兼顾长期扩展性，但本阶段聚焦于 Windows ARM64 QNN 单次图像分割。
- **当前设计方案（推荐）**：
  - 核心协商机制引入 `supported_features` 与 `required_features`；
  - 明确将以下远期高级特性从最小冻结集合中剥离，并在 Worker 中默认不声明、在收到请求时以 `unsupported_feature` 拒绝：
    - `streaming-frames`（实时视频流传输）；
    - `dynamic-geometry`（非等比或任意尺寸动态图变形）；
    - `daemon-service`（多租户常驻 RPC 守护进程）；
    - `photoshop-plugin-bridge`（Photoshop 8BF/Automation 滤镜桥接）。
- **提议结论**：**建议冻结“最小必须集 + 未实现特性显式报错拒绝”策略**，绝不让远期特性拖慢 M2 落地。
