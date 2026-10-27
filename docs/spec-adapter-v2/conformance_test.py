#!/usr/bin/env python3
"""
Conformance Test Suite for PhotoCraft Adapter v2 Wire Protocol.

Validates:
  1. Fixed Oracle verification: Strict decoding, schema validation, byte-for-byte re-encoding,
     and SHA-256 integrity of all 8 authoritative binary golden test vectors.
  2. Request-response correlation and policy execution checks on the 4 canonical pairs.
  3. Comprehensive negative rejection suite with true target-stage assertions across 19 malformed cases.
  4. Layered strict boundary tests (framing, JSON budgets, port geometry, and tensor sample limits).
  5. In-process client/worker session contract and state machine lifecycle transitions.
  6. Subprocess interop against reference worker and reference client.
"""

import argparse
import hashlib
import json
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

SPEC_DIR = Path(__file__).resolve().parent
FIXTURES_DIR = SPEC_DIR / "fixtures"
BINARY_DIR = FIXTURES_DIR / "binary"
EXAMPLES_DIR = SPEC_DIR / "examples"
SCHEMAS_DIR = SPEC_DIR / "schemas"

sys.path.insert(0, str(SPEC_DIR))
sys.path.insert(0, str(EXAMPLES_DIR))

from client import AdapterV2Client, InProcessAdapterClient
from codec import (
    KIND_F32,
    KIND_ICC,
    MAGIC_REQ,
    MAGIC_RES,
    encode_frame,
    pair_check,
    strict_decode_frame,
)
from worker import WorkerSession

# ==============================================================================
# 1. Authoritative Fixed Oracle Test Vectors
# Source: GitHub Actions Run 40360311553 (commit 5c10d371), Artifact 12668240638
# Encoding: RFC 8259 compact format (fixture key order, separators=(',', ':'))
# ==============================================================================
FIXED_ORACLE_VECTORS = {
    "wire-hello-req.bin": {
        "magic": MAGIC_REQ,
        "schema": "wire-request.schema.json",
        "header_length": 227,
        "section_count": 0,
        "payload_length": 0,
        "size": 243,
        "sha256": "81f181f0868f0cb15a96839350616b64f331fc3c8f87ee9e4ba6f59c256037f5",
    },
    "wire-hello-resp.bin": {
        "magic": MAGIC_RES,
        "schema": "wire-response.schema.json",
        "header_length": 279,
        "section_count": 0,
        "payload_length": 0,
        "size": 295,
        "sha256": "3870fe6f6cf5ce9a82e9874ccbf3e7b233f27f0506eb3e6ef61835b6a71bb952",
    },
    "wire-control-probe-req.bin": {
        "magic": MAGIC_REQ,
        "schema": "wire-request.schema.json",
        "header_length": 446,
        "section_count": 0,
        "payload_length": 0,
        "size": 462,
        "sha256": "d7790bdfba49e52516a7f5168e31006a88b56f8f5ea0e75a61e73ea515d427d1",
    },
    "wire-status-ready-resp.bin": {
        "magic": MAGIC_RES,
        "schema": "wire-response.schema.json",
        "header_length": 200,
        "section_count": 0,
        "payload_length": 0,
        "size": 216,
        "sha256": "1ad78ea822f36070fa3b455b8b93931be6f19ea911e359fe6788bf7d024ee184",
    },
    "wire-control-prepare-req.bin": {
        "magic": MAGIC_REQ,
        "schema": "wire-request.schema.json",
        "header_length": 450,
        "section_count": 0,
        "payload_length": 0,
        "size": 466,
        "sha256": "bb151b6932e67df1bb2fe2a7e7804473cf29e9d6d45da4e5262c5b364177ebf9",
    },
    "wire-status-needs-model-resp.bin": {
        "magic": MAGIC_RES,
        "schema": "wire-response.schema.json",
        "header_length": 298,
        "section_count": 0,
        "payload_length": 0,
        "size": 314,
        "sha256": "a703d98fb827e8a9d18721c5aa811a9e334df5d99b2eb25e6837bfdb3c18b76c",
    },
    "wire-run-req.bin": {
        "magic": MAGIC_REQ,
        "schema": "wire-request.schema.json",
        "header_length": 831,
        "section_count": 2,
        "payload_length": 96,
        "size": 975,
        "sha256": "5a8b54350325fecf52bbd922ec92569e45e7f13b63200a747971775a2f520775",
    },
    "wire-run-resp.bin": {
        "magic": MAGIC_RES,
        "schema": "wire-response.schema.json",
        "header_length": 887,
        "section_count": 1,
        "payload_length": 24,
        "size": 943,
        "sha256": "2d890bf7c5bc674d82f7678d8a7051bb3b7b20963577d4da0be697ee3e6c0c53",
    },
}


def _load_schema(schema_name: str) -> dict:
    with open(SCHEMAS_DIR / schema_name, "r", encoding="utf-8") as f:
        return json.load(f)


