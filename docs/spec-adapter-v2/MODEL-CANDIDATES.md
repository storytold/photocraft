# PhotoCraft QNN 适配器：分割模型候选与许可资料核实报告

- **核实日期**：2026-10-25
- **规范版本**：Adapter v2 审查草案（Review Draft 2.0.0-draft-review-1）
- **核实原则与红线约束**：
  1. **禁止虚构任何产物**：绝不虚构 digest、张量 shape、量化参数或硬件成功运行记录；
  2. **三级事实定性（Fact Classification）**：所有技术陈述严格划分为**官方既成事实（Official Facts）**、**推论（Inferences）**与**待验证未知项（Unknowns）**；
  3. **禁止无依据宣称 QNN 兼容**：在缺乏真实编译生成的 QNN Context Binary / QDQ ONNX 产物以及真机运行日志之前，**严禁宣称任何模型已具备 QNN 兼容性**，所有模型在 QNN 上的运行能力一律定性为“**未验证（Unverified / Open）**”；
  4. **严格区分任务领域**：严格区分**通用主体分割（General Subject Segmentation）**与**人像抠图（Portrait Matting）**，严禁混淆；
  5. **训练集许可污染深度核查（Dataset-Level License Contamination）**：严禁仅看代码开源许可证（如 MIT/Apache-2.0），必须深入审查训练集授权。凡包含非商用数据集（如 DIS5K、DUTS、Adobe Matting 等）训练的权重，一律严正标注商用受限；
  6. **历史 CI 证据严格隔离**：严正澄清历史 CI 运行记录，严禁将 QNN 伴生代码测试与 Adapter v2 规范套件混为一谈。

---

## 一、候选模型对比总览与许可污染定性

| 候选模型 | 任务定位 | 代码许可证 | 预训练权重许可与训练集污染审查 | 官方/社区导出产物 | 输入 Shape / Layout | 默认分辨率 | 通用主体支持度 |
|---|---|---|---|---|---|---|---|
| **RMBG-1.4** (BRIA AI) | **通用主体分割** (General Subject) | **CC BY-NC 4.0** | **明确非商用**（官方权重与代码同属 CC BY-NC 4.0；商用须向 BRIA 购买商业授权） | 官方 PyTorch、社区 ONNX (FP32, ~176 MB) | `[1, 3, 1024, 1024]` NCHW, float32 | 1024x1024 | **完全支持**（人、动物、商品、车辆、文本等） |
| **BiRefNet-general** (Zheng et al.) | **通用主体与显著目标** (DIS / General) | **MIT** | **存在严重训练集非商用污染**（代码为 MIT，但官方通用权重在 DIS5K、DUTS-TR 等数据集上训练，包含学术非商用协议，商用存在合规风险） | 官方 PyTorch、官方/社区 ONNX (FP32, ~160 MB) | `[1, 3, 1024, 1024]` (支持 512x512) NCHW, float32 | 1024x1024 | **完全支持**（高分辨率显著主体） |
| **MODNet** (Zhang et al.) | **仅限人像/人体抠图** (Portrait ONLY) | **Apache 2.0** | **存在严重训练集非商用污染**（代码为 Apache-2.0，但官方权重使用 PPM-100 及 Flickr/Adobe 非商用人像数据集，非商用且不能用于通用主体） | 官方 PyTorch、官方 `modnet.onnx` (~25 MB) | `[1, 3, 512, 512]` NCHW, float32 | 512x512 | **不支持通用主体**（对非人像物体几乎失效） |

---

## 二、候选模型深度事实核实与边界分析

### 1. 候选一：RMBG-1.4 (BRIA AI) —— 通用主体分割精度参考

