"""Narrow QNN artifact-configuration gate; not a network or content sandbox.

Run only on GitHub Actions after approval. Requires PyYAML==6.0.2.
The workflow and this policy require independent protected review together.
Historical unauthorized runtime artifact 12413508676 under 17a8a7d3 is acknowledged.
This policy validates the reviewed workflow configuration and named evidence
paths, and gates its two designated evidence uploads. It does not attest to
historical uploads or arbitrary script/network activity.
"""
import argparse
import stat
from pathlib import Path

import yaml
from yaml.tokens import AliasToken, AnchorToken, TagToken

JOBS = {
    "pure-tests": ("windows-latest", "20", "photocraft-qnn-pure-evidence", [
        "artifacts/qnn-tests/dotnet-build.log",
        "artifacts/qnn-tests/core-results.json",
        "artifacts/qnn-tests/core-tests.log",
        "artifacts/qnn-tests/packaging-results.json",
        "artifacts/qnn-tests/packaging-tests.log",
    ]),
    "publish-inventory-smoke": ("windows-11-arm", "35", "photocraft-qnn-arm64-evidence-only", [
        "artifacts/qnn-evidence/validation-report.json",
        "artifacts/qnn-evidence/nuget-packages.json",
        "artifacts/qnn-evidence/native-dsp-inventory.json",
        "artifacts/qnn-evidence/legal-inventory.json",
        "artifacts/qnn-evidence/publish-inventory.json",
        "artifacts/qnn-evidence/adapter.json",
        "artifacts/qnn-evidence/process-smoke.json",
        "artifacts/qnn-evidence/publish.log",
        "artifacts/qnn-evidence/process-smoke.log",
    ]),
}
UPLOAD_IF = "${{ always() && steps.evidence_gate.outcome == 'success' }}"
SCRIPT = "./examples/onnx_qnn_worker/Tests/WorkflowGate.py"
PIP_RUN = (
    "python -m pip install --disable-pip-version-check PyYAML==6.0.2\n"
    "if ($LASTEXITCODE -ne 0) { throw 'Workflow policy dependency installation failed.' }"
)
GATE_FAILURE = "if ($LASTEXITCODE -ne 0) { throw 'QNN evidence gate failed; no artifact upload is allowed.' }"


class GateError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise GateError(message)


class UniqueLoader(yaml.BaseLoader):
    def construct_mapping(self, node, deep=False):
        result = {}
        for key_node, value_node in node.value:
            key = self.construct_object(key_node, deep=deep)
            require(isinstance(key, str) and key != "<<", "Non-scalar or merge key")
            require(key not in result, f"Duplicate key: {key}")
            result[key] = self.construct_object(value_node, deep=deep)
        return result


def load_workflow(text):
    for token in yaml.scan(text):
        require(not isinstance(token, (AliasToken, AnchorToken, TagToken)),
                "Aliases, anchors and explicit tags are not allowed")
    value = yaml.load(text, Loader=UniqueLoader)
    require(isinstance(value, dict), "Workflow must be a mapping")
    return value


POLICY_TEST_RUN = (
    "python ./examples/onnx_qnn_worker/Tests/WorkflowGate.Tests.py\n"
    "if ($LASTEXITCODE -ne 0) { throw 'Workflow gate regression assertions failed.' }"
)


def gate_run(job):
    dependency_check = (
        "if ('${{ steps.policy_python.outcome }}' -ne 'success' -or "
        "'${{ steps.policy_dependency.outcome }}' -ne 'success') "
        "{ throw 'Policy prerequisites failed; no evidence upload is allowed.' }"
    )
    test_check = (
        "\nif ('${{ steps.policy_tests.outcome }}' -ne 'success') "
        "{ throw 'Policy regression assertions did not pass.' }"
    ) if job == "pure-tests" else ""
    return f"{dependency_check}{test_check}\npython {SCRIPT} --job {job}\n{GATE_FAILURE}"


