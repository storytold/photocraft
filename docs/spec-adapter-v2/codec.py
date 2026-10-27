#!/usr/bin/env python3
"""
PhotoCraft Adapter v2 Strict Codec and Verification Library.

Defines wire protocol constants, strict binary framing encoder and decoder,
budget checkers, section correlators, and request-response pair correlation
conforming to docs/spec-adapter-v2/ specifications.
"""

import json
import math
import struct
from typing import Any

MAGIC_REQ = b"PCADREQ2"
MAGIC_RES = b"PCADRES2"

KIND_ICC = 1
KIND_F32 = 2

KIND_INT_TO_STR = {KIND_ICC: "icc", KIND_F32: "f32"}
KIND_STR_TO_INT = {"icc": KIND_ICC, "f32": KIND_F32}

BASE_FEATURES = {"image-mask-v2", "same-geometry", "resource-control"}
ZERO_DIGEST = "0" * 64

# Strict protocol budgets
MAX_HEADER_BYTES = 1_048_576       # 1 MiB
MAX_SECTIONS = 64
MAX_ICC_SECTION_BYTES = 4_194_304   # 4 MiB
MAX_AGGREGATE_ICC_BYTES = 16_777_216  # 16 MiB
MAX_F32_SAMPLES = 67_108_864
MAX_PAYLOAD_BYTES = 268_435_456    # 256 MiB

MAX_JSON_DEPTH = 16
MAX_JSON_NODES = 100_000
MAX_OBJECT_PROPERTIES = 512
MAX_ARRAY_ITEMS = 512
MAX_KEY_BYTES = 128
MAX_STRING_CHARS = 4096


def _no_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    res = {}
    for k, v in pairs:
        if k in res:
            raise ValueError(f"Duplicate JSON key: {k!r}")
        res[k] = v
    return res


def _reject_constant(val: str) -> None:
    raise ValueError(f"Non-standard JSON numeric constant rejected: {val!r}")


def _check_tree_budget(value: Any, depth: int = 1, counter: list[int] | None = None) -> None:
    if counter is None:
        counter = [0]
    counter[0] += 1
    if depth > MAX_JSON_DEPTH or counter[0] > MAX_JSON_NODES:
        raise ValueError("JSON depth or node budget exceeded")

    if isinstance(value, dict):
        if len(value) > MAX_OBJECT_PROPERTIES:
            raise ValueError("JSON object property count budget exceeded")
        for k, v in value.items():
            if len(k.encode("utf-8")) > MAX_KEY_BYTES:
                raise ValueError("JSON object key byte length budget exceeded")
            _check_tree_budget(v, depth + 1, counter)
    elif isinstance(value, list):
        if len(value) > MAX_ARRAY_ITEMS:
            raise ValueError("JSON array item count budget exceeded")
        for item in value:
            _check_tree_budget(item, depth + 1, counter)
    elif isinstance(value, str):
        if len(value) > MAX_STRING_CHARS:
            raise ValueError("JSON string character length budget exceeded")
        value.encode("utf-8", errors="strict")  # Disallow lone surrogates
    elif isinstance(value, float):
        if not math.isfinite(value):
            raise ValueError("Non-finite float value in JSON")


def strict_json_loads(data: bytes) -> dict[str, Any]:
    """Strictly deserialize JSON bytes with budget, nesting, and duplicate checks."""
    if data.startswith(b"\xef\xbb\xbf"):
        raise ValueError("UTF-8 byte order mark (BOM) is forbidden in JSON header")

    text = data.decode("utf-8", errors="strict")

    # Fast nesting validation
    nesting = 0
    in_quote = False
    escaped = False
    for ch in text:
        if in_quote:
            if escaped:
                escaped = False
            elif ch == "\\":
                escaped = True
            elif ch == '"':
                in_quote = False
        elif ch == '"':
            in_quote = True
        elif ch in "[{":
            nesting += 1
            if nesting > MAX_JSON_DEPTH:
                raise ValueError("JSON nesting depth exceeded limit 16")
        elif ch in "]}":
            nesting -= 1

    obj = json.loads(
        text,
        object_pairs_hook=_no_duplicates,
        parse_constant=_reject_constant,
    )
    _check_tree_budget(obj)
    if not isinstance(obj, dict):
        raise ValueError("JSON header root must be an object")
    return obj


