#!/usr/bin/env python3
"""CI-only draft document checks, NOT a codec/binary/hardware conformance runner.

--static: standard library only, selected document semantic/inventory checks.
--schema: additionally requires pinned jsonschema and performs full Draft 2020-12
validation and negative mutations. Neither mode parses a binary frame or verifies
artifact bytes, RFC 8785 digests, licenses, a runtime or a production implementation.
"""

import argparse
import copy
import importlib.metadata
import json
import math
import os
from pathlib import Path
import sys
from urllib.parse import urldefrag

ROOT = Path(__file__).resolve().parent
SCHEMA_NAMES = {
    "common.schema.json", "manifest.schema.json", "model-resource.schema.json",
    "wire-request.schema.json", "wire-response.schema.json",
}
EXPECTED_FIXTURES = {
    "hello-request.json": "wire-request.schema.json",
    "hello-response.json": "wire-response.schema.json",
    "control-probe-request.json": "wire-request.schema.json",
    "control-prepare-request.json": "wire-request.schema.json",
    "run-subject-request.json": "wire-request.schema.json",
    "status-ready-response.json": "wire-response.schema.json",
    "status-needs-model-response.json": "wire-response.schema.json",
    "ok-mask-response.json": "wire-response.schema.json",
    "model-resource-example.json": "model-resource.schema.json",
    "package-manifest-example.json": "manifest.schema.json",
}
ZERO = "0" * 64
BASE_FEATURES = {"image-mask-v2", "same-geometry", "resource-control"}
PINS = {
    "attrs": "24.3.0", "jsonschema": "4.23.0",
    "jsonschema-specifications": "2024.10.1", "referencing": "0.35.1",
    "rpds-py": "0.22.3", "rfc3986-validator": "0.1.1",
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def no_duplicates(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"duplicate JSON key: {key}")
        result[key] = value
    return result


def reject_constant(value):
    raise ValueError(f"non-JSON numeric constant: {value}")


def tree_budget(value, depth=1, counter=None):
    if counter is None:
        counter = [0]
    counter[0] += 1
    require(depth <= 16 and counter[0] <= 100000, "JSON depth/node budget exceeded")
    if isinstance(value, dict):
        require(len(value) <= 512, "object property budget exceeded")
        for key, child in value.items():
            require(len(key.encode("utf-8")) <= 128, "JSON key byte budget exceeded")
            tree_budget(child, depth + 1, counter)
    elif isinstance(value, list):
        require(len(value) <= 512, "array item budget exceeded")
        for child in value:
            tree_budget(child, depth + 1, counter)
    elif isinstance(value, str):
        require(len(value) <= 4096, "string character budget exceeded")
        value.encode("utf-8", errors="strict")  # Reject unpaired surrogates.
    elif isinstance(value, float):
        require(math.isfinite(value), "non-finite JSON number")


def strict_load(path):
    require(path.is_file() and path.stat().st_size <= 1048576,
            f"missing or oversized JSON document: {path.name}")
    raw = path.read_bytes()
    require(not raw.startswith(b"\xef\xbb\xbf"), f"BOM is forbidden: {path.name}")
    text = raw.decode("utf-8", errors="strict")
    # Pre-scan nesting BEFORE json.loads, ignoring braces inside strings.
    nesting, quoted, escaped = 0, False, False
    for char in text:
        if quoted:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                quoted = False
        elif char == '"':
            quoted = True
        elif char in "[{":
            nesting += 1
            require(nesting <= 16, f"JSON nesting budget exceeded: {path.name}")
        elif char in "]}":
            nesting -= 1
    value = json.loads(text, object_pairs_hook=no_duplicates, parse_constant=reject_constant)
    tree_budget(value)
    require(isinstance(value, dict), f"document root must be object: {path.name}")
    return value


def walk(value):
    yield value
    if isinstance(value, dict):
        for child in value.values():
            yield from walk(child)
    elif isinstance(value, list):
        for child in value:
            yield from walk(child)


def check_refs(schemas):
    ids = {schema["$id"]: schema for schema in schemas.values()}
    require(len(ids) == len(schemas), "duplicate schema $id")
    for schema in schemas.values():
        require(schema["$schema"] == "https://json-schema.org/draft/2020-12/schema",
                "unexpected schema dialect")
        for node in walk(schema):
            if not isinstance(node, dict) or "$ref" not in node:
                continue
            uri, fragment = urldefrag(node["$ref"])
            if not uri:
                target = schema
            elif uri in schemas:
                target = schemas[uri]
            else:
                require(uri in ids, f"nonlocal/unresolved schema reference: {uri}")
                target = ids[uri]
            if fragment:
                require(fragment.startswith("/"), "only JSON Pointer fragments are allowed")
                for key in fragment[1:].split("/"):
                    key = key.replace("~1", "/").replace("~0", "~")
                    require(isinstance(target, dict) and key in target,
                            f"unresolved schema fragment: {node['$ref']}")
                    target = target[key]


def unique_names(items, key, label):
    values = [item[key] for item in items]
    require(len(values) == len(set(values)), f"duplicate {label}")


def inventory(doc, aggregate):
    import re
    paths = []
    for item in doc["files"]:
        path = item["relative_path"]
        require(re.fullmatch(r"[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*(?:/[A-Za-z0-9_-]+(?:\.[A-Za-z0-9_-]+)*)*", path)
                is not None and len(path) <= 260, "unsafe relative inventory path")
        for part in path.split("/"):
            base = part.split(".")[0].upper()
            require(base not in {"CON", "PRN", "AUX", "NUL", "CLOCK$"}
                    and not re.fullmatch(r"(?:COM|LPT)[0-9]", base), "reserved platform path")
        require(item["size_bytes"] > 0 and item["sha256"] != ZERO,
                "invalid file size or sentinel file digest")
        paths.append(path.casefold())
    require(len(paths) == len(set(paths)), "case-insensitive inventory collision")
    require(sum(item["size_bytes"] for item in doc["files"]) <= aggregate,
            "inventory aggregate budget exceeded")
    return set(paths)


def port_sections(doc):
    sections = doc.get("sections", [])
    previous_end, icc_total, sample_total = 0, 0, 0
    require(len(sections) <= 64, "too many sections")
    for section in sections:
        require(section["offset"] == (previous_end + 3) // 4 * 4,
                "noncanonical section gap/overlap/order")
        require(section["length"] > 0, "empty section")
        previous_end = section["offset"] + section["length"]
        require(previous_end <= 268435456, "payload area budget exceeded")
        if section["kind"] == "icc":
            require(section["length"] <= 4194304, "ICC section budget exceeded")
            icc_total += section["length"]
        else:
            require(section["kind"] == "f32" and section["length"] % 4 == 0
                    and section["offset"] % 4 == 0, "invalid sample section kind/alignment")
            sample_total += section["length"] // 4
    require(icc_total <= 16777216 and sample_total <= 67108864, "aggregate section budget exceeded")
    seen = []
    ports = doc.get("inputs", doc.get("outputs", {}))
    for port in ports.values():
        require(-2147483648 <= port["x"] + port["width"] <= 2147483647
                and -2147483648 <= port["y"] + port["height"] <= 2147483647,
                "rectangle endpoint exceeds i32")
        index = port["data_section_index"]
        require(0 <= index < len(sections), "data section index out of range")
        expected = port["width"] * port["height"] * port["channels"] * 4
        require(expected <= 268435456 and sections[index]["kind"] == "f32"
                and sections[index]["length"] == expected, "sample product/section mismatch")
        seen.append(index)
        if "icc_section_index" in port:
            index = port["icc_section_index"]
            require(0 <= index < len(sections) and sections[index]["kind"] == "icc",
                    "ICC section index/kind mismatch")
            seen.append(index)
    require(sorted(seen) == list(range(len(sections))), "unreferenced or aliased section")
    if doc["type"] != "run" or doc.get("status", "ok") != "ok":
        require(not sections, "hello/control/failure must not carry sections")
    if "negotiation" in doc and "wire_versions" in doc["negotiation"]:
        offer = doc["negotiation"]
        require(2 in offer["wire_versions"], "v2 bootstrap offer must include v2")
        require(set(offer["required_features"]) <= set(offer["supported_features"]),
                "required features were not offered")
    if doc.get("status") == "ready" and "readiness" in doc:
        allowed = {"probe": {"declared", "files_present", "files_verified"},
                   "prepare": {"files_verified", "session_created"}}
        require(doc["readiness"] in allowed[doc["action"]], "illegal readiness/action combination")
    codes = {"needs_model": "resource_missing", "needs_permission": "permission_required",
             "unavailable": "device_unavailable", "cancelled": "cancelled", "timeout": "deadline_exceeded"}
    if doc.get("status") in codes:
        require(doc["error"]["code"] == codes[doc["status"]], "status/error-code mismatch")
    if "resources" in doc:
        keys = [(entry["identity"]["id"], entry["identity"]["version"], entry["operation"],
                 entry["operation_version"]) for entry in doc["resources"]]
        require(len(keys) == len(set(keys)), "duplicate resource inventory record")
    if "execution" in doc:
        execution = doc["execution"]
        policy = execution["requested_policy"]
        require(execution["backend"] in policy["allowed_backends"], "backend violates allowlist")
        require(not execution["graph_fallback"] or policy["allow_graph_fallback"], "forbidden graph fallback")
        unique_names(execution["devices"], "id", "execution device ID")
        if policy["mode"] == "exact":
            require(all(device["id"] == policy["target_device"] for device in execution["devices"]),
                    "graph device violates exact policy")
        if policy["mode"] == "require_kind":
            require(all(device["kind"] == policy["target_kind"] for device in execution["devices"]),
                    "graph device violates required kind")
        timing = execution["timing"]
        require(timing["total_ms"] + 0.001 >= sum(timing[key] for key in
                ("cpu_preprocess_ms", "graph_execution_ms", "cpu_postprocess_ms")), "inconsistent serial timing")
        if execution["evidence"] == "synthetic_fixture":
            require(execution["runtime_id"].startswith("example.synthetic")
                    and execution["backend"].startswith("example.synthetic")
                    and all(device["id"].startswith("example.synthetic") for device in execution["devices"])
                    and execution["resource"]["id"].startswith("example.synthetic."),
                    "synthetic report uses non-example identity")
        else:
            require(execution["resource"]["manifest_digest"] != ZERO
                    and execution["configuration_digest"] != ZERO, "sentinel digest in non-synthetic report")


def document_semantics(schema_name, doc):
    tree_budget(doc)
    if schema_name.startswith("wire-"):
        port_sections(doc)
    elif schema_name == "manifest.schema.json":
        paths = inventory(doc, 536870912)
        if doc["deployment"] == "runnable":
            require(doc["entrypoint"].casefold() in paths, "entrypoint missing from inventory")
        unique_names(doc["operations"], "id", "operation ID")
        unique_names(doc["devices"], "id", "device ID")
        ids = {operation["id"] for operation in doc["operations"]}
        for device in doc["devices"]:
            require(set(device["operations"]) <= ids, "device references unknown operation")
        for operation in doc["operations"]:
            for field in ("input_ports", "output_ports"):
                unique_names(operation[field], "name", "port name")
            for node in walk(operation["params_schema"]["schema"]):
                if isinstance(node, dict):
                    require(not ({"$id", "$dynamicRef", "$recursiveRef"} & set(node)),
                            "parameter schema must not redefine URI scope or dynamic references")
                    if "$ref" in node:
                        require(node["$ref"].startswith("#"), "remote parameter-schema reference forbidden")
    elif schema_name == "model-resource.schema.json":
        paths = inventory(doc, 34359738368)
        if doc["deployment"] == "runnable":
            require(doc["manifest_digest"] != ZERO, "sentinel manifest digest")
            require(doc["license"]["text_relative_path"].casefold() in paths, "license file missing from inventory")
            require(any(item["role"] in {"model", "weights"} for item in doc["files"]), "missing model file")
        for field in ("inputs", "outputs"):
            unique_names(doc["bindings"][field], "port", "binding port")
            unique_names(doc["bindings"][field], "name", "model binding name")
            for binding in doc["bindings"][field]:
                require(math.prod(binding["shape"]) <= 67108864, "binding shape product exceeds budget")
                if binding["layout"] in {"nchw", "nhwc"}:
                    require(len(binding["shape"]) == 4, "rank/layout mismatch")


def pair_check(request, response):
    require(request["request_id"] == response["request_id"] and request["type"] == response["type"],
            "response correlation mismatch")
    if request["type"] == "hello" and response["status"] == "negotiated":
        offer, selected = request["negotiation"], response["negotiation"]
        require(selected["selected_wire_version"] in offer["wire_versions"], "unoffered wire version")
        enabled = set(selected["enabled_features"])
        require(enabled <= set(offer["supported_features"])
                and set(offer["required_features"]) <= enabled and BASE_FEATURES <= enabled,
                "invalid negotiated feature set")
    if request["type"] == "control":
        require(request["action"] == response["action"], "response action mismatch")
    if request["type"] == "control" and "resources" in response:
        for entry in response["resources"]:
            require(entry["operation"] == request["operation"]
                    and entry["operation_version"] == request["operation_version"], "resource-list operation scope mismatch")
            if "resource" in request:
                require(entry["identity"] == request["resource"], "resource-list identity scope mismatch")
    if response.get("status") == "ok":
        require(response["execution"]["requested_policy"] == request["device_policy"], "policy echo mismatch")
        require(response["execution"]["resource"] == request["resource"], "resource identity mismatch")
        source = request["inputs"]["source"]
        for port in response["outputs"].values():
            require(all(port[key] == source[key] for key in ("x", "y", "width", "height")), "same_geometry mismatch")


def operation_check(request, response, manifest):
    operation = next(op for op in manifest["operations"] if op["id"] == request["operation"])
    require(operation["semantic_version"] == request["operation_version"], "operation-version mismatch")
    for field, declarations in (("inputs", "input_ports"), ("outputs", "output_ports")):
        ports = request[field] if field == "inputs" else response[field]
        expected = {port["name"]: port for port in operation[declarations]}
        require(set(ports) <= set(expected) and all(not port["required"] or port["name"] in ports
                for port in expected.values()), "undeclared or missing required port")
        require(all(port["kind"] == expected[name]["kind"] for name, port in ports.items()), "port kind mismatch")
    source = request["inputs"]["source"]
    require(source["color_mode"] in operation["accepted_color_modes"]
            and source["source_depth"] in operation["accepted_source_depths"]
            and ("straight" if source["alpha"] else "none") in operation["accepted_alpha"], "unsupported source provenance")
    require(source["width"] * source["height"] <= operation["max_pixels"], "operation pixel budget exceeded")
    for port in request["inputs"].values():
        require(all(port[key] == source[key] for key in ("x", "y", "width", "height")), "input geometry mismatch")


def expect_rejected(label, action):
    try:
        action()
    except ValueError:
        return
    raise ValueError(f"negative regression was accepted: {label}")


def negative_checks(fixtures, validate):
    cases = []

    def case(file, label, mutate):
        value = copy.deepcopy(fixtures[file])
        mutate(value)
        cases.append((file, label, value))

    case("run-subject-request.json", "missing run resource", lambda d: d.pop("resource"))
    case("run-subject-request.json", "run/control mixing", lambda d: d.update(action="probe"))
    case("control-probe-request.json", "control with inputs", lambda d: d.update(inputs={}))
    case("control-prepare-request.json", "exact without target", lambda d: d["device_policy"].pop("target_device"))
    case("control-prepare-request.json", "exact with graph fallback", lambda d: d["device_policy"].update(allow_graph_fallback=True))
    case("control-probe-request.json", "kind without target", lambda d: d["device_policy"].update(mode="require_kind"))
    case("control-probe-request.json", "auto with target", lambda d: d["device_policy"].update(target_kind="npu"))
    case("control-probe-request.json", "evict without scope", lambda d: d.update(action="evict"))
    case("run-subject-request.json", "missing sample reference", lambda d: d["inputs"]["source"].pop("data_section_index"))
    case("run-subject-request.json", "bad sample product", lambda d: d["sections"][0].update(length=68))
    case("run-subject-request.json", "section overlap", lambda d: d["sections"][1].update(offset=0))
    case("run-subject-request.json", "payload bound", lambda d: d["sections"][0].update(length=268435460))
    case("run-subject-request.json", "rectangle overflow", lambda d: d["inputs"]["source"].update(x=2147483647))
    case("run-subject-request.json", "section alias", lambda d: d["inputs"]["selection"].update(data_section_index=0))
    case("run-subject-request.json", "image channel mismatch", lambda d: d["inputs"]["source"].update(channels=4))
    case("run-subject-request.json", "mask color metadata", lambda d: d["inputs"]["selection"].update(color_encoding="srgb"))
    case("run-subject-request.json", "alpha without policy", lambda d: d["inputs"]["source"].update(alpha=True, channels=4))
    case("ok-mask-response.json", "ok missing execution", lambda d: d.pop("execution"))
    case("ok-mask-response.json", "ok contains error", lambda d: d.update(error={"code": "internal_error", "retryable": False}))
    case("ok-mask-response.json", "error contains output", lambda d: d.update(status="error", error={"code": "internal_error", "retryable": False}))
    case("ok-mask-response.json", "undeclared backend", lambda d: d["execution"].update(backend="forbidden"))
    case("status-needs-model-response.json", "error missing reason", lambda d: d.pop("error"))
    case("status-ready-response.json", "probe falsely claims session", lambda d: d.update(readiness="session_created"))
    case("hello-request.json", "required feature not offered", lambda d: d["negotiation"].update(required_features=["unknown.required"]))
    case("model-resource-example.json", "unbounded shape", lambda d: d["bindings"]["inputs"][0].update(shape=[-1]))
    case("model-resource-example.json", "empty constraints", lambda d: d.update(runtime_constraints=[]))
    case("model-resource-example.json", "placeholder promoted", lambda d: d.update(deployment="runnable"))
    case("package-manifest-example.json", "unknown nested device field", lambda d: d["devices"][0].update(unknown=True))
    case("package-manifest-example.json", "missing primary output", lambda d: d["operations"][0]["output_ports"][0].update(primary=False))
    case("package-manifest-example.json", "duplicate port name", lambda d: d["operations"][0]["input_ports"][1].update(name="source"))
    for file, label, value in cases:
        expect_rejected(label, lambda f=file, v=value: validate(EXPECTED_FIXTURES[f], v))
    request = fixtures["run-subject-request.json"]
    wrong = copy.deepcopy(fixtures["ok-mask-response.json"])
    wrong["request_id"] = "wrong"
    expect_rejected("pair correlation", lambda: pair_check(request, wrong))
    wrong = copy.deepcopy(fixtures["ok-mask-response.json"])
    wrong["execution"]["resource"]["version"] = "9.9.9"
    expect_rejected("pair resource binding", lambda: pair_check(request, wrong))
    expect_rejected("duplicate keys", lambda: json.loads('{"a":1,"a":2}', object_pairs_hook=no_duplicates))
    expect_rejected("nonfinite JSON", lambda: json.loads('{"a":NaN}', parse_constant=reject_constant))
    expect_rejected("nonfinite numeric overflow", lambda: tree_budget(json.loads('{"a":1e999}')))
    return len(cases) + 5


def main():
    require(os.environ.get("GITHUB_ACTIONS") == "true", "CI ONLY: local validation/test execution is prohibited")
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--static", action="store_true")
    mode.add_argument("--schema", action="store_true")
    args = parser.parse_args()
    require({path.name for path in (ROOT / "schemas").glob("*.json")} == SCHEMA_NAMES, "unexpected/missing schema file")
    schemas = {name: strict_load(ROOT / "schemas" / name) for name in sorted(SCHEMA_NAMES)}
    check_refs(schemas)
    index = strict_load(ROOT / "fixtures" / "index.json")
    mapping = {entry["file"]: entry["schema"] for entry in index["fixtures"]}
    require(mapping == EXPECTED_FIXTURES and len(index["fixtures"]) == len(mapping), "fixture-index mismatch")
    require({path.name for path in (ROOT / "fixtures").glob("*.json")} == set(mapping) | {"index.json"}, "unindexed/missing fixture")
    fixtures = {name: strict_load(ROOT / "fixtures" / name) for name in sorted(mapping)}
    validators = {}
    if args.schema:
        for distribution, version in PINS.items():
            require(importlib.metadata.version(distribution) == version, f"CI dependency pin mismatch: {distribution}")
        from jsonschema import Draft202012Validator, FormatChecker
        from referencing import Registry, Resource
        formats = FormatChecker()
        require("uri" in formats.checkers, "URI format checker is unavailable; do not silently skip it")
        registry = Registry().with_resources((schema["$id"], Resource.from_contents(schema)) for schema in schemas.values())
        for name, schema in schemas.items():
            Draft202012Validator.check_schema(schema)
            validators[name] = Draft202012Validator(schema, registry=registry, format_checker=formats)

    def validate(name, value):
        if args.schema:
            errors = list(validators[name].iter_errors(value))
            require(not errors, f"{name}: {errors[0].message}" if errors else "")
        document_semantics(name, value)

    for name, value in fixtures.items():
        validate(mapping[name], value)
    for req, res in (("hello-request.json", "hello-response.json"),
                     ("control-probe-request.json", "status-ready-response.json"),
                     ("control-prepare-request.json", "status-needs-model-response.json"),
                     ("run-subject-request.json", "ok-mask-response.json")):
        pair_check(fixtures[req], fixtures[res])
    manifest = fixtures["package-manifest-example.json"]
    operation_check(fixtures["run-subject-request.json"], fixtures["ok-mask-response.json"], manifest)
    regressions = 0
    if args.schema:
        for operation in manifest["operations"]:
            embedded = operation["params_schema"]["schema"]
            Draft202012Validator.check_schema(embedded)
            errors = list(Draft202012Validator(embedded, registry=registry).iter_errors(fixtures["run-subject-request.json"]["params"]))
            require(not errors, "operation params do not match embedded schema")
        regressions = negative_checks(fixtures, validate)
    print(f"PASS: {len(schemas)} draft schemas / {len(fixtures)} synthetic JSON fixtures; "
          f"mode={'schema+static' if args.schema else 'static-only (NO JSON Schema validation)'}; "
          f"negative cases={regressions}. NOT binary/runtime/NPU conformance.")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, KeyError, TypeError, OSError, UnicodeError, RecursionError,
            importlib.metadata.PackageNotFoundError, ImportError, StopIteration) as exc:
        print(f"FAIL: {exc}", file=sys.stderr)
        sys.exit(1)
