#!/usr/bin/env python3
"""
Generate and verify real Adapter v2 binary golden test vectors.

Binary Wire Protocol Layout (PCADREQ2 / PCADRES2):
  Offset                Length        Type      Description
  0                     8             bytes     ASCII Magic: b"PCADREQ2" or b"PCADRES2"
  8                     4             u32le     header_length (bytes of UTF-8 JSON)
  12                    L             bytes     UTF-8 JSON header (compact RFC 8259 format, separators=(',', ':'), preserving key definition order)
  12 + L                4             u32le     section_count (0 <= N <= 64)
  16 + L                N * 16        bytes     Section Table:
                                                  kind: u32le (1=icc, 2=f32)
                                                  reserved: u32le (0)
                                                  offset: u32le (4-byte aligned payload offset)
                                                  length: u32le (byte length)
  16 + L + N * 16       P             bytes     Binary payload area
"""

import hashlib
import json
import struct
import sys
from pathlib import Path

SPEC_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(SPEC_DIR))

from codec import (
    KIND_F32,
    KIND_ICC,
    MAGIC_REQ,
    MAGIC_RES,
    encode_frame,
    strict_decode_frame,
)

FIXTURES_DIR = SPEC_DIR / "fixtures"
BINARY_DIR = FIXTURES_DIR / "binary"


def generate_all() -> list[dict]:
    BINARY_DIR.mkdir(parents=True, exist_ok=True)

    vectors_spec = [
        # 1. Hello Request
        {
            "name": "wire-hello-req.bin",
            "magic": MAGIC_REQ,
            "json_file": "hello-request.json",
            "sections": [],
            "description": "Initial handshake negotiation request (protocol=2, features=[image-mask-v2, same-geometry, resource-control])",
            "origin": "fixtures/hello-request.json",
        },
        # 2. Hello Response
        {
            "name": "wire-hello-resp.bin",
            "magic": MAGIC_RES,
            "json_file": "hello-response.json",
            "sections": [],
            "description": "Initial handshake negotiation response (status=negotiated, selected_wire_version=2)",
            "origin": "fixtures/hello-response.json",
        },
        # 3. Control Probe Request
        {
            "name": "wire-control-probe-req.bin",
            "magic": MAGIC_REQ,
            "json_file": "control-probe-request.json",
            "sections": [],
            "description": "Read-only environment and model presence probe (action=probe, device_policy=auto)",
            "origin": "fixtures/control-probe-request.json",
        },
        # 4. Status Ready Response
        {
            "name": "wire-status-ready-resp.bin",
            "magic": MAGIC_RES,
            "json_file": "status-ready-response.json",
            "sections": [],
            "description": "Probe readiness response (status=ready, readiness=declared)",
            "origin": "fixtures/status-ready-response.json",
        },
        # 5. Control Prepare Request
        {
            "name": "wire-control-prepare-req.bin",
            "magic": MAGIC_REQ,
            "json_file": "control-prepare-request.json",
            "sections": [],
            "description": "Session pre-warming request (action=prepare, mode=exact, target_device=example.synthetic.cpu)",
            "origin": "fixtures/control-prepare-request.json",
        },
        # 6. Status Needs Model Response
        {
            "name": "wire-status-needs-model-resp.bin",
            "magic": MAGIC_RES,
            "json_file": "status-needs-model-response.json",
            "sections": [],
            "description": "Resource missing error response (status=needs_model, code=resource_missing, retryable=false)",
            "origin": "fixtures/status-needs-model-response.json",
        },
        # 7. Run Subject Request
        {
            "name": "wire-run-req.bin",
            "magic": MAGIC_REQ,
            "json_file": "run-subject-request.json",
            "sections": [
                # Section 0: 3x2 RGB float32 image samples (18 floats = 72 bytes)
                (KIND_F32, struct.pack("<18f", *[0.05 * (i + 1) for i in range(18)])),
                # Section 1: 3x2 Gray float32 selection mask samples (6 floats = 24 bytes)
                (KIND_F32, struct.pack("<6f", 1.0, 1.0, 1.0, 0.0, 0.0, 0.0)),
            ],
            "description": "Single-inference run request (operation=synthetic-mask, 2 f32 input sections: source RGB + selection)",
            "origin": "fixtures/run-subject-request.json + IEEE754 LE f32 samples",
        },
        # 8. Run OK Mask Response
        {
            "name": "wire-run-resp.bin",
            "magic": MAGIC_RES,
            "json_file": "ok-mask-response.json",
            "sections": [
                # Section 0: 3x2 Gray float32 mask output samples (6 floats = 24 bytes)
                (KIND_F32, struct.pack("<6f", 0.0, 1.0 / 255.0, 63.0 / 255.0, 127.0 / 255.0, 128.0 / 255.0, 1.0)),
            ],
            "description": "Single-inference run response (status=ok, 1 f32 output section: normalized mask [0.0, 1.0] + execution report)",
            "origin": "fixtures/ok-mask-response.json + IEEE754 LE f32 samples",
        },
    ]

    records = []
    for vec in vectors_spec:
        with open(FIXTURES_DIR / vec["json_file"], "r", encoding="utf-8") as f:
            json_obj = json.load(f)

        raw = encode_frame(vec["magic"], json_obj, vec["sections"])
        out_path = BINARY_DIR / vec["name"]
        with open(out_path, "wb") as f:
            f.write(raw)

        # Immediate round-trip decode verification using strict codec
        magic, decoded_json, sections_meta, payload = strict_decode_frame(raw)
        assert magic == vec["magic"]
        assert len(sections_meta) == len(vec["sections"])

        sha256 = hashlib.sha256(raw).hexdigest()
        records.append({
            "name": vec["name"],
            "size": len(raw),
            "sha256": sha256,
            "header_length": struct.unpack("<I", raw[8:12])[0],
            "section_count": len(sections_meta),
            "payload_length": len(payload),
            "description": vec["description"],
            "json_file": vec["json_file"],
            "origin": vec["origin"],
        })

    hashes_map = {
        r["name"]: {
            "size": r["size"],
            "sha256": r["sha256"],
            "header_length": r["header_length"],
            "section_count": r["section_count"],
            "payload_length": r["payload_length"],
            "description": r["description"],
            "origin": r["origin"],
        }
        for r in records
    }
    with open(BINARY_DIR / "golden-hashes.json", "w", encoding="utf-8") as f:
        json.dump(hashes_map, f, indent=2)

    return records