def encode_frame(magic: bytes, json_obj: dict[str, Any], sections_data: list[tuple[int, bytes]]) -> bytes:
    """Encode a complete v2 wire frame with binary payload and strict alignment."""
    if magic not in (MAGIC_REQ, MAGIC_RES):
        raise ValueError(f"Invalid magic bytes: {magic!r}")

    # Direction consistency check
    if magic == MAGIC_REQ:
        if "status" in json_obj or "outputs" in json_obj or "execution" in json_obj:
            raise ValueError("Request frame (PCADREQ2) cannot contain response fields ('status', 'outputs', 'execution')")
    elif magic == MAGIC_RES:
        if "status" not in json_obj:
            raise ValueError("Response frame (PCADRES2) must contain 'status'")
        if "inputs" in json_obj or "params" in json_obj:
            raise ValueError("Response frame (PCADRES2) cannot contain request fields ('inputs', 'params')")

    json_bytes = json.dumps(json_obj, separators=(",", ":"), ensure_ascii=False).encode("utf-8")
    header_length = len(json_bytes)
    if header_length > MAX_HEADER_BYTES:
        raise ValueError(f"Header length {header_length} exceeds budget {MAX_HEADER_BYTES}")

    section_count = len(sections_data)
    if not (0 <= section_count <= MAX_SECTIONS):
        raise ValueError(f"Section count {section_count} exceeds limit {MAX_SECTIONS}")

    table_bytes = bytearray()
    payload_bytes = bytearray()
    current_offset = 0

    for kind, data in sections_data:
        if kind not in (KIND_ICC, KIND_F32):
            raise ValueError(f"Unknown section kind: {kind}")
        data_len = len(data)
        if current_offset % 4 != 0:
            pad = 4 - (current_offset % 4)
            payload_bytes.extend(b"\x00" * pad)
            current_offset += pad

        table_bytes.extend(struct.pack("<IIII", kind, 0, current_offset, data_len))
        payload_bytes.extend(data)
        current_offset += data_len

    if len(payload_bytes) % 4 != 0:
        pad = 4 - (len(payload_bytes) % 4)
        payload_bytes.extend(b"\x00" * pad)

    frame = bytearray()
    frame.extend(magic)
    frame.extend(struct.pack("<I", header_length))
    frame.extend(json_bytes)
    frame.extend(struct.pack("<I", section_count))
    frame.extend(table_bytes)
    frame.extend(payload_bytes)

    return bytes(frame)