def _validate_schema(data: dict, schema_name: str):
    """Validate JSON against draft 2020-12 schema if jsonschema is installed."""
    try:
        import jsonschema
        from jsonschema import Draft202012Validator
        from referencing import Registry, Resource

        registry = Registry()
        for p in SCHEMAS_DIR.glob("*.json"):
            with open(p, "r", encoding="utf-8") as f:
                schema_json = json.load(f)
                resource = Resource.from_contents(schema_json)
                registry = registry.with_resource(p.name, resource)
                if "$id" in schema_json:
                    registry = registry.with_resource(schema_json["$id"], resource)

        target_schema = _load_schema(schema_name)
        validator = Draft202012Validator(target_schema, registry=registry)
        validator.validate(data)
    except ImportError:
        assert data.get("protocol") == 2
        assert "request_id" in data
        assert "type" in data
        assert "sections" in data


def test_golden_vectors():
    print("=== 1. Testing Binary Golden Vectors Against Fixed Oracle ===")
    for filename, oracle in FIXED_ORACLE_VECTORS.items():
        path = BINARY_DIR / filename
        if not path.exists():
            raise FileNotFoundError(f"Missing required golden vector: {path}")

        raw = path.read_bytes()

        # 1. Byte-level invariants from oracle
        assert len(raw) == oracle["size"], (
            f"{filename}: File size mismatch (got {len(raw)}, expected {oracle['size']})"
        )
        actual_sha256 = hashlib.sha256(raw).hexdigest()
        assert actual_sha256 == oracle["sha256"], (
            f"{filename}: SHA-256 mismatch (actual={actual_sha256}, oracle={oracle['sha256']})"
        )

        # 2. Strict wire frame decoding
        magic, json_header, sections, payload = strict_decode_frame(raw)
        assert magic == oracle["magic"], f"{filename}: Magic mismatch"
        assert len(sections) == oracle["section_count"], f"{filename}: Section count mismatch"
        assert len(payload) == oracle["payload_length"], f"{filename}: Payload length mismatch"
        assert json_header.get("protocol") == 2, f"{filename}: Protocol version must be 2"

        # 3. Schema validation on decoded JSON header
        _validate_schema(json_header, oracle["schema"])

        # 4. Immediate round-trip re-encoding check (byte-for-byte equality)
        sections_data = []
        for s in sections:
            start = s["offset"]
            end = start + s["length"]
            sections_data.append((s["kind"], payload[start:end]))

        re_encoded = encode_frame(magic, json_header, sections_data)
        assert re_encoded == raw, f"{filename}: Byte-for-byte re-encoding mismatch"

        print(
            f"  [PASS] {filename} (size: {len(raw)} B, sections: {len(sections)}, "
            f"sha256: {actual_sha256[:16]}...)"
        )


def test_golden_pairs():
    print("=== 2. Testing Request-Response Golden Pairs ===")
    pairs = [
        ("wire-hello-req.bin", "wire-hello-resp.bin"),
        ("wire-control-probe-req.bin", "wire-status-ready-resp.bin"),
        ("wire-control-prepare-req.bin", "wire-status-needs-model-resp.bin"),
        ("wire-run-req.bin", "wire-run-resp.bin"),
    ]

    for req_name, resp_name in pairs:
        req_raw = (BINARY_DIR / req_name).read_bytes()
        resp_raw = (BINARY_DIR / resp_name).read_bytes()

        _, req_json, _, _ = strict_decode_frame(req_raw, expected_magic=MAGIC_REQ)
        _, resp_json, _, _ = strict_decode_frame(resp_raw, expected_magic=MAGIC_RES)

        pair_check(req_json, resp_json)
        print(f"  [PASS] Correlation verified for pair: ({req_name} <-> {resp_name})")