def write_documentation(records: list[dict]):
    md_lines = [
        "# PhotoCraft Adapter v2 二进制候选测试向量清单与规范（Binary Candidate Vectors）",
        "",
        "> **规范状态**：审查草案（Review Draft 2.0.0-draft-review-1）  ",
        "> **生成机制**：`docs/spec-adapter-v2/generate_binary_vectors.py`（基于 `codec.py` 严格编解码）  ",
        "> **产物定位**：候选向量与草案运行证据（Candidate Vectors & Draft Run Evidence）。固定基准、负例命中阶段、严格边界、握手及文档统一继续保持 Open。  ",
        "",
        "---",
        "",
        "## 1. 机器清单与产物来源（Machine Inventory & Provenance）",
        "",
        "- **架构中立性**：所有整型数值均采用明确的小端字节序（Little-Endian, `u32le`），浮点采样采用 IEEE 754 单精度浮点数小端（`f32le`），支持 x86_64 与 ARM64 跨平台无歧义解析；",
        "- **标头序列化**：JSON 标头采用 RFC 8259 严格紧凑序列化（无空格分隔符，键排序，UTF-8 严格编码，无 BOM）；",
        "- **来源追溯**：每个测试向量均由对应的 `docs/spec-adapter-v2/fixtures/*.json` 与规范浮点/分段采样切片确定性生成；",
        "- **交付物清单**：",
        "  1. `wire-hello-req.bin`",
        "  2. `wire-hello-resp.bin`",
        "  3. `wire-control-probe-req.bin`",
        "  4. `wire-status-ready-resp.bin`",
        "  5. `wire-control-prepare-req.bin`",
        "  6. `wire-status-needs-model-resp.bin`",
        "  7. `wire-run-req.bin`",
        "  8. `wire-run-resp.bin`",
        "",
        "---",
        "",
        "## 2. 黄金测试向量清单与完整 SHA-256 校验和",
        "",
        "| 文件名 | 大小 (B) | Header 长度 | Sections | Payload 长度 | 完整 SHA-256 校验和 | 来源与语义说明 |",
        "|---|---|---|---|---|---|---|",
    ]

    for r in records:
        md_lines.append(
            f"| `{r['name']}` | {r['size']} | {r['header_length']} | {r['section_count']} | {r['payload_length']} | `{r['sha256']}` | {r['origin']}：{r['description']} |"
        )

    md_lines.extend([
        "",
        "---",
        "",
        "## 3. 物理帧格式与字节布局解析",
        "",
        "每个 `.bin` 测试向量完全遵循以下物理内存布局：",
        "",
        "```text",
        "+-------------------+-------------------+--------------------+--------------------+--------------------+",
        "| Magic (8 字节)    | HeaderLen (4 字节)| UTF-8 JSON Header  | SectionCount(4字节)| Section Table (16N)| Payload (P 字节)   |",
        "| 'PCADREQ2'/'RES2' | u32le             | 长度 = HeaderLen    | 0 <= N <= 64       | N 个 16 字节条目   | 4 字节对齐数据区   |",
        "+-------------------+-------------------+--------------------+--------------------+--------------------+",
        "```",
        "",
        "### 3.1 Section Table Entry 结构（16 字节固定条目）",
        "- `kind: u32le`：`1` 表示 ICC Profile 二进制，`2` 表示 IEEE 754 float32 LE 采样值；",
        "- `reserved: u32le`：保留字段，严格为 `0`；",
        "- `offset: u32le`：相对 Payload 起始地址的字节偏移量，必须 4 字节对齐；",
        "- `length: u32le`：数据实际有效字节长度。",
        "",
        "### 3.2 典型向量解析示例",
        "",
        "#### `wire-run-req.bin` (推理请求)",
        "- **Magic**：`PCADREQ2` (`0x50 0x43 0x41 0x44 0x52 0x45 0x51 0x32`)；",
        "- **Section 0**：`kind=2` (f32), `offset=0`, `length=72`（3x2 RGB 图像，18 个 float32 数值）；",
        "- **Section 1**：`kind=2` (f32), `offset=72`, `length=24`（3x2 单通道灰度选区掩码，6 个 float32 数值，前三个为 1.0，后三个为 0.0）；",
        "- **Payload**：共计 96 字节，严格按 4 字节对齐。",
        "",
        "#### `wire-run-resp.bin` (推理成功响应)",
        "- **Magic**：`PCADRES2` (`0x50 0x43 0x41 0x44 0x52 0x45 0x53 0x32`)；",
        "- **Section 0**：`kind=2` (f32), `offset=0`, `length=24`（3x2 单通道灰度主体掩码，6 个 float32 数值，取值严格落在 [0.0, 1.0] 区间：`[0.0, 1/255, 63/255, 127/255, 128/255, 1.0]`）；",
        "- **Execution Report**：包含 `devices`, `backend`, `timing`（满足串行一致性）及 `graph_fallback: false`。",
        "",
        "---",
        "",
        "## 4. 一致性核验要求（Conformance Testing）",
        "",
        "任何声明支持 Adapter v2 的 Worker 或 Host 实现，必须在 CI 中通过 `docs/spec-adapter-v2/conformance_test.py` 的严格检验：",
        "1. **解帧一致性**：能够完整读取并反序列化上述 8 个测试向量，解出的 JSON 必须通过 JSON Schema 验证与契约关联验证；",
        "2. **编帧一致性**：输入相同的 JSON 与 Float 数据切片，生成的二进制帧必须与上述 SHA-256 校验和完全一致；",
        "3. **越界与畸变防护**：若篡改 `header_length`、`section_count` 或 `offset + length` 使其超过帧总长或违反预算，解码器必须立即返回 `invalid_request`，严禁触发空指针解引用或内存越界读取。",
    ])

    doc_path = BINARY_DIR / "GOLDEN-VECTORS.md"
    with open(doc_path, "w", encoding="utf-8") as f:
        f.write("\n".join(md_lines) + "\n")


if __name__ == "__main__":
    records = generate_all()
    write_documentation(records)
    print(f"Generated {len(records)} binary golden vectors and documentation.")