#### (1) 官方既成事实（Official Facts）
- **发布方与发布时间**：BRIA AI 于 2024 年 1 月公开发布；
- **官方仓库与代码**：`https://huggingface.co/briaai/RMBG-1.4`；
- **开源许可证与商用约束（保持未核验，不作许可批准）**：**CC BY-NC 4.0**（明确规定仅限非商业科研与评估使用；争议结论严格保持为未核验状态，本工程及技术团队不作任何许可合规判定或商业使用批准，商业集成必须由法务评估并取得正式商业授权）；
- **任务目标**：通用主体背景移除（Background Removal across multiple domains）；
- **官方张量规格**：
  - 输入：`[1, 3, 1024, 1024]`，`float32`，`NCHW` 布局，RGB 色彩空间；
  - 预处理算式：$X_{norm} = (X / 255.0 - 0.5) / 1.0$，输出动态范围为 `[-0.5, 0.5]`；
  - 输出：`[1, 1, 1024, 1024]`，单通道掩膜，经 Sigmoid 输出 $[0.0, 1.0]$ 连续概率分布。

#### (2) 推论与理论依据（Inferences）
- **架构算子推论**：模型骨干基于 IS-Net 深度监督架构，主要算子为 Conv2D、BatchNorm（可完全折叠进卷积权重）、ReLU 激活与 Bilinear Resize；
- **SDK 理论匹配**：Qualcomm QNN SDK 2.19+ 官方文档中列出了上述算子的 HTP 支持，且输入输出尺寸固定（Static Shape），符合 HTP 对非动态图的要求；
- **部署模式推论**：因 CC BY-NC 4.0 约束，该官方权重不可直接打包为 PhotoCraft 商业发行包默认预装，但可作为用户自行安装的外部受控资源或企业客户自备模型（BYOM）。

#### (3) 待验证未知项（Unknowns）
- **QNN 算子方言兼容性未知**：ONNX 导出包含 `Resize` 算子（`coordinate_transformation_mode: half_pixel`），未在真实 `QnnHtp.dll` 上验证是否会触发算子回退到 CPU（Fallback）；
- **量化精度损失未知**：HTP 必须依靠 INT8/INT16 量化释放峰值算力，尚未生成真实校准集 QDQ 模型，量化后在细微毛发、半透明物体的边缘分割精度退化程度完全未知；
- **TCM 紧密耦合内存分配未知**：1024 静态分辨率激活张量较大，是否会触发片上 TCM 溢出及主存频密换页导致的延迟未知；
- **真实硬件推理延迟未知**：无任何在 Snapdragon X Elite 或 8cx Gen3 上实测的端到端时延数据。

---

### 2. 候选二：BiRefNet-general (Zheng et al.) —— 商业友好通用主体候选

#### (1) 官方既成事实（Official Facts）
- **发布方与出处**：Zheng et al.，发表于 CAAI 2024 / CVPR 研讨会；
- **官方仓库**：`https://github.com/ZhengPeng7/BiRefNet`，Hugging Face `ZhengPeng7/BiRefNet`；
- **代码开源许可证**：**MIT 许可证**；
- **训练数据集与许可污染审查**：
  - 官方通用主体预训练权重 `BiRefNet-general` 深度依赖 DIS5K 训练集（DIS-TR）及 DUTS 训练集；
  - **DIS5K 许可**：由学术团队基于网络爬取构建，协议明确限定为仅供学术研究使用（Research Only / Non-Commercial）；
  - **DUTS 许可**：明确限定仅用于学术科研目的（Strictly for non-commercial research purposes）；
  - **合规争议定性（保持未核验，不作许可批准）**：尽管代码为 MIT，官方直接发布的权重文件存在训练集协议污染与商用受限争议。**争议结论严格保持为未核验（Unverified）状态，本工程及技术团队不作任何法律合规结论判定或商业使用许可批准**。在法务合规部门出具正式法律审查结论前，一律视为未批准；若需商业分发，须由法务核验原始数据集授权或在具有完整商用授权的数据集上自研训练。