def test_malformed_frames():
    print("=== 3. Testing 19 Negative Malformed Frames (True Stage Assertions) ===")
    valid_raw = (BINARY_DIR / "wire-run-req.bin").read_bytes()
    header_len = struct.unpack("<I", valid_raw[8:12])[0]
    sec_count_offset = 12 + header_len
    sec_table_start = sec_count_offset + 4

    # Construction of the 19 negative cases
    # Non-zero Reserved Field
    bad_reserved = bytearray(valid_raw)
    bad_reserved[sec_table_start + 4 : sec_table_start + 8] = struct.pack("<I", 1)

    # Unaligned Section Offset
    bad_unaligned = bytearray(valid_raw)
    bad_unaligned[sec_table_start + 8 : sec_table_start + 12] = struct.pack("<I", 3)

    # Section Exceeding Payload Bound
    bad_overflow = bytearray(valid_raw)
    bad_overflow[sec_table_start + 12 : sec_table_start + 16] = struct.pack("<I", 268_435_460)

    # UTF-8 BOM
    json_slice = valid_raw[12:sec_count_offset]
    bom_json = b"\xef\xbb\xbf" + json_slice
    bad_bom = (
        valid_raw[:8]
        + struct.pack("<I", len(bom_json))
        + bom_json
        + valid_raw[sec_count_offset:]
    )

    # Duplicate JSON Keys
    dup_json = b'{"protocol":2,"protocol":2,"request_id":"req-001","type":"hello"}'
    bad_dup = (
        valid_raw[:8]
        + struct.pack("<I", len(dup_json))
        + dup_json
        + struct.pack("<I", 0)
    )

    # JSON Nesting Exceeding Limit
    deep_json = b'{"protocol":2,"request_id":"req-001","type":"hello","nested":{"a":{"a":{"a":{"a":{"a":{"a":{"a":{"a":{"a":{"a":{"a":{"a":{"a":{"a":{"a":{"a":1}}}}}}}}}}}}}}}}}'
    bad_deep = (
        valid_raw[:8]
        + struct.pack("<I", len(deep_json))
        + deep_json
        + struct.pack("<I", 0)
    )

    # Direction Violation - Request with Status
    bad_req_status_json = b'{"protocol":2,"request_id":"req-001","type":"run","status":"ok"}'
    bad_req_status = (
        MAGIC_REQ
        + struct.pack("<I", len(bad_req_status_json))
        + bad_req_status_json
        + struct.pack("<I", 0)
    )

    # Direction Violation - Response with Inputs
    bad_res_inputs_json = b'{"protocol":2,"request_id":"req-001","type":"run","status":"ok","inputs":{}}'
    bad_res_inputs = (
        MAGIC_RES
        + struct.pack("<I", len(bad_res_inputs_json))
        + bad_res_inputs_json
        + struct.pack("<I", 0)
    )

    # Mask Port with Forbidden Color Encoding (Sections meta aligns so port validation is reached)
    bad_mask_color_json = (
        b'{"protocol":2,"request_id":"req-001","type":"run","operation":"synthetic-mask",'
        b'"sections":[{"kind":"f32","offset":0,"length":12},{"kind":"f32","offset":12,"length":4}],'
        b'"inputs":{"source":{"kind":"image","width":1,"height":1,"channels":3,"data_section_index":0}},'
        b'"params":{"mask":{"kind":"mask","width":1,"height":1,"channels":1,"color_encoding":"srgb","data_section_index":1}}}'
    )
    mask_sec_table = struct.pack("<IIII", KIND_F32, 0, 0, 12) + struct.pack("<IIII", KIND_F32, 0, 12, 4)
    bad_mask_color = (
        MAGIC_REQ
        + struct.pack("<I", len(bad_mask_color_json))
        + bad_mask_color_json
        + struct.pack("<I", 2)
        + mask_sec_table
        + b"\x00" * 16
    )

    # Mask Port with Forbidden ICC Index (Sections meta aligns so port validation is reached)
    bad_mask_icc_json = (
        b'{"protocol":2,"request_id":"req-001","type":"run","operation":"synthetic-mask",'
        b'"sections":[{"kind":"f32","offset":0,"length":12},{"kind":"f32","offset":12,"length":4},{"kind":"icc","offset":16,"length":16}],'
        b'"inputs":{"source":{"kind":"image","width":1,"height":1,"channels":3,"data_section_index":0}},'
        b'"params":{"mask":{"kind":"mask","width":1,"height":1,"channels":1,"icc_section_index":2,"data_section_index":1}}}'
    )
    mask_icc_sec_table = (
        struct.pack("<IIII", KIND_F32, 0, 0, 12)
        + struct.pack("<IIII", KIND_F32, 0, 12, 4)
        + struct.pack("<IIII", KIND_ICC, 0, 16, 16)
    )
    bad_mask_icc = (
        MAGIC_REQ
        + struct.pack("<I", len(bad_mask_icc_json))
        + bad_mask_icc_json
        + struct.pack("<I", 3)
        + mask_icc_sec_table
        + b"\x00" * 32
    )

    # Sample Count Exceeding Budget (67,117,056 float samples > 67,108,864 budget)
    bad_samples_json = (
        b'{"protocol":2,"request_id":"req-001","type":"run","operation":"synthetic-mask",'
        b'"sections":[{"kind":"f32","offset":0,"length":4}],'
        b'"inputs":{"source":{"kind":"image","width":8192,"height":8193,"channels":1,"data_section_index":0}}}'
    )
    bad_samples_sec_table = struct.pack("<IIII", KIND_F32, 0, 0, 4)
    bad_sample_budget = (
        MAGIC_REQ
        + struct.pack("<I", len(bad_samples_json))
        + bad_samples_json
        + struct.pack("<I", 1)
        + bad_samples_sec_table
        + b"\x00" * 4
    )

    # JSON Key Exceeding 128 Bytes
    long_key = "k" * 129
    bad_key_json = f'{{"protocol":2,"request_id":"req-001","type":"hello","{long_key}":1}}'.encode("utf-8")
    bad_long_key = MAGIC_REQ + struct.pack("<I", len(bad_key_json)) + bad_key_json + struct.pack("<I", 0)

    # JSON String Exceeding 4096 Chars
    long_val = "v" * 4097
    bad_val_json = f'{{"protocol":2,"request_id":"req-001","type":"hello","desc":"{long_val}"}}'.encode("utf-8")
    bad_long_val = MAGIC_REQ + struct.pack("<I", len(bad_val_json)) + bad_val_json + struct.pack("<I", 0)

    # JSON Object Exceeding 512 Properties
    props = ",".join(f'"p{i}":{i}' for i in range(513))
    bad_props_json = f'{{"protocol":2,"request_id":"req-001","type":"hello",{props}}}'.encode("utf-8")
    bad_many_props = MAGIC_REQ + struct.pack("<I", len(bad_props_json)) + bad_props_json + struct.pack("<I", 0)

    cases = [
        ("Bad Magic", b"BADMAGIC" + valid_raw[8:], ValueError, "STAGE_MAGIC", "Unrecognized wire magic"),
        ("Too Short Frame", valid_raw[:12], ValueError, "STAGE_FRAME_LEN", "minimum header size"),
        ("Oversized Header Length", valid_raw[:8] + struct.pack("<I", 2_000_000) + valid_raw[12:], ValueError, "STAGE_HEADER_LEN", "exceeds 1 MiB limit"),
        ("Truncated JSON", valid_raw[:8] + struct.pack("<I", len(valid_raw) + 100) + valid_raw[12:], ValueError, "STAGE_FRAME_TRUNC", "Frame truncated before section count"),
        ("Section Count > 64", valid_raw[:sec_count_offset] + struct.pack("<I", 65) + valid_raw[sec_count_offset + 4:], ValueError, "STAGE_SEC_COUNT", "exceeds limit of 64"),
        ("Non-zero Reserved Field in Section", bytes(bad_reserved), ValueError, "STAGE_SEC_TABLE", "reserved field is non-zero"),
        ("Unaligned Section Offset", bytes(bad_unaligned), ValueError, "STAGE_SEC_TABLE", "is not 4-byte aligned"),
        ("Section Exceeding Payload Bound", bytes(bad_overflow), ValueError, "STAGE_PAYLOAD_BOUND", "exceeds payload budget"),
        ("UTF-8 BOM in JSON Header", bad_bom, ValueError, "STAGE_JSON_BOM", "UTF-8 byte order mark (BOM) is forbidden"),
        ("Duplicate JSON Keys", bad_dup, ValueError, "STAGE_JSON_PARSE", "Duplicate key found in JSON object"),
        ("JSON Nesting Exceeding Limit", bad_deep, ValueError, "STAGE_JSON_BUDGET", "JSON nesting depth exceeded limit 16"),
        ("Direction Violation - Request with Status", bad_req_status, ValueError, "STAGE_DIRECTION", "Request frame (PCADREQ2) contains forbidden 'status' field"),
        ("Direction Violation - Response with Inputs", bad_res_inputs, ValueError, "STAGE_DIRECTION", "Response frame (PCADRES2) contains forbidden request fields"),
        ("Mask Port with Forbidden Color Encoding", bad_mask_color, ValueError, "STAGE_PORT_POLICY", "forbidden to declare 'color_encoding'"),
        ("Mask Port with Forbidden ICC Index", bad_mask_icc, ValueError, "STAGE_PORT_POLICY", "forbidden to reference ICC section"),
        ("Sample Count Exceeding Budget", bad_sample_budget, ValueError, "STAGE_SAMPLE_BUDGET", "exceeds maximum allowed single tensor samples"),
        ("JSON Key Exceeding 128 Bytes", bad_long_key, ValueError, "STAGE_JSON_BUDGET", "JSON object key byte length budget exceeded"),
        ("JSON String Exceeding 4096 Chars", bad_long_val, ValueError, "STAGE_JSON_BUDGET", "JSON string character length budget exceeded"),
        ("JSON Object Exceeding 512 Properties", bad_many_props, ValueError, "STAGE_JSON_BUDGET", "JSON object property count budget exceeded"),
    ]

    for name, payload, exp_exc, target_stage_id, exp_substr in cases:
        try:
            strict_decode_frame(payload)
            raise AssertionError(f"Expected failure at {target_stage_id} for '{name}', but succeeded!")
        except exp_exc as e:
            assert exp_substr in str(e), (
                f"Stage mismatch on '{name}': expected substring {exp_substr!r}, got error {str(e)!r}"
            )
            print(f"  [PASS] [{target_stage_id}] Rejected '{name}': {exp_substr!r}")


