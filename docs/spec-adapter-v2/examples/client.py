#!/usr/bin/env python3
"""
Standalone Reference Client for PhotoCraft Adapter v2 Wire Protocol.

This reference implementation demonstrates:
  1. PCADREQ2 request encoding with binary section table and payload using strict codec.
  2. Subprocess invocation via `--action <probe|prepare|run|hello|listResources|evict> --request <req.bin> --response <resp.bin>`.
  3. PCADRES2 response decoding with strict memory, limit, and section correlation checks.
  4. Contract verification via pair_check ensuring device policy and request correlation.
"""

import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

# Ensure codec can be imported from parent directory
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from codec import (
    KIND_F32,
    KIND_ICC,
    MAGIC_REQ,
    MAGIC_RES,
    encode_frame,
    pair_check,
    strict_decode_frame,
)


class AdapterV2Client:
    def __init__(self, worker_executable: str | Path, timeout_seconds: float = 30.0):
        self.worker_executable = Path(worker_executable).resolve()
        self.timeout = timeout_seconds

    @staticmethod
    def encode_request(json_header: dict[str, Any], sections: list[tuple[int, bytes]]) -> bytes:
        """Encode a PCADREQ2 binary frame with strict formatting and alignment."""
        return encode_frame(MAGIC_REQ, json_header, sections)

    @staticmethod
    def decode_response(raw: bytes) -> tuple[dict[str, Any], list[dict[str, Any]], bytes]:
        """Decode a PCADRES2 binary frame strictly."""
        magic, json_header, sections_meta, payload = strict_decode_frame(raw)
        if magic != MAGIC_RES:
            raise ValueError(f"Expected PCADRES2, got {magic!r}")
        return json_header, sections_meta, payload

    def invoke(
        self, action: str, request_frame: bytes, verify_pair: bool = True
    ) -> tuple[dict[str, Any], list[dict[str, Any]], bytes]:
        """Invoke external companion worker using isolated temp files and verify pair contract."""
        _, req_json, _, _ = strict_decode_frame(request_frame)

        with tempfile.TemporaryDirectory(prefix="pcad_v2_") as tmp_dir:
            tmp_path = Path(tmp_dir)
            req_file = tmp_path / "request.bin"
            res_file = tmp_path / "response.bin"

            req_file.write_bytes(request_frame)

            if str(self.worker_executable).endswith(".py"):
                cmd = [
                    sys.executable,
                    str(self.worker_executable),
                    "--action",
                    action,
                    "--request",
                    str(req_file),
                    "--response",
                    str(res_file),
                ]
            else:
                cmd = [
                    str(self.worker_executable),
                    "--action",
                    action,
                    "--request",
                    str(req_file),
                    "--response",
                    str(res_file),
                ]

            proc = subprocess.run(cmd, capture_output=True, timeout=self.timeout)
            if proc.returncode != 0:
                raise RuntimeError(
                    f"Worker process failed with exit code {proc.returncode}:\n"
                    f"{proc.stderr.decode('utf-8', errors='replace')}"
                )

            if not res_file.exists():
                raise FileNotFoundError(f"Worker did not create response file at {res_file}")

            res_bytes = res_file.read_bytes()
            res_json, res_sections, res_payload = self.decode_response(res_bytes)

            if verify_pair:
                pair_check(req_json, res_json)

            return res_json, res_sections, res_payload


class InProcessAdapterClient:
    """
    Direct in-process client communicating with a WorkerSession without subprocess overhead.
    """

    def __init__(self, session: Any):
        self.session = session

    def invoke(
        self, request_frame: bytes, verify_pair: bool = True
    ) -> tuple[dict[str, Any], list[dict[str, Any]], bytes]:
        """Invoke in-process worker session and optionally verify pair contract."""
        _, req_json, _, _ = strict_decode_frame(request_frame, expected_magic=MAGIC_REQ)
        resp_bytes = self.session.handle_request_frame(request_frame)
        _, res_json, res_sections, res_payload = strict_decode_frame(
            resp_bytes, expected_magic=MAGIC_RES
        )

        if verify_pair:
            pair_check(req_json, res_json)

        return res_json, res_sections, res_payload

    def close(self) -> None:
        """Explicitly close underlying worker session."""
        if hasattr(self.session, "close"):
            self.session.close()

    @property
    def is_closed(self) -> bool:
        """Check if underlying worker session is closed."""
        return bool(getattr(self.session, "is_closed", False))


if __name__ == "__main__":
    print("PhotoCraft Adapter v2 Reference Client module loaded.")