- **官方张量规格**：
  - 输入：`[1, 3, 1024, 1024]`（或 `512x512`），`float32`，`NCHW`，RGB；
  - 预处理：ImageNet 标准归一化（$mean=[0.485, 0.456, 0.406], std=[0.229, 0.224, 0.225]$）；
  - 输出：`[1, 1, 1024, 1024]`，Sigmoid 概率值 $[0.0, 1.0]$。

#### (2) 推论与理论依据（Inferences）
- **二次训练合规路径**：MIT 代码允许团队在内部清洗的商用数据集上自主微调或重新训练，生成完全拥有自主商用版权的模型权重；
- **结构推论**：提供 ResNet 与 Swin-Transformer 两种骨干，其中 ResNet-50 骨干在移动端与边缘 NPU 上的算子映射成熟度理论上优于 Swin 注意力机制。

#### (3) 待验证未知项（Unknowns）
- **QNN 编译可行性未知**：BiRefNet 包含高频引导重建模块与密集双向连接分支，QNN 图优化器能否顺利将所有子图切分并固化入单个 HTP 上下文二进制完全未知；
- **量化敏感度未知**：双向引导模块跨层特征融合是否会在 INT8 量化下出现数值溢出或截断未知；
- **真机运行状态未知**：尚未在物理 NPU 上产生任何 Profile 日志。

---

### 3. 候选三：MODNet (Zhang et al.) —— 仅限人像抠图的定性澄清

#### (1) 官方既成事实（Official Facts）
- **官方仓库与出处**：`https://github.com/ZKD-YML/MODNet` (AAAI 2022)；
- **代码开源许可证**：**Apache 2.0**；
- **训练数据与许可污染审查**：
  - 官方权重完全采用人像抠图数据集（Matting-Human-Datasets）训练，包含 PPM-100（部分为 Flickr 非商用授权照片）及 Adobe Deep Image Matting（明确仅供非商用研究）；
  - **合规争议定性（保持未核验，不作许可批准）**：官方权重存在使用受限与版权争议，**争议结论严格保持为未核验（Unverified）状态，本工程及技术团队不作任何许可合规判定或商业使用批准**；若商业落地，须在具备合法授权的人像标注数据集上重新训练，并经法务审计通过；
- **任务目标定位**：核心目标是实时人像（Portrait / Human Matting），利用了显著的人体形态结构与面部先验；
- **输入规格**：`[1, 3, 512, 512]`，`float32`，`NCHW`，RGB，归一化到 `[-1.0, 1.0]`。

#### (2) 边界判定与严正澄清
- **严正澄清**：在 PhotoCraft 的“主体选区（Select Subject）”通用功能中，**严禁使用 MODNet 冒充通用主体分割模型**；
- **失效场景**：当输入图片为主体不是人类的照片（如宠物猫狗、汽车、电商静物、花卉等）时，MODNet 由于缺乏相应类别的语义表征，将输出全零黑图或严重混乱的伪影边缘；
- **架构归位结论**：MODNet 仅能作为后续独立的操作扩展项（如专用 `operation: select-portrait`），绝不能接入通用的 `operation: select-subject`。

---

## 三、模型存储与 Worker 交付物理隔离规范

依据 `docs/flash-agent-adapter-qnn-plan.zh-CN.md` 架构约束，模型资源定义与执行器代码必须在物理和逻辑上彻底解耦：

1. **不可变模型记录（Immutable Model Record）**：
   - 包含独立三元组：`id`（如 `com.photocraft.models.birefnet-general`）、`version`（如 `1.0.0`）、`manifest_digest`（64位小写十六进制 SHA-256）；
   - 模型清单记录所有权重文件、配置文件、许可证文件的相对路径、大小与哈希；
   - 模型包体积上限 32 GiB，单个文件上限 16 GiB，严禁通过膨胀 Worker 可执行程序配额来容纳模型。