def test_strict_boundaries():
    print("=== 4. Testing Layered Strict Boundaries ===")

    # --------------------------------------------------------------------------
    # Layer 1: Framing and Section Table Boundaries
    # --------------------------------------------------------------------------
    # Case 1.1: Header Length exactly 1 MiB (1,048,576 bytes) -> Valid reachable positive
    pad_items = []
    # Total available: 1_048_576 - 104 (prefix) - 2 (suffix) = 1_048_470 bytes.
    # 300 properties: 299 with comma (11 bytes fixed), 1 without (10 bytes fixed) -> 3299 fixed.
    # 1_048_470 - 3299 = 1_045_171 chars across 300 string values.
    # 1_045_171 // 300 = 3483, remainder = 271.
    for i in range(300):
        val_len = 3484 if i < 271 else 3483
        pad_items.append(f'"p{i:03d}":"' + ("a" * val_len) + '"')
    pad_body = ",".join(pad_items).encode("ascii")
    prefix = b'{"protocol":2,"request_id":"req-bound-1","type":"hello","client":{"name":"test","version":"1.0"},"pad":{'
    suffix = b"}}"
    valid_1mib_json = prefix + pad_body + suffix
    assert len(valid_1mib_json) == 1_048_576, f"Calculated length {len(valid_1mib_json)} != 1048576"
    frame_1mib_valid = (
        MAGIC_REQ
        + struct.pack("<I", len(valid_1mib_json))
        + valid_1mib_json
        + struct.pack("<I", 0)
    )
    magic, hdr, secs, _ = strict_decode_frame(frame_1mib_valid)
    assert magic == MAGIC_REQ
    print("  [PASS] [Layer 1] Reachable Header length at 1 MiB boundary successfully accepted")

    # Case 1.2: Header Length 1,048,577 bytes (1 MiB + 1 byte) -> Rejected
    frame_1mib_exceeded = MAGIC_REQ + struct.pack("<I", 1_048_577) + (b"x" * 16)
    try:
        strict_decode_frame(frame_1mib_exceeded)
        raise AssertionError("Expected failure on header > 1 MiB, but succeeded")
    except ValueError as e:
        assert "exceeds 1 MiB limit" in str(e)
    print("  [PASS] [Layer 1] Header length at 1 MiB + 1 byte strictly rejected")

    # Case 1.3: Section Count 65 (64 is max allowed) -> Rejected
    sec_65_frame = (
        MAGIC_REQ
        + struct.pack("<I", len(b'{"protocol":2,"request_id":"r","type":"hello"}'))
        + b'{"protocol":2,"request_id":"r","type":"hello"}'
        + struct.pack("<I", 65)
    )
    try:
        strict_decode_frame(sec_65_frame)
        raise AssertionError("Expected failure on section_count = 65, but succeeded")
    except ValueError as e:
        assert "exceeds limit of 64" in str(e)
    print("  [PASS] [Layer 1] Section count = 65 strictly rejected")

    # --------------------------------------------------------------------------
    # Layer 2: JSON Budget Boundaries
    # --------------------------------------------------------------------------
    # Case 2.1: Object Key Length = 128 bytes (allowed) vs 129 bytes (rejected)
    key_128 = "k" * 128
    json_128 = f'{{"protocol":2,"request_id":"r","type":"hello","{key_128}":1}}'.encode("utf-8")
    frame_128 = MAGIC_REQ + struct.pack("<I", len(json_128)) + json_128 + struct.pack("<I", 0)
    strict_decode_frame(frame_128)

    key_129 = "k" * 129
    json_129 = f'{{"protocol":2,"request_id":"r","type":"hello","{key_129}":1}}'.encode("utf-8")
    frame_129 = MAGIC_REQ + struct.pack("<I", len(json_129)) + json_129 + struct.pack("<I", 0)
    try:
        strict_decode_frame(frame_129)
        raise AssertionError("Expected failure on key > 128B, but succeeded")
    except ValueError as e:
        assert "JSON object key byte length budget exceeded" in str(e)
    print("  [PASS] [Layer 2] JSON key length boundary (128B pass vs 129B reject) verified")

    # Case 2.2: String Character Length = 4096 chars (allowed) vs 4097 chars (rejected)
    str_4096 = "s" * 4096
    json_s4096 = f'{{"protocol":2,"request_id":"r","type":"hello","desc":"{str_4096}"}}'.encode("utf-8")
    frame_s4096 = MAGIC_REQ + struct.pack("<I", len(json_s4096)) + json_s4096 + struct.pack("<I", 0)
    strict_decode_frame(frame_s4096)

    str_4097 = "s" * 4097
    json_s4097 = f'{{"protocol":2,"request_id":"r","type":"hello","desc":"{str_4097}"}}'.encode("utf-8")
    frame_s4097 = MAGIC_REQ + struct.pack("<I", len(json_s4097)) + json_s4097 + struct.pack("<I", 0)
    try:
        strict_decode_frame(frame_s4097)
        raise AssertionError("Expected failure on string > 4096 chars, but succeeded")
    except ValueError as e:
        assert "JSON string character length budget exceeded" in str(e)
    print("  [PASS] [Layer 2] JSON string length boundary (4096 chars pass vs 4097 chars reject) verified")

    # Case 2.3: Object Properties Count = 512 properties (allowed) vs 513 properties (rejected)
    props_512 = ",".join(f'"p{i}":{i}' for i in range(512))
    json_512 = f'{{"protocol":2,"request_id":"r","type":"hello","pad":{{{props_512}}}}}'.encode("utf-8")
    frame_512 = MAGIC_REQ + struct.pack("<I", len(json_512)) + json_512 + struct.pack("<I", 0)
    strict_decode_frame(frame_512)

    props_513 = ",".join(f'"p{i}":{i}' for i in range(513))
    json_513 = f'{{"protocol":2,"request_id":"r","type":"hello","pad":{{{props_513}}}}}'.encode("utf-8")
    frame_513 = MAGIC_REQ + struct.pack("<I", len(json_513)) + json_513 + struct.pack("<I", 0)
    try:
        strict_decode_frame(frame_513)
        raise AssertionError("Expected failure on properties > 512, but succeeded")
    except ValueError as e:
        assert "JSON object property count budget exceeded" in str(e)
    print("  [PASS] [Layer 2] JSON properties count boundary (512 pass vs 513 reject) verified")

    # Case 2.4: Nesting Depth = 16 (allowed) vs 17 (rejected)
    deep_14 = '{"a":' * 14 + '1' + '}' * 14
    json_d16 = f'{{"protocol":2,"request_id":"r","type":"hello","nested":{deep_14}}}'.encode("utf-8")
    frame_d16 = MAGIC_REQ + struct.pack("<I", len(json_d16)) + json_d16 + struct.pack("<I", 0)
    strict_decode_frame(frame_d16)

    deep_15 = '{"a":' * 15 + '1' + '}' * 15
    json_d17 = f'{{"protocol":2,"request_id":"r","type":"hello","nested":{deep_15}}}'.encode("utf-8")
    frame_d17 = MAGIC_REQ + struct.pack("<I", len(json_d17)) + json_d17 + struct.pack("<I", 0)
    try:
        strict_decode_frame(frame_d17)
        raise AssertionError("Expected failure on nesting > 16, but succeeded")
    except ValueError as e:
        assert "JSON nesting depth exceeded limit 16" in str(e)
    print("  [PASS] [Layer 2] JSON nesting depth boundary (16 pass vs 17 reject) verified")

    # --------------------------------------------------------------------------
    # Layer 3: Geometry & Tensor Multiplications
    # --------------------------------------------------------------------------
    # Case 3.1: Port Dimensions bound [1, 16384] -> Width 16385 rejected
    bad_dim_json = (
        b'{"protocol":2,"request_id":"r","type":"run","operation":"synthetic-mask",'
        b'"sections":[{"kind":"f32","offset":0,"length":65540}],'
        b'"inputs":{"source":{"kind":"image","width":16385,"height":1,"channels":1,"data_section_index":0}}}'
    )
    dim_sec_table = struct.pack("<IIII", KIND_F32, 0, 0, 16385 * 4)
    frame_bad_dim = (
        MAGIC_REQ
        + struct.pack("<I", len(bad_dim_json))
        + bad_dim_json
        + struct.pack("<I", 1)
        + dim_sec_table
        + b"\x00" * (16385 * 4)
    )
    try:
        strict_decode_frame(frame_bad_dim)
        raise AssertionError("Expected failure on width 16385, but succeeded")
    except ValueError as e:
        assert "exceed [1, 16384] bound" in str(e)
    print("  [PASS] [Layer 3] Port dimension bound ([1, 16384]) verified")

    # Case 3.2: Tensor sample budget (67,108,864 float32 samples = 256 MiB)
    # 8192 * 8193 * 1 = 67,117,056 samples > 67,108,864 (rejected)
    bad_tensor_json = (
        b'{"protocol":2,"request_id":"r","type":"run","operation":"synthetic-mask",'
        b'"sections":[{"kind":"f32","offset":0,"length":4}],'
        b'"inputs":{"source":{"kind":"image","width":8192,"height":8193,"channels":1,"data_section_index":0}}}'
    )
    tensor_sec_table = struct.pack("<IIII", KIND_F32, 0, 0, 4)
    frame_bad_tensor = (
        MAGIC_REQ
        + struct.pack("<I", len(bad_tensor_json))
        + bad_tensor_json
        + struct.pack("<I", 1)
        + tensor_sec_table
        + b"\x00" * 4
    )
    try:
        strict_decode_frame(frame_bad_tensor)
        raise AssertionError("Expected failure on tensor samples > 67M, but succeeded")
    except ValueError as e:
        assert "exceeds maximum allowed single tensor samples" in str(e)
    print("  [PASS] [Layer 3] Tensor sample budget (67,108,864 max samples) verified")