def strict_decode_frame(
    raw: bytes, expected_magic: bytes | None = None
) -> tuple[bytes, dict[str, Any], list[dict[str, Any]], bytes]:
    """
    Strictly decode a v2 wire frame with direction, framing, budget, and section correlation checks.

    Returns:
      (magic, json_header, sections_meta, payload_bytes)
    Raises:
      ValueError on any framing, budget, structural, direction, or correlation violation.
    """
    if len(raw) < 16:
        raise ValueError(f"Frame length {len(raw)} < minimum header size 16")

    magic = raw[:8]
    if magic not in (MAGIC_REQ, MAGIC_RES):
        raise ValueError(f"Unrecognized wire magic: {magic!r}")

    if expected_magic is not None and magic != expected_magic:
        raise ValueError(f"Frame magic mismatch: expected {expected_magic!r}, got {magic!r}")

    header_length = struct.unpack("<I", raw[8:12])[0]
    if header_length < 2:
        raise ValueError(f"Header length {header_length} is below minimum JSON size (2 bytes)")
    if header_length > MAX_HEADER_BYTES:
        raise ValueError(f"Header length {header_length} exceeds 1 MiB limit")

    json_end = 12 + header_length
    if json_end + 4 > len(raw):
        raise ValueError(f"Frame truncated before section count: need {json_end + 4}, got {len(raw)}")

    json_header = strict_json_loads(raw[12:json_end])

    section_count = struct.unpack("<I", raw[json_end:json_end + 4])[0]
    if section_count > MAX_SECTIONS:
        raise ValueError(f"Section count {section_count} exceeds limit of 64")

    table_start = json_end + 4
    table_end = table_start + section_count * 16
    if table_end > len(raw):
        raise ValueError(f"Frame truncated inside section table: need {table_end}, got {len(raw)}")

    sections_meta: list[dict[str, Any]] = []
    previous_end = 0
    icc_total = 0
    sample_total = 0

    for i in range(section_count):
        entry_offset = table_start + i * 16
        kind, reserved, offset, length = struct.unpack("<IIII", raw[entry_offset:entry_offset + 16])
        if reserved != 0:
            raise ValueError(f"Section {i} reserved field is non-zero ({reserved})")
        if offset % 4 != 0:
            raise ValueError(f"Section {i} offset {offset} is not 4-byte aligned")
        if length == 0:
            raise ValueError(f"Section {i} length is zero (empty section)")
        expected_offset = (previous_end + 3) // 4 * 4
        if offset != expected_offset:
            raise ValueError(f"Section {i} offset {offset} != expected contiguous offset {expected_offset}")
        if kind not in (KIND_ICC, KIND_F32):
            raise ValueError(f"Section {i} has invalid kind: {kind}")

        previous_end = offset + length
        if previous_end > MAX_PAYLOAD_BYTES:
            raise ValueError(f"Section {i} end {previous_end} exceeds payload budget {MAX_PAYLOAD_BYTES}")

        if kind == KIND_ICC:
            if length > MAX_ICC_SECTION_BYTES:
                raise ValueError(f"ICC section {i} length {length} exceeds {MAX_ICC_SECTION_BYTES}")
            icc_total += length
        else:
            if length % 4 != 0:
                raise ValueError(f"f32 section {i} length {length} is not a multiple of 4")
            sample_total += length // 4

        sections_meta.append({
            "index": i,
            "kind": kind,
            "kind_str": KIND_INT_TO_STR[kind],
            "offset": offset,
            "length": length,
        })

    if icc_total > MAX_AGGREGATE_ICC_BYTES:
        raise ValueError(f"Aggregate ICC bytes {icc_total} exceeds {MAX_AGGREGATE_ICC_BYTES}")
    if sample_total > MAX_F32_SAMPLES:
        raise ValueError(f"Aggregate f32 samples {sample_total} exceeds {MAX_F32_SAMPLES}")

    payload = raw[table_end:]
    if len(payload) < previous_end:
        raise ValueError(f"Payload truncated: need at least {previous_end} bytes, got {len(payload)}")

    # Section Correlation with JSON Header
    json_sections = json_header.get("sections", [])
    if len(json_sections) != section_count:
        raise ValueError(
            f"Section count mismatch: binary table has {section_count}, JSON header has {len(json_sections)}"
        )

    for i, (b_sec, j_sec) in enumerate(zip(sections_meta, json_sections)):
        if b_sec["kind_str"] != j_sec.get("kind"):
            raise ValueError(f"Section {i} kind mismatch: binary={b_sec['kind_str']} vs json={j_sec.get('kind')}")
        if b_sec["offset"] != j_sec.get("offset"):
            raise ValueError(f"Section {i} offset mismatch: binary={b_sec['offset']} vs json={j_sec.get('offset')}")
        if b_sec["length"] != j_sec.get("length"):
            raise ValueError(f"Section {i} length mismatch: binary={b_sec['length']} vs json={j_sec.get('length')}")

    # Direction and Schema Boundary Enforcement
    msg_type = json_header.get("type")
    if msg_type not in ("hello", "control", "run"):
        raise ValueError(f"Invalid message type: {msg_type!r}")

    if magic == MAGIC_REQ:
        # Request Frame constraints
        if "status" in json_header:
            raise ValueError("Request frame (PCADREQ2) contains forbidden 'status' field")
        if "outputs" in json_header or "execution" in json_header:
            raise ValueError("Request frame (PCADREQ2) contains forbidden response fields ('outputs', 'execution')")

        if msg_type in ("hello", "control"):
            if section_count != 0 or len(payload) != 0:
                raise ValueError(f"Request type={msg_type} must carry 0 sections and 0 payload (got {section_count} sections)")
        elif msg_type == "run":
            if section_count < 1:
                raise ValueError("Run request must carry at least 1 input section")
            if "inputs" not in json_header:
                raise ValueError("Run request missing required 'inputs' object")
            if "source" not in json_header["inputs"]:
                raise ValueError("Run request inputs must declare 'source' image port")

        ports = json_header.get("inputs", {})

    else:
        # Response Frame constraints (MAGIC_RES)
        if "status" not in json_header:
            raise ValueError("Response frame (PCADRES2) missing required 'status' field")
        if "inputs" in json_header or "params" in json_header:
            raise ValueError("Response frame (PCADRES2) contains forbidden request fields ('inputs', 'params')")

        msg_status = json_header.get("status")

        if msg_type in ("hello", "control"):
            if section_count != 0 or len(payload) != 0:
                raise ValueError(f"Response type={msg_type} must carry 0 sections and 0 payload (got {section_count} sections)")
        elif msg_type == "run":
            if msg_status == "ok":
                if section_count < 1:
                    raise ValueError("OK run response must carry at least 1 output section")
                if "outputs" not in json_header:
                    raise ValueError("OK run response missing required 'outputs' object")
                if "execution" not in json_header:
                    raise ValueError("OK run response missing required 'execution' report")
            else:
                if section_count != 0 or len(payload) != 0:
                    raise ValueError(f"Non-ok run response (status={msg_status}) must carry 0 sections and 0 payload")
                if "outputs" in json_header or "execution" in json_header:
                    raise ValueError(f"Non-ok run response (status={msg_status}) must not carry 'outputs' or 'execution'")
                if "error" not in json_header:
                    raise ValueError(f"Non-ok run response (status={msg_status}) missing required 'error' field")

        ports = json_header.get("outputs", {})

    # Port section reference and sample product checks
    seen_sections: set[int] = set()
    for port_name, port in ports.items():
        if not isinstance(port, dict):
            continue

        # Coordinate and dimension bounds
        x = port.get("x", 0)
        y = port.get("y", 0)
        w = port.get("width", 0)
        h = port.get("height", 0)
        ch = port.get("channels", 0)

        if not (-2147483648 <= x + w <= 2147483647 and -2147483648 <= y + h <= 2147483647):
            raise ValueError(f"Port '{port_name}' rectangle coordinates exceed 32-bit signed integer range")
        if not (1 <= w <= 16384 and 1 <= h <= 16384):
            raise ValueError(f"Port '{port_name}' dimensions ({w}x{h}) exceed [1, 16384] bound")
        if not (1 <= ch <= 5):
            raise ValueError(f"Port '{port_name}' channels ({ch}) exceed [1, 5] bound")

        samples = w * h * ch
        if samples > MAX_F32_SAMPLES:
            raise ValueError(
                f"Port '{port_name}' samples ({samples}) exceeds maximum allowed single tensor samples {MAX_F32_SAMPLES}"
            )

        idx = port.get("data_section_index")
        if idx is not None:
            if idx < 0 or idx >= section_count:
                raise ValueError(f"Port '{port_name}' data_section_index {idx} out of range [0, {section_count})")
            sec = sections_meta[idx]
            if sec["kind"] != KIND_F32:
                raise ValueError(f"Port '{port_name}' data section {idx} is not float32")
            expected_bytes = w * h * ch * 4
            if sec["length"] != expected_bytes:
                raise ValueError(
                    f"Port '{port_name}' sample product ({expected_bytes} bytes) != section length ({sec['length']} bytes)"
                )
            if idx in seen_sections:
                raise ValueError(f"Section {idx} aliased by multiple ports (conflict at '{port_name}')")
            seen_sections.add(idx)

        if port.get("kind") == "mask":
            if port.get("color_encoding") is not None:
                raise ValueError(f"Mask port '{port_name}' forbidden to declare 'color_encoding'")

        icc_idx = port.get("icc_section_index")
        if icc_idx is not None:
            if port.get("kind") == "mask":
                raise ValueError(f"Mask port '{port_name}' forbidden to reference ICC section")
            if icc_idx < 0 or icc_idx >= section_count:
                raise ValueError(f"Port '{port_name}' icc_section_index {icc_idx} out of range [0, {section_count})")
            sec = sections_meta[icc_idx]
            if sec["kind"] != KIND_ICC:
                raise ValueError(f"Port '{port_name}' ICC section {icc_idx} is not kind icc")
            if icc_idx in seen_sections:
                raise ValueError(f"ICC Section {icc_idx} aliased by multiple ports")
            seen_sections.add(icc_idx)

    # In run with sections, all declared sections must be referenced
    if msg_type == "run" and section_count > 0:
        if seen_sections != set(range(section_count)):
            unref = set(range(section_count)) - seen_sections
            raise ValueError(f"Unreferenced orphan sections in frame: {sorted(unref)}")

    return magic, json_header, sections_meta, payload


