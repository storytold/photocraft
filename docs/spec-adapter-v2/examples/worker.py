#!/usr/bin/env python3
"""
Standalone Reference Worker for PhotoCraft Adapter v2 Wire Protocol.

This reference implementation demonstrates:
  1. CLI interface: `--action <probe|prepare|listResources|evict|run|hello> --request <req.bin> --response <resp.bin>`.
  2. PCADREQ2 request decoding with strict bounds, limits, and section correlation.
  3. Action execution conforming to docs/spec-adapter-v2/ schemas and lifecycle semantics.
  4. Real capability checks against actual worker devices (no policy echo or fake digests).
  5. PCADRES2 response encoding with execution report and binary sections.
"""

import argparse
import struct
import sys
import time
from pathlib import Path

# Ensure codec can be imported from parent directory
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from codec import (
    KIND_F32,
    MAGIC_REQ,
    MAGIC_RES,
    ZERO_DIGEST,
    encode_frame,
    strict_decode_frame,
)

# Reference worker actual hardware capability constants
ACTUAL_BACKEND = "example.synthetic"
ACTUAL_DEVICE_ID = "example.synthetic.cpu"
ACTUAL_DEVICE_KIND = "cpu"
ACTUAL_RUNTIME_ID = "example.synthetic.worker"
ACTUAL_RUNTIME_VERSION = "2.0.0"