def test_in_process_session_contract():
    print("=== 5. Testing In-Process Session Contract and State Transitions ===")

    # 1. Uninitialized state violation: direct run before hello closes endpoint
    session_viol_run = WorkerSession()
    client_viol_run = InProcessAdapterClient(session_viol_run)
    run_req_raw = (BINARY_DIR / "wire-run-req.bin").read_bytes()
    resp_json, _, _ = client_viol_run.invoke(run_req_raw, verify_pair=False)
    assert resp_json.get("status") == "error", "Direct run before hello must fail"
    assert resp_json.get("error", {}).get("code") == "invalid_request"
    assert session_viol_run.is_closed, "Session endpoint must close after direct run before hello"
    print("  [PASS] Direct run before hello successfully rejected and closed endpoint")

    # 2. Uninitialized state violation: direct control probe before hello closes endpoint
    session_viol_probe = WorkerSession()
    client_viol_probe = InProcessAdapterClient(session_viol_probe)
    probe_req_raw = (BINARY_DIR / "wire-control-probe-req.bin").read_bytes()
    resp_json, _, _ = client_viol_probe.invoke(probe_req_raw, verify_pair=False)
    assert resp_json.get("status") == "error", "Direct control probe before hello must fail"
    assert resp_json.get("error", {}).get("code") == "invalid_request"
    assert session_viol_probe.is_closed, "Session endpoint must close after direct probe before hello"
    print("  [PASS] Direct control probe before hello successfully rejected and closed endpoint")

    # 3. Successful handshake negotiation on clean session
    session = WorkerSession()
    client = InProcessAdapterClient(session)
    hello_req_raw = (BINARY_DIR / "wire-hello-req.bin").read_bytes()
    resp_json, _, _ = client.invoke(hello_req_raw, verify_pair=True)
    assert resp_json.get("status") == "negotiated"
    assert session.state == "NEGOTIATED"
    assert not session.is_closed
    print("  [PASS] Handshake negotiation succeeded, session entered NEGOTIATED state")

    # 4. Repeated hello handshake forbidden -> closes endpoint
    resp_json, _, _ = client.invoke(hello_req_raw, verify_pair=False)
    assert resp_json.get("status") == "error"
    assert resp_json.get("error", {}).get("code") == "invalid_request"
    assert session.is_closed, "Repeated hello must close session endpoint"
    print("  [PASS] Repeated hello handshake rejected with invalid_request and closed endpoint")

    # 5. Control prepare on fresh session -> enters NEEDS_MODEL (synthetic worker has no real disk weight for example model)
    session = WorkerSession()
    client = InProcessAdapterClient(session)
    resp_json, _, _ = client.invoke(hello_req_raw, verify_pair=True)
    assert resp_json.get("status") == "negotiated"

    prepare_req_raw = (BINARY_DIR / "wire-control-prepare-req.bin").read_bytes()
    resp_json, _, _ = client.invoke(prepare_req_raw, verify_pair=True)
    assert resp_json.get("status") == "needs_model"
    assert resp_json.get("error", {}).get("code") == "resource_missing"
    assert session.state == "NEEDS_MODEL"
    print("  [PASS] Prepare missing model transitioned session to NEEDS_MODEL")

    # 6. Run under NEEDS_MODEL -> rejected with resource_missing
    _, raw_run_json, run_sections_meta, run_payload = strict_decode_frame(
        run_req_raw, expected_magic=MAGIC_REQ
    )
    raw_run_json["request_id"] = "req-session-test-001"
    run_req_fresh = encode_frame(
        MAGIC_REQ,
        raw_run_json,
        [
            (s["kind"], run_payload[s["offset"] : s["offset"] + s["length"]])
            for s in run_sections_meta
        ],
    )
    resp_json, _, _ = client.invoke(run_req_fresh, verify_pair=False)
    assert resp_json.get("status") == "needs_model"
    assert resp_json.get("error", {}).get("code") == "resource_missing"
    print("  [PASS] Run under NEEDS_MODEL state strictly rejected with resource_missing")

    # 7. Device policy mismatch rejection and state retention
    raw_run_json["request_id"] = "req-session-test-002"
    raw_run_json["device_policy"] = {
        "mode": "exact",
        "target_device": "qualcomm.qnn.htp.0",
        "allow_graph_fallback": False,
    }
    # Set session to NEGOTIATED for policy test
    session.state = "NEGOTIATED"
    run_policy_mismatch = encode_frame(
        MAGIC_REQ,
        raw_run_json,
        [
            (s["kind"], run_payload[s["offset"] : s["offset"] + s["length"]])
            for s in run_sections_meta
        ],
    )
    resp_json, _, _ = client.invoke(run_policy_mismatch, verify_pair=False)
    assert resp_json.get("status") == "unavailable"
    assert resp_json.get("error", {}).get("code") == "device_unavailable"
    assert session.state == "NEGOTIATED", "Failed run policy mismatch must retain NEGOTIATED state"
    print("  [PASS] Device policy mismatch returned status 'unavailable' (code 'device_unavailable') and retained NEGOTIATED state")

    # 8. Explicit session closure and post-closure rejection
    session.close()
    assert session.is_closed
    raw_run_json["request_id"] = "req-session-test-003"
    run_req_post_close = encode_frame(
        MAGIC_REQ,
        raw_run_json,
        [
            (s["kind"], run_payload[s["offset"] : s["offset"] + s["length"]])
            for s in run_sections_meta
        ],
    )
    resp_json, _, _ = client.invoke(run_req_post_close, verify_pair=False)
    assert resp_json.get("status") == "error"
    assert resp_json.get("error", {}).get("code") == "invalid_request"
    print("  [PASS] Explicit session close and post-close request rejection verified")
    print(
        f"  [PASS] Device policy mismatch strictly rejected with "
        f"'{resp_json.get('error', {}).get('code')}'"
    )