2. **Worker 可执行包清单（Worker Package Manifest）**：
   - 独立 `manifest_version: 2`；
   - 仅包含 Worker 可执行二进制、依赖的动态链接库（如 `QnnHtp.dll`）及运行时元数据；
   - 单文件上限 256 MiB，包总体积上限 512 MiB；
   - Worker 严禁将模型文件内嵌或私有化硬编码。

---

## 四、CI 证据定位与严格隔离说明

- **Run 40075123796（commit `561a0e09`）定位**：
  - 该次运行系归档于 `qnn-review-20261023/FINAL-ARCHIVE.zh-CN.md` 中的 **QNN 独立伴生代码库（`tests/qnn_workflow_review/`）** 的 T1（纯合约校验）与 T4（ARM64 烟雾测试）证据；
  - **严正声明**：该次运行属于此前独立门禁里程碑，**绝非** `docs/spec-adapter-v2/` 二进制向量生成或一致性套件的执行记录；两者物理隔离，不可混淆。
- **Run 40360311553（commit `5c10d371`）定位**：
  - 触发工作流：`.github/workflows/adapter-spec.yml`（分支 `compat/host-neutral-adapters-v1-20261020`）；
  - 产出工件：`adapter-v2-candidate-vectors`（ID `12668240638`，大小 8,126 字节，归档容器哈希 `sha256:cc2d7258230160c2e897c83383863da39a6210c90e86ce57cac597034c997fce`，保留期 30 天）；
  - **范围界定**：该次运行仅为规范候选向量与草案脚本的狭窄运行证据，固定基准源与 8 个全量记录、真实目标阶段断言、严格边界正例可达性及会话状态机映射仍保持 OPEN，绝不作为 M2 规范冻结通过依据。
- **Run 40360311691（commit `5c10d371`）定位**：
  - 触发工作流：`.github/workflows/ci.yml`（因分支匹配 `compat/**` 自动触发）；
  - 平台覆盖事实：Linux (`ubuntu-latest`) 与 Windows (`windows-latest`，单元测试与集成测试两路并行)；macOS 仅在 `release.yml` 构建，不参与 PR/push 测试；
  - 运行结果：`test (windows-latest, integration)` 耗时约 45 分钟顺利通过；但整体因预存在的代码格式（Format）、Windows 单元测试及语料库下载（Corpus tests）三个历史问题失败，根因继续保持 OPEN，不擅自修改主 CI。

---

## 五、执行状态与阶段结论

依据用户制定的严格状态标准（Written / Delivered / CI-verified / Unfinished），当前状态判定如下：

| 项目 / 维度 | 当前判定状态 | 具体事实说明 |
|---|---|---|
| **模型候选调研与技术报告** | **Delivered (已交付)** | 完整梳理 RMBG-1.4、BiRefNet、MODNet 事实/推论/未知项，严正标注训练集非商用许可污染与人像/通用边界，模型争议结论严格保持未核验且不作许可批准 |
| **张量规范与预处理契约** | **Delivered (已交付)** | 严格明确 `[1,3,H,W]` NCHW、RGB 通道、浮点归一化及输出掩膜区间 |
| **QNN Context Binary 编译** | **Unfinished (未开始)** | 无真实 QDQ ONNX、未进行 Qualcomm SDK 远程交叉编译 |
| **Snapdragon 物理 NPU 推理** | **Unfinished (未完成)** | 真实硬件推理尚未进行，严禁使用任何模拟器或伪造日志冒充成功 |
| **Adapter v2 候选向量 CI 验证** | **CI-verified (狭窄草案证据)** | Commit `5c10d371` 已在 GitHub Actions Run 40360311553 成功验证候选向量生成与草案套件；M2 冻结继续保持 OPEN |
| **M2 规范冻结结论** | **Unfinished (OPEN)** | 固定基准校验、严格目标阶段断言、边界正例可达性、进程内会话契约等待最终核定，绝不根据草案绿灯宣布 M2 冻结或进入 M3–M5 |