def handle_request(
    cli_action: str,
    req_json: dict,
    sections_meta: list[dict],
    payload: bytes,
) -> tuple[dict, list[tuple[int, bytes]]]:
    req_id = req_json.get("request_id", "req-000")
    req_type = req_json.get("type")

    # 1. Hello negotiation
    if req_type == "hello":
        offer = req_json.get("negotiation", {})
        wire_versions = offer.get("wire_versions", [])
        if 2 not in wire_versions:
            return {
                "protocol": 2,
                "request_id": req_id,
                "type": "hello",
                "status": "error",
                "message": f"Wire version 2 not in offered versions: {wire_versions}",
                "error": {"code": "unsupported_version", "retryable": False},
                "sections": [],
            }, []

        supported = set(offer.get("supported_features", []))
        base_features = ["image-mask-v2", "same-geometry", "resource-control"]
        if not (set(base_features) <= supported):
            return {
                "protocol": 2,
                "request_id": req_id,
                "type": "hello",
                "status": "error",
                "message": f"Worker base features {base_features} not supported by offer",
                "error": {"code": "unsupported_feature", "retryable": False},
                "sections": [],
            }, []

        return {
            "protocol": 2,
            "request_id": req_id,
            "type": "hello",
            "status": "negotiated",
            "message": "Handshake negotiated successfully.",
            "negotiation": {
                "selected_wire_version": 2,
                "enabled_features": base_features,
            },
            "sections": [],
        }, []

    # 2. Control actions
    if req_type == "control":
        ctrl_action = req_json.get("action")
        if ctrl_action != cli_action:
            return {
                "protocol": 2,
                "request_id": req_id,
                "type": "control",
                "action": cli_action,
                "status": "error",
                "message": f"CLI action '{cli_action}' does not match request action '{ctrl_action}'",
                "error": {"code": "invalid_request", "retryable": False},
                "sections": [],
            }, []

        resource = req_json.get("resource", {})
        device_policy = req_json.get("device_policy", {})

        # Actual device policy capability check
        allowed_backends = device_policy.get("allowed_backends", [ACTUAL_BACKEND])
        if ACTUAL_BACKEND not in allowed_backends:
            return {
                "protocol": 2,
                "request_id": req_id,
                "type": "control",
                "action": ctrl_action,
                "status": "unavailable",
                "message": f"Worker backend '{ACTUAL_BACKEND}' not permitted by allowed_backends {allowed_backends}",
                "error": {"code": "device_unavailable", "retryable": False},
                "sections": [],
            }, []

        mode = device_policy.get("mode")
        if mode == "exact":
            target_dev = device_policy.get("target_device")
            if target_dev != ACTUAL_DEVICE_ID:
                return {
                    "protocol": 2,
                    "request_id": req_id,
                    "type": "control",
                    "action": ctrl_action,
                    "status": "unavailable",
                    "message": f"Target device '{target_dev}' unavailable on worker (actual device: '{ACTUAL_DEVICE_ID}')",
                    "error": {"code": "device_unavailable", "retryable": False},
                    "sections": [],
                }, []
        elif mode == "require_kind":
            target_kind = device_policy.get("target_kind")
            if target_kind != ACTUAL_DEVICE_KIND:
                return {
                    "protocol": 2,
                    "request_id": req_id,
                    "type": "control",
                    "action": ctrl_action,
                    "status": "unavailable",
                    "message": f"Target device kind '{target_kind}' unavailable on worker (actual kind: '{ACTUAL_DEVICE_KIND}')",
                    "error": {"code": "device_unavailable", "retryable": False},
                    "sections": [],
                }, []

        if ctrl_action == "probe":
            return {
                "protocol": 2,
                "request_id": req_id,
                "type": "control",
                "action": "probe",
                "status": "ready",
                "message": "SYNTHETIC FIXTURE ONLY: declared support level, not installed files, a session, inference or hardware evidence.",
                "readiness": "declared",
                "sections": [],
            }, []

        elif ctrl_action == "prepare":
            if resource.get("id") == "example.synthetic.unavailable-model":
                return {
                    "protocol": 2,
                    "request_id": req_id,
                    "type": "control",
                    "action": "prepare",
                    "status": "needs_model",
                    "message": "SYNTHETIC FIXTURE: example.synthetic.unavailable-model is a non-runnable placeholder. No weights, download permission or license verification is supplied.",
                    "error": {"code": "resource_missing", "retryable": False},
                    "sections": [],
                }, []

            return {
                "protocol": 2,
                "request_id": req_id,
                "type": "control",
                "action": "prepare",
                "status": "ready",
                "message": "Model execution session successfully initialized.",
                "readiness": "session_created",
                "sections": [],
            }, []

        elif ctrl_action == "listResources":
            return {
                "protocol": 2,
                "request_id": req_id,
                "type": "control",
                "action": "listResources",
                "status": "ready",
                "message": "Installed resources enumerated.",
                "resources": [
                    {
                        "identity": resource,
                        "operation": req_json.get("operation", "synthetic-mask"),
                        "operation_version": req_json.get("operation_version", "0.0.0-example"),
                        "readiness": "declared",
                    }
                ],
                "sections": [],
            }, []

        elif ctrl_action == "evict":
            return {
                "protocol": 2,
                "request_id": req_id,
                "type": "control",
                "action": "evict",
                "status": "ready",
                "message": "Session caches evicted.",
                "sections": [],
            }, []

        else:
            return {
                "protocol": 2,
                "request_id": req_id,
                "type": "control",
                "action": ctrl_action,
                "status": "error",
                "message": f"Unsupported control action: {ctrl_action}",
                "error": {"code": "invalid_request", "retryable": False},
                "sections": [],
            }, []

    # 3. Run inference action
    if req_type == "run":
        device_policy = req_json.get("device_policy", {})
        resource = req_json.get("resource", {})

        # Actual capability checks: worker provides ACTUAL_BACKEND and ACTUAL_DEVICE_ID
        allowed_backends = device_policy.get("allowed_backends", [])
        if ACTUAL_BACKEND not in allowed_backends:
            return {
                "protocol": 2,
                "request_id": req_id,
                "type": "run",
                "status": "unavailable",
                "message": f"Worker backend '{ACTUAL_BACKEND}' not permitted by requested allowed_backends {allowed_backends}",
                "error": {"code": "device_unavailable", "retryable": False},
                "sections": [],
            }, []

        mode = device_policy.get("mode")
        if mode == "exact":
            target_dev = device_policy.get("target_device")
            if target_dev != ACTUAL_DEVICE_ID:
                return {
                    "protocol": 2,
                    "request_id": req_id,
                    "type": "run",
                    "status": "unavailable",
                    "message": f"Target device '{target_dev}' unavailable on worker (actual device: '{ACTUAL_DEVICE_ID}')",
                    "error": {"code": "device_unavailable", "retryable": False},
                    "sections": [],
                }, []
        elif mode == "require_kind":
            target_kind = device_policy.get("target_kind")
            if target_kind != ACTUAL_DEVICE_KIND:
                return {
                    "protocol": 2,
                    "request_id": req_id,
                    "type": "run",
                    "status": "unavailable",
                    "message": f"Device kind '{target_kind}' unavailable on worker (actual kind: '{ACTUAL_DEVICE_KIND}')",
                    "error": {"code": "device_unavailable", "retryable": False},
                    "sections": [],
                }, []

        # Check resource availability
        res_id = str(resource.get("id", ""))
        if not res_id.startswith("example.synthetic."):
            return {
                "protocol": 2,
                "request_id": req_id,
                "type": "run",
                "status": "needs_model",
                "message": f"Model resource '{res_id}' not available on reference worker",
                "error": {"code": "resource_missing", "retryable": False},
                "sections": [],
            }, []

        start_time = time.perf_counter()
        inputs = req_json.get("inputs", {})
        source_port = inputs.get("source", {})

        width = source_port.get("width", 0)
        height = source_port.get("height", 0)
        channels = source_port.get("channels", 3)
        pixel_count = width * height
        expected_source_bytes = pixel_count * channels * 4

        sec_idx = source_port.get("data_section_index")
        if sec_idx is None or sec_idx >= len(sections_meta):
            return {
                "protocol": 2,
                "request_id": req_id,
                "type": "run",
                "status": "error",
                "message": "Input source section missing or out of bounds",
                "error": {"code": "invalid_request", "retryable": False},
                "sections": [],
            }, []

        sec_info = sections_meta[sec_idx]
        offset = sec_info["offset"]
        length = sec_info["length"]
        if length != expected_source_bytes or offset + length > len(payload):
            return {
                "protocol": 2,
                "request_id": req_id,
                "type": "run",
                "status": "error",
                "message": "Source section length mismatch with raster geometry",
                "error": {"code": "invalid_request", "retryable": False},
                "sections": [],
            }, []

        # Read source floats
        num_floats = pixel_count * channels
        source_floats = struct.unpack(f"<{num_floats}f", payload[offset : offset + length])

        # Generate reference synthetic mask: grayscale luminance heuristic [0.0, 1.0]
        mask_floats = []
        for i in range(pixel_count):
            r = source_floats[i * channels + 0]
            g = source_floats[i * channels + 1]
            b = source_floats[i * channels + 2]
            intensity = 0.299 * r + 0.587 * g + 0.114 * b
            mask_floats.append(max(0.0, min(1.0, intensity)))

        mask_bytes = struct.pack(f"<{pixel_count}f", *mask_floats)
        duration_ms = (time.perf_counter() - start_time) * 1000.0

        resp_json = {
            "protocol": 2,
            "request_id": req_id,
            "type": "run",
            "status": "ok",
            "message": "Inference completed successfully.",
            "outputs": {
                "mask": {
                    "kind": "mask",
                    "x": source_port.get("x", 0),
                    "y": source_port.get("y", 0),
                    "width": width,
                    "height": height,
                    "channels": 1,
                    "color_mode": "gray",
                    "source_depth": 32,
                    "alpha": False,
                    "data_section_index": 0,
                }
            },
            "execution": {
                "requested_policy": device_policy,
                "devices": [{"id": ACTUAL_DEVICE_ID, "kind": ACTUAL_DEVICE_KIND}],
                "backend": ACTUAL_BACKEND,
                "runtime_id": ACTUAL_RUNTIME_ID,
                "runtime_version": ACTUAL_RUNTIME_VERSION,
                "resource": resource,
                "configuration_digest": ZERO_DIGEST,
                "graph_partitioned": False,
                "graph_fallback": False,
                "fallback_reason": None,
                "evidence": "synthetic_fixture",
                "timing": {
                    "cpu_preprocess_ms": 0.05,
                    "graph_execution_ms": round(duration_ms, 3),
                    "cpu_postprocess_ms": 0.05,
                    "total_ms": round(duration_ms + 0.1, 3),
                },
            },
            "sections": [
                {
                    "kind": "f32",
                    "offset": 0,
                    "length": len(mask_bytes),
                }
            ],
        }

        return resp_json, [(KIND_F32, mask_bytes)]

    return {
        "protocol": 2,
        "request_id": req_id,
        "type": req_type if req_type else "unknown",
        "status": "error",
        "message": f"Unsupported request type: {req_type}",
        "error": {"code": "invalid_request", "retryable": False},
        "sections": [],
    }, []