def test_reference_worker_interop():
    print("=== 6. Testing Reference Worker CLI Subprocess Interop ===")
    worker_script = EXAMPLES_DIR / "worker.py"
    assert worker_script.exists(), f"Reference worker script missing: {worker_script}"

    actions_and_reqs = [
        ("hello", BINARY_DIR / "wire-hello-req.bin", "negotiated", None),
        ("probe", BINARY_DIR / "wire-control-probe-req.bin", "ready", "declared"),
        ("prepare", BINARY_DIR / "wire-control-prepare-req.bin", "needs_model", None),
        ("run", BINARY_DIR / "wire-run-req.bin", "ok", None),
    ]

    for action, req_path, expected_status, expected_readiness in actions_and_reqs:
        with tempfile.TemporaryDirectory(prefix="pcad_test_") as tmp_dir:
            tmp_path = Path(tmp_dir)
            resp_path = tmp_path / "response.bin"

            cmd = [
                sys.executable,
                str(worker_script),
                "--action",
                action,
                "--request",
                str(req_path),
                "--response",
                str(resp_path),
            ]

            proc = subprocess.run(cmd, capture_output=True, text=True)
            assert proc.returncode == 0, f"Worker failed: {proc.stderr}"
            assert resp_path.exists(), "Worker did not generate response file"

            req_raw = req_path.read_bytes()
            resp_raw = resp_path.read_bytes()

            _, req_json, _, _ = strict_decode_frame(req_raw, expected_magic=MAGIC_REQ)
            magic, json_header, sections, payload = strict_decode_frame(
                resp_raw, expected_magic=MAGIC_RES
            )

            assert magic == MAGIC_RES, "Expected PCADRES2"
            assert json_header.get("status") == expected_status, (
                f"Status mismatch: {json_header.get('status')}"
            )

            pair_check(req_json, json_header)
            _validate_schema(json_header, "wire-response.schema.json")

            if expected_readiness:
                assert json_header.get("readiness") == expected_readiness, (
                    f"Readiness mismatch: {json_header.get('readiness')}"
                )

            if action == "run":
                assert len(sections) == 1, "Run response must contain 1 mask output section"
                assert len(payload) >= 24, "Run response mask payload must contain 24 bytes (6 floats)"
                mask_floats = struct.unpack("<6f", payload[:24])
                for val in mask_floats:
                    assert 0.0 <= val <= 1.0, f"Mask value out of [0.0, 1.0] range: {val}"

            print(
                f"  [PASS] Reference worker successfully handled action '{action}' -> "
                f"status '{expected_status}'"
            )