def validate_workflow(workflow):
    require(set(workflow) == {"name", "on", "permissions", "concurrency", "defaults", "jobs"},
            "Unexpected workflow keys")
    require(workflow["permissions"] == {"contents": "read"}, "Permissions changed")
    require(workflow["defaults"] == {"run": {"shell": "pwsh"}}, "Shell/defaults changed")
    require(workflow["on"] == {
        "push": {"branches": ["main", "compat/**"], "paths": [
            "examples/onnx_qnn_worker/**", ".github/workflows/companion-qnn.yml"]},
        "pull_request": {"paths": [
            "examples/onnx_qnn_worker/**", ".github/workflows/companion-qnn.yml"]},
        "workflow_dispatch": "",
    }, "Triggers changed")
    require(workflow["concurrency"] == {
        "group": "qnn-draft-${{ github.workflow }}-${{ github.ref }}",
        "cancel-in-progress": "true",
    }, "Concurrency changed")
    jobs = workflow["jobs"]
    require(isinstance(jobs, dict) and set(jobs) == set(JOBS), "Unexpected jobs/reusable workflow")
    for job_id, (runner, timeout, artifact, paths) in JOBS.items():
        job = jobs[job_id]
        keys = {"name", "runs-on", "timeout-minutes", "steps"}
        if job_id == "publish-inventory-smoke":
            keys.add("needs")
        require(isinstance(job, dict) and set(job) == keys, f"Unexpected job keys: {job_id}")
        require(job["runs-on"] == runner and job["timeout-minutes"] == timeout,
                f"Runner/timeout changed: {job_id}")
        if "needs" in keys:
            require(job["needs"] == "pure-tests", "T4 dependency changed")
        steps = job["steps"]
        require(isinstance(steps, list) and len(steps) >= 6, "Invalid steps")
        require(steps[:4] == [
            {"uses": "actions/checkout@v4"},
            {"uses": "actions/setup-dotnet@v4", "with": {"dotnet-version": "8.0.x"}},
            {"uses": "actions/setup-python@v5", "id": "policy_python", "with": {"python-version": "3.12"}},
            {"name": "Install pinned workflow policy parser", "id": "policy_dependency", "run": PIP_RUN + "\n"},
        ], f"Setup/policy dependency steps changed: {job_id}")
        for step in steps:
            require(isinstance(step, dict), "Step must be a mapping")
            require(set(step) <= {"name", "id", "if", "run", "uses", "with"},
                    "Unknown step controls (including continue-on-error) are forbidden")
            require(("uses" in step) != ("run" in step), "Step must have exactly one of uses/run")
            if "uses" in step:
                require(step["uses"] in {
                    "actions/checkout@v4", "actions/setup-dotnet@v4",
                    "actions/setup-python@v5", "actions/upload-artifact@v4",
                }, f"Non-allowlisted action (including cache/release): {step['uses']}")
            else:
                require(isinstance(step["run"], str) and "with" not in step, "Invalid run step")
                require(step.get("if") in (None, "always()"), "Unexpected run condition")
        require([s["uses"] for s in steps if "uses" in s] == [
            "actions/checkout@v4", "actions/setup-dotnet@v4",
            "actions/setup-python@v5", "actions/upload-artifact@v4",
        ], "Additional/reordered actions or uploads")
        gate = {"name": "Validate workflow policy and exact evidence paths", "id": "evidence_gate",
                "if": "always()", "run": gate_run(job_id) + "\n"}
        upload = {"name": "Upload exact evidence only after this job gate succeeds",
                  "if": UPLOAD_IF, "uses": "actions/upload-artifact@v4", "with": {
                      "name": artifact, "path": "\n".join(paths) + "\n",
                      "if-no-files-found": "error", "retention-days": "14",
                  }}
        require(steps[-2:] == [gate, upload], "Final gate/upload must match the exact contract")
        expected_ids = ["policy_python", "policy_dependency"]
        if job_id == "pure-tests":
            expected_ids.append("policy_tests")
            require(steps[4] == {"name": "Assert workflow gate regression cases", "id": "policy_tests",
                                 "run": POLICY_TEST_RUN + "\n"}, "Policy assertion step changed")
        expected_ids.append("evidence_gate")
        require([s.get("id") for s in steps if "id" in s] == expected_ids,
                "IDs must be unique, ordered and local to this job")


def validate_evidence(root, job):
    require(not (root / "artifacts/photocraft-onnx-qnn-arm64.zip").exists(),
            "Unreviewed runtime ZIP must never be produced/uploaded by this draft")
    # lstat each component: do not follow an evidence symlink/junction into the publish tree.
    # This is a point-in-time path check, not race resistance, content sanitization or hardlink proof.
    reparse = getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0x400)
    for relative in JOBS[job][3]:
        cursor = root
        parts = Path(relative).parts
        for index, part in enumerate(parts):
            cursor = cursor / part
            info = cursor.lstat()  # Missing any single required file fails closed.
            require(not stat.S_ISLNK(info.st_mode)
                    and not (getattr(info, "st_file_attributes", 0) & reparse),
                    f"Linked/reparse evidence path: {relative}")
            require(stat.S_ISREG(info.st_mode) if index == len(parts) - 1
                    else stat.S_ISDIR(info.st_mode), f"Invalid evidence file/directory: {relative}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--job", choices=JOBS, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[3]
    workflow = root / ".github/workflows/companion-qnn.yml"
    validate_workflow(load_workflow(workflow.read_text(encoding="utf-8")))
    validate_evidence(root, args.job)
    print(f"Explicit artifact configuration/path gate passed for {args.job}; legal review remains BLOCKED.")


if __name__ == "__main__":
    main()