def pair_check(request: dict[str, Any], response: dict[str, Any]) -> None:
    """
    Validate request-response correlation and policy execution constraints.

    Enforces:
      1. Direction validity: request must be a request; response must be a response.
      2. Request ID and message type correlation.
      3. Handshake wire-version and feature intersection contract.
      4. Control action correlation, readiness constraints, and resource scoping.
      5. Error code matching for failure states.
      6. Execution report consistency, device policy adherence, same-geometry,
         serial timing conservation, and synthetic backend identity verification.
    """
    # 1. Direction validation
    if "status" in request or "outputs" in request or "execution" in request:
        raise ValueError("First argument to pair_check must be a request, but contains response fields ('status', 'outputs', 'execution')")
    if "status" not in response:
        raise ValueError("Second argument to pair_check must be a response, but missing required 'status' field")
    if "inputs" in response or "params" in response:
        raise ValueError("Second argument to pair_check must be a response, but contains request fields ('inputs', 'params')")

    # 2. Correlation
    if request.get("request_id") != response.get("request_id"):
        raise ValueError(
            f"Correlation mismatch: req_id {request.get('request_id')} != res_id {response.get('request_id')}"
        )
    if request.get("type") != response.get("type"):
        raise ValueError(f"Type mismatch: req type {request.get('type')} != res type {response.get('type')}")

    req_type = request.get("type")
    res_status = response.get("status")

    # 3. Hello negotiation
    if req_type == "hello" and res_status == "negotiated":
        offer = request.get("negotiation", {})
        selected = response.get("negotiation", {})
        if selected.get("selected_wire_version") not in offer.get("wire_versions", []):
            raise ValueError("Negotiated wire version not offered in request")
        enabled = set(selected.get("enabled_features", []))
        supported = set(offer.get("supported_features", []))
        required = set(offer.get("required_features", []))
        if not (enabled <= supported):
            raise ValueError(f"Enabled features {enabled - supported} not in supported offer")
        if not (required <= enabled):
            raise ValueError(f"Required features {required - enabled} not enabled")
        if not (BASE_FEATURES <= enabled):
            raise ValueError(f"Base features {BASE_FEATURES - enabled} missing from negotiated set")

    # 4. Control actions
    if req_type == "control":
        if request.get("action") != response.get("action"):
            raise ValueError(f"Action mismatch: req action {request.get('action')} != res action {response.get('action')}")
        ctrl_action = request.get("action")

        if res_status == "ready":
            readiness = response.get("readiness")
            allowed_readiness = {
                "probe": {"declared", "files_present", "files_verified"},
                "prepare": {"files_verified", "session_created"},
            }
            if ctrl_action in allowed_readiness:
                if readiness not in allowed_readiness[ctrl_action]:
                    raise ValueError(f"Illegal readiness '{readiness}' for action '{ctrl_action}'")

        if "resources" in response:
            for entry in response["resources"]:
                if (
                    entry.get("operation") != request.get("operation")
                    or entry.get("operation_version") != request.get("operation_version")
                ):
                    raise ValueError("Resource entry operation scope mismatch")
                if "resource" in request and entry.get("identity") != request.get("resource"):
                    raise ValueError("Resource entry identity mismatch with request scope")

    # 5. Status / Error code verification for failures
    status_error_codes = {
        "needs_model": "resource_missing",
        "needs_permission": "permission_required",
        "unavailable": "device_unavailable",
        "cancelled": "cancelled",
        "timeout": "deadline_exceeded",
    }
    if res_status in status_error_codes:
        err = response.get("error", {})
        expected_code = status_error_codes[res_status]
        if err.get("code") != expected_code:
            raise ValueError(f"Status '{res_status}' error code '{err.get('code')}' != expected '{expected_code}'")

    # 6. Run inference verification
    if req_type == "run" and res_status == "ok":
        exec_rep = response.get("execution", {})
        req_policy = request.get("device_policy", {})

        if exec_rep.get("requested_policy") != req_policy:
            raise ValueError("Execution report requested_policy does not match request device_policy")
        if exec_rep.get("resource") != request.get("resource"):
            raise ValueError("Execution report resource does not match request resource")
        if exec_rep.get("backend") not in req_policy.get("allowed_backends", []):
            raise ValueError(f"Execution backend '{exec_rep.get('backend')}' violates allowed_backends")
        if exec_rep.get("graph_fallback") and not req_policy.get("allow_graph_fallback"):
            raise ValueError("Execution report claimed graph_fallback when allow_graph_fallback is false")

        mode = req_policy.get("mode")
        devices = exec_rep.get("devices", [])
        if mode == "exact":
            target = req_policy.get("target_device")
            if not all(d.get("id") == target for d in devices):
                raise ValueError("Graph device violates exact target_device policy")
        elif mode == "require_kind":
            target_kind = req_policy.get("target_kind")
            if not all(d.get("kind") == target_kind for d in devices):
                raise ValueError("Graph device violates require_kind target_kind policy")

        # Serial timing conservation
        timing = exec_rep.get("timing", {})
        parts_sum = (
            timing.get("cpu_preprocess_ms", 0)
            + timing.get("graph_execution_ms", 0)
            + timing.get("cpu_postprocess_ms", 0)
        )
        if timing.get("total_ms", 0) + 0.001 < parts_sum:
            raise ValueError(f"Inconsistent serial timing: total_ms {timing.get('total_ms')} < parts sum {parts_sum}")

        # same_geometry output constraint
        source = request.get("inputs", {}).get("source", {})
        for port_name, out_port in response.get("outputs", {}).items():
            for dim in ("x", "y", "width", "height"):
                if out_port.get(dim) != source.get(dim):
                    raise ValueError(
                        f"Output port '{port_name}' {dim} ({out_port.get(dim)}) != source {dim} ({source.get(dim)})"
                    )

        # Synthetic backend identity enforcement
        evidence = exec_rep.get("evidence")
        if evidence == "synthetic_fixture":
            if (
                not str(exec_rep.get("runtime_id", "")).startswith("example.synthetic")
                or not str(exec_rep.get("backend", "")).startswith("example.synthetic")
                or not all(str(d.get("id", "")).startswith("example.synthetic") for d in devices)
                or not str(exec_rep.get("resource", {}).get("id", "")).startswith("example.synthetic.")
            ):
                raise ValueError("Synthetic fixture execution report uses non-example identity")
        else:
            if exec_rep.get("resource", {}).get("manifest_digest") == ZERO_DIGEST:
                raise ValueError("Sentinel all-zero digest in non-synthetic report resource")
            if exec_rep.get("configuration_digest") == ZERO_DIGEST:
                raise ValueError("Sentinel all-zero digest in non-synthetic report configuration_digest")