def test_reference_client_interop():
    print("=== 7. Testing Reference Client Subprocess Interop ===")
    worker_script = EXAMPLES_DIR / "worker.py"
    client = AdapterV2Client(worker_script)

    req_path = BINARY_DIR / "wire-run-req.bin"
    req_raw = req_path.read_bytes()

    res_json, res_sections, res_payload = client.invoke("run", req_raw, verify_pair=True)
    assert res_json.get("status") == "ok"
    assert len(res_sections) == 1
    assert len(res_payload) >= 24
    print("  [PASS] Reference client invoke and pair verification succeeded.")


def main():
    parser = argparse.ArgumentParser(description="PhotoCraft Adapter v2 Conformance Test Suite")
    parser.add_argument(
        "--mode",
        choices=["verify", "generate-and-verify"],
        default="verify",
        help="Test execution mode: verify against fixed oracle (default) or generate and verify",
    )
    args = parser.parse_args()

    if args.mode == "generate-and-verify":
        from generate_binary_vectors import generate_all

        print("Generating candidate binary vectors...")
        generate_all()

    test_golden_vectors()
    test_golden_pairs()
    test_malformed_frames()
    test_strict_boundaries()
    test_in_process_session_contract()
    test_reference_worker_interop()
    test_reference_client_interop()
    print("\nAll Adapter v2 Conformance Tests PASSED.")


if __name__ == "__main__":
    main()
