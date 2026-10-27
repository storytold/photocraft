"""Proposed GH-only gate regression assertions; no ORT, model or NPU execution."""
import copy
import stat
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import yaml

import WorkflowGate as gate

ROOT = Path(__file__).resolve().parents[3]
WORKFLOW = ROOT / ".github/workflows/companion-qnn.yml"


class WorkflowPolicyTests(unittest.TestCase):
    def setUp(self):
        self.workflow = gate.load_workflow(WORKFLOW.read_text(encoding="utf-8"))

    def reject(self, mutate):
        candidate = copy.deepcopy(self.workflow)
        mutate(candidate)
        with self.assertRaises(gate.GateError):
            gate.validate_workflow(candidate)

    def test_review_candidate_is_accepted(self):
        gate.validate_workflow(self.workflow)

    def test_ambiguous_yaml_is_rejected(self):
        for text in ("jobs: {}\njobs: {}\n", "x: &x {}\ny: *x\n", "<<: {}\n", "x: !!str value\n"):
            with self.subTest(text=text), self.assertRaises((gate.GateError, yaml.YAMLError)):
                gate.load_workflow(text)

    def test_cache_release_and_other_action_nodes_are_rejected(self):
        actions = ("actions/cache@v4", "actions/cache/restore@v4", "actions/cache/save@v4",
                   "actions/create-release@v1", "softprops/action-gh-release@v2",
                   "actions/upload-artifact@v3", "./local-action", "docker://example/image")
        for job in gate.JOBS:
            for action in actions:
                with self.subTest(job=job, action=action):
                    self.reject(lambda w, j=job, a=action:
                                w["jobs"][j]["steps"].insert(-2, {"uses": a}))

    def test_reusable_workflow_and_extra_job_are_rejected(self):
        self.reject(lambda w: w["jobs"].update({"release": {"uses": "owner/repo/.github/workflows/release.yml@main"}}))
        self.reject(lambda w: w["jobs"]["pure-tests"].update({"uses": "./.github/workflows/other.yml"}))

    def test_upload_and_gate_controls_are_exact_in_each_job(self):
        for job in gate.JOBS:
            mutations = {
                "always upload": lambda s: s[-1].update({"if": "always()"}),
                "conclusion not outcome": lambda s: s[-1].update({"if": gate.UPLOAD_IF.replace("outcome", "conclusion")}),
                "foreign gate": lambda s: s[-1].update({"if": "${{ always() && needs.pure-tests.result == 'success' }}"}),
                "no condition": lambda s: s[-1].pop("if"),
                "bypass gate": lambda s: s[-2].update({"run": "Write-Host 'pass'\n"}),
                "missing gate": lambda s: s.pop(-2),
                "gate skipped": lambda s: s[-2].update({"if": "success()"}),
                "continue on error": lambda s: s[-2].update({"continue-on-error": "true"}),
                "wrong gate id": lambda s: s[-2].update({"id": "different_gate"}),
                "duplicate gate id": lambda s: s.insert(-2, copy.deepcopy(s[-2])),
                "upload before gate": lambda s: s.insert(-2, s.pop(-1)),
                "duplicate upload": lambda s: s.append(copy.deepcopy(s[-1])),
                "prerequisite failure ignored": lambda s: s[-2].update({"run": "\n".join(s[-2]["run"].splitlines()[1:]) + "\n"}),
                "cache setup option": lambda s: s[1]["with"].update({"cache": "true"}),
            }
            for description, mutate in mutations.items():
                with self.subTest(job=job, description=description):
                    self.reject(lambda w, j=job, m=mutate: m(w["jobs"][j]["steps"]))

    def test_upload_with_is_exact_in_each_job(self):
        for job in gate.JOBS:
            mutations = {
                "runtime name": {"name": "photocraft-onnx-qnn-arm64"},
                "runtime directory": {"path": "artifacts/publish-qnn-arm64/**\n"},
                "wildcard": {"path": "artifacts/qnn-tests/*.json\n"},
                "extra path": {"path": "\n".join(gate.JOBS[job][3]) + "\nartifacts/other.log\n"},
                "expression path": {"path": "${{ github.workspace }}\n"},
                "exclude pattern": {"path": "artifacts/**\n!artifacts/*.dll\n"},
                "ignore missing": {"if-no-files-found": "ignore"},
                "extra option": {"include-hidden-files": "true"},
                "retention changed": {"retention-days": "90"},
            }
            for description, change in mutations.items():
                with self.subTest(job=job, description=description):
                    self.reject(lambda w, j=job, c=change: w["jobs"][j]["steps"][-1]["with"].update(c))

    def test_script_names_and_comments_do_not_self_match(self):
        # The real workflow invokes ProcessSmoke.ps1 and contains policy script paths.
        # Disallowed action strings within comments/run text are not action nodes.
        self.workflow["jobs"]["pure-tests"]["steps"][5]["run"] += "\n# actions/cache@v4 gh release\n"
        gate.validate_workflow(self.workflow)

    def test_permissions_shell_and_unknown_controls_are_rejected(self):
        self.reject(lambda w: w.update({"env": {"EXTRA_UPLOAD": "true"}}))
        self.reject(lambda w: w["permissions"].update({"contents": "write"}))
        self.reject(lambda w: w["defaults"]["run"].update({"shell": "bash"}))
        self.reject(lambda w: w["jobs"]["pure-tests"].update({"continue-on-error": "true"}))


class EvidencePathTests(unittest.TestCase):
    def make_files(self, root, job):
        for relative in gate.JOBS[job][3]:
            path = root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("synthetic policy test evidence only\n", encoding="utf-8")

    def test_complete_named_evidence_passes(self):
        for job in gate.JOBS:
            with self.subTest(job=job), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                self.make_files(root, job)
                gate.validate_evidence(root, job)

    def test_each_missing_required_file_fails(self):
        for job in gate.JOBS:
            for missing in gate.JOBS[job][3]:
                with self.subTest(job=job, missing=missing), tempfile.TemporaryDirectory() as temp:
                    root = Path(temp)
                    self.make_files(root, job)
                    (root / missing).unlink()
                    with self.assertRaises(FileNotFoundError):
                        gate.validate_evidence(root, job)

    def test_runtime_zip_blocks_both_jobs(self):
        for job in gate.JOBS:
            with self.subTest(job=job), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                self.make_files(root, job)
                (root / "artifacts/photocraft-onnx-qnn-arm64.zip").write_bytes(b"synthetic not a ZIP")
                with self.assertRaises(gate.GateError):
                    gate.validate_evidence(root, job)

    def test_directory_as_evidence_is_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.make_files(root, "pure-tests")
            path = root / gate.JOBS["pure-tests"][3][0]
            path.unlink()
            path.mkdir()
            with self.assertRaises(gate.GateError):
                gate.validate_evidence(root, "pure-tests")

    def test_link_and_windows_reparse_attributes_are_rejected(self):
        # Attribute simulation does not require Windows symlink/developer privileges.
        for mode, attributes in ((stat.S_IFLNK, 0), (stat.S_IFDIR, 0x400)):
            with self.subTest(mode=mode, attributes=attributes), tempfile.TemporaryDirectory() as temp:
                fake = SimpleNamespace(st_mode=mode, st_file_attributes=attributes)
                with patch.object(Path, "lstat", return_value=fake), self.assertRaises(gate.GateError):
                    gate.validate_evidence(Path(temp), "pure-tests")


if __name__ == "__main__":
    unittest.main(verbosity=2)