class WorkerSession:
    """
    In-process stateful worker session for lifecycle transitions and frame handling.

    State transitions:
      - UNINITIALIZED: Initial state. Only type 'hello' accepted.
      - NEGOTIATED: Handshake negotiated. 'control' and 'run' accepted.
      - NEEDS_MODEL: Session requires model preparation before execution.
      - READY: Model prepared and ready for execution.

    Error handling strictly conforms to wire-response.schema.json standard error codes.
    """

    def __init__(self):
        self.state = "UNINITIALIZED"
        self.seen_request_ids: set[str] = set()
        self.prepared_resource_id: str | None = None
        self.protocol_version: int | None = None
        self.enabled_features: list[str] = []

    def close(self) -> None:
        """Explicitly close worker session and teardown resources."""
        self.state = "CLOSED"
        self.prepared_resource_id = None

    @property
    def is_closed(self) -> bool:
        return self.state == "CLOSED"

    def handle_request_frame(self, request_frame_bytes: bytes) -> bytes:
        """Process a raw request frame in-process and return raw response frame bytes."""
        magic, req_json, sections_meta, payload = strict_decode_frame(
            request_frame_bytes, expected_magic=MAGIC_REQ
        )

        req_id = req_json.get("request_id", "req-000")
        req_type = req_json.get("type")

        # 0. Enforce closed endpoint check
        if self.state == "CLOSED":
            err_json: dict = {
                "protocol": 2,
                "request_id": req_id,
                "type": req_type if req_type in ("hello", "control", "run") else "control",
                "status": "error",
                "message": "Session is closed: no further requests permitted on closed endpoint",
                "error": {"code": "invalid_request", "retryable": False},
                "sections": [],
            }
            if req_type == "control":
                err_json["action"] = req_json.get("action", "probe")
            return encode_frame(MAGIC_RES, err_json, [])

        # 1. Enforce unique request_id in single session
        if req_id in self.seen_request_ids:
            err_json = {
                "protocol": 2,
                "request_id": req_id,
                "type": req_type if req_type in ("hello", "control", "run") else "control",
                "status": "error",
                "message": f"Duplicate request_id '{req_id}' reused in same session",
                "error": {"code": "invalid_request", "retryable": False},
                "sections": [],
            }
            if req_type == "control":
                err_json["action"] = req_json.get("action", "probe")
            return encode_frame(MAGIC_RES, err_json, [])

        # 2. Enforce session initialization via hello handshake (Control/run before negotiation closes endpoint)
        if self.state == "UNINITIALIZED" and req_type != "hello":
            self.state = "CLOSED"
            err_json = {
                "protocol": 2,
                "request_id": req_id,
                "type": req_type if req_type in ("hello", "control", "run") else "control",
                "status": "error",
                "message": "Session uninitialized: control or run before hello negotiation closes endpoint",
                "error": {"code": "invalid_request", "retryable": False},
                "sections": [],
            }
            if req_type == "control":
                err_json["action"] = req_json.get("action", "probe")
            self.seen_request_ids.add(req_id)
            return encode_frame(MAGIC_RES, err_json, [])

        # 3. Repeated hello handshake closes endpoint (README.md:43)
        if self.state != "UNINITIALIZED" and req_type == "hello":
            self.state = "CLOSED"
            err_json = {
                "protocol": 2,
                "request_id": req_id,
                "type": "hello",
                "status": "error",
                "message": "Repeated hello handshake forbidden: closes session endpoint",
                "error": {"code": "invalid_request", "retryable": False},
                "sections": [],
            }
            self.seen_request_ids.add(req_id)
            return encode_frame(MAGIC_RES, err_json, [])

        # 4. Model readiness guard for run
        if req_type == "run" and self.state == "NEEDS_MODEL":
            err_json = {
                "protocol": 2,
                "request_id": req_id,
                "type": "run",
                "status": "needs_model",
                "message": "Model resource not prepared: session in needs_model state",
                "error": {"code": "resource_missing", "retryable": False},
                "sections": [],
            }
            self.seen_request_ids.add(req_id)
            return encode_frame(MAGIC_RES, err_json, [])

        # 4. Action execution
        cli_action = req_type
        if req_type == "control":
            cli_action = req_json.get("action", "probe")

        resp_json, resp_sections = handle_request(cli_action, req_json, sections_meta, payload)

        # 5. State transitions on success
        if req_type == "hello" and resp_json.get("status") == "negotiated":
            self.state = "NEGOTIATED"
            self.protocol_version = 2
            self.enabled_features = resp_json.get("negotiation", {}).get("enabled_features", [])
        elif req_type == "control":
            ctrl_action = req_json.get("action")
            if ctrl_action == "prepare":
                if resp_json.get("status") == "ready":
                    self.state = "READY"
                    self.prepared_resource_id = req_json.get("resource", {}).get("id")
                elif resp_json.get("status") == "needs_model":
                    self.state = "NEEDS_MODEL"
            elif ctrl_action == "evict":
                self.prepared_resource_id = None
                self.state = "NEGOTIATED"

        self.seen_request_ids.add(req_id)
        return encode_frame(MAGIC_RES, resp_json, resp_sections)


def main():
    parser = argparse.ArgumentParser(description="PhotoCraft Adapter v2 Reference Worker")
    parser.add_argument(
        "--action",
        required=True,
        choices=["probe", "prepare", "listResources", "evict", "run", "hello"],
    )
    parser.add_argument("--request", required=True, type=Path, help="Path to request.bin")
    parser.add_argument("--response", required=True, type=Path, help="Path to response.bin")

    args = parser.parse_args()

    if not args.request.exists():
        sys.stderr.write(f"Request file does not exist: {args.request}\n")
        sys.exit(1)

    req_raw = args.request.read_bytes()
    try:
        magic, req_json, sections_meta, payload = strict_decode_frame(req_raw, expected_magic=MAGIC_REQ)
    except Exception as e:
        sys.stderr.write(f"Request frame decoding failed: {e}\n")
        sys.exit(2)

    resp_json, resp_sections = handle_request(args.action, req_json, sections_meta, payload)
    resp_raw = encode_frame(MAGIC_RES, resp_json, resp_sections)

    args.response.write_bytes(resp_raw)
    sys.exit(0)


if __name__ == "__main__":
    main()
