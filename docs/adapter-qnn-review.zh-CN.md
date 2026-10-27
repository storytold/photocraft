# Adapter / QNN 复核记录（2026-10-22）

## 基线与范围

- 复核基线：`0835a19b7a1695952ee3e73e7c6d01d185182949`，贡献分支 `compat/host-neutral-adapters-v1-20261020`。
- 用户授权：重新复核，修复后提交/推送该贡献分支，启动 GitHub Actions 验证。
- 本地不执行 cargo/dotnet 编译、测试或产品打包；不修改系统/用户持久环境变量。`../craftrules` 不存在，不能声称已读取该私有规则。
- 保留既有草稿和下载目录，不提交旧 MSIX/CER、模型、证书私钥或本地备份。原有补丁仅包含已跟踪文件，**不是所有未跟踪草稿的完整备份**。

## 纠正此前结论

此前会话将 P0–P5 全部标为完成，超出了实际证据。以下为复核时的准确状态，不能因工作流文件存在就宣称验证通过。

| 里程碑 | 当前可交给远端验证的内容 | 尚未完成/不能宣称 |
| --- | --- | --- |
| M1 | C# 响应去掉未知字段；真实生产 codec 双向 fixture 链路；缺失输入负向门禁 | 需要匹配提交的 CI 成功结果；不证明 NPU |
| M2 | 未评审的 v2 RFC、schema 和明确标为 synthetic 的 JSON 示例及 schema 复核脚本 | **没有冻结规范**、v2 core codec、二进制 golden vectors、独立 client conformance 或模型 store |
| M3 | 独立 ARM64 QNN 源码、硬件中立测试、完整 publish/inventory 与许可发布限制 | 没有已验证的模型 artifact、许可发布审查结论或 Snapdragon 推理验收；不得公开发布 runtime ZIP 冒充成品 |
| M4 | 自动路由能力/缓存过滤和文件数边界测试；原有事务保护保留 | 主体 F32 mask 仍降为 u8；大 I/O 流式 hash/端到端 timeout/取消、per-model readiness、默认色彩策略/UI 未完成 |
| M5 | GitHub Actions 验证入口 | 只有实际完成且匹配 SHA 的 run 才能提供证据；T5 硬件验收独立且未执行 |

## 本轮发现与修复

1. 原双向 workflow 第二次启动 C# 测试时，会向同一目录重新写 response fixture；生产 writer 拒绝覆盖，导致必然失败。改为互斥的 `--fixtures`（生成）与 `--verify-rust`（只读验证）模式。
2. 原 Rust→C# 只检查 RGB 首像素和选区长度，并未覆盖所宣称的 alpha。现在覆盖 8/16/32 来源深度、RGBA 直通 alpha、负坐标、全部源/选区 f32le 样本，以及仅用于 transport 的 synthetic ICC header。未运行 WIC 转换或推理。
3. C# reader 现在拒绝未知字段及缺失的必填图像 metadata，control 不允许添加 null 字段偷扩 v1；保留可选 resource/device/selection 的 missing/null 策略并加入回归。
4. QNN 原打包脚本生成错误的扁平 manifest，并对 native 缺失只 warning；复核要求按实际宿主结构生成 manifest，完整核验载荷、文件数/字节预算与法律文件。未经许可审查，不上传 runtime ZIP。
5. v2 原文使用“Frozen Baseline / golden”等无证据措辞，且示例虚构 SINet digest/shape 与 NPU 成功；改为未实现草案和 synthetic 示例，真实模型必须另外固定来源、许可、形状与摘要。
6. `.gitignore` 的 `*.patch` 过宽，改为只忽略已知备份和产物，避免隐藏其他贡献者补丁。

## 远端证据如何阅读

- Windows AI companion workflow：双向生产 codec fixture 与缺失输入门禁，以及独立 MSIX 构建/签名检查；没有系统模型下载或 NPU 推理。
- QNN workflow：纯测试、ARM64 publish、实际运行时文件 inventory/法律文件证据，以及可用 runner 上的无模型进程 smoke；没有真实模型图执行。
- Adapter v2 draft schema workflow：JSON Schema/正反例检查；不是 binary wire conformance。
- 主 CI：通过贡献分支 push 启动既有格式、Clippy、测试、layer、wasm 等门禁。完整结果按 run/job 实际状态报告，不能以其中一个 job 代替全绿。

CI URL 与最终提交 SHA 由本轮执行结果另行记录；本文件不预填虚构 run 或通过结论。
