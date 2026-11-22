#!/usr/bin/env python3
"""Synthetic dispatcher regression fixtures; original local work, not live API/crypto evidence."""
import contextlib
import copy
import io
import json
import os
from pathlib import Path
import re
import tempfile
import textwrap
import unittest
from unittest.mock import patch
import urllib.error

ROOT = Path(__file__).resolve().parents[1]
DISPATCHER = ROOT / ".github/workflows/nightly-dispatch.yml"


def inline_scripts(path):
    """Read literal run blocks, not a substitute for YAML/actionlint validation."""
    return [textwrap.dedent(block) for block in re.findall(
        r"^        run: \|\n((?:          .*\n|\n)+)", path.read_text(), re.MULTILINE)]


class Response:
    status = 204

    def __enter__(self):
        return self

    def __exit__(self, *args):
        return False


class DispatchBoundary(unittest.TestCase):
    def setUp(self):
        # All generated metadata is synthetic, including the presence-only mock token.
        self.event = {"action": "completed", "repository": {"full_name": "storytold/photocraft"},
                      "workflow_run": {"id": 71, "run_attempt": 1, "status": "completed",
                                       "conclusion": "success", "event": "schedule",
                                       "path": ".github/workflows/nightly.yml", "head_branch": "main",
                                       "head_repository": {"full_name": "storytold/photocraft"}}}
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.event_path = Path(self.temp.name) / "event.json"
        self.env = {"GITHUB_REPOSITORY": "storytold/photocraft", "GITHUB_EVENT_NAME": "workflow_run",
                    "GITHUB_EVENT_PATH": str(self.event_path), "GH_TOKEN": "synthetic-not-a-token"}
        scripts = inline_scripts(DISPATCHER)
        if len(scripts) != 1:
            raise RuntimeError("dispatcher must have one locally exercisable literal script")
        self.program = compile(scripts[0], str(DISPATCHER), "exec")

    def execute(self, event=None, env=None, failure=None, response=None):
        self.event_path.write_text(json.dumps(self.event if event is None else event))
        scope = {}
        with patch.dict(os.environ, self.env if env is None else env, clear=True), \
                patch("urllib.request.build_opener") as opener, \
                contextlib.redirect_stdout(io.StringIO()):
            opener.return_value.open.return_value = response or Response()
            opener.return_value.open.side_effect = failure
            try:
                exec(self.program, scope)
            except Exception as error:
                return opener, scope, error
        return opener, scope, None

    def assert_refused(self, event=None, env=None):
        opener, _, error = self.execute(event=event, env=env)
        self.assertIsNotNone(error, "invalid transport must refuse")
        opener.assert_not_called()

    def test_only_fixed_release_destination_and_numeric_request_are_transported(self):
        # Caller-selected source/ref/digest fields have no effect on the transmitted request.
        self.event["workflow_run"].update({"ref": "main", "artifact_id": 999,
                                           "sha256": "untrusted", "tooling": "untrusted"})
        opener, _, error = self.execute()
        self.assertIsNone(error)
        opener.return_value.open.assert_called_once()
        request = opener.return_value.open.call_args.args[0]
        self.assertEqual(request.full_url,
                         "https://api.github.com/repos/storytold/photocraft/actions/workflows/nightly-sign.yml/dispatches")
        self.assertEqual(request.get_method(), "POST")
        self.assertEqual(json.loads(request.data), {"ref": "release", "inputs": {
            "producer_run_id": "71", "producer_run_attempt": "1"}})
        self.assertGreater(opener.return_value.open.call_args.kwargs["timeout"], 0)

    def test_wrong_top_level_context_refuses_before_network(self):
        for key, value in (("GITHUB_REPOSITORY", "other/photocraft"),
                           ("GITHUB_EVENT_NAME", "workflow_dispatch")):
            with self.subTest(key=key):
                self.assert_refused(env={**self.env, key: value})

    def test_foreign_or_noncompleted_events_refuse_before_network(self):
        for key, value in (("action", "requested"),
                           ("repository", {"full_name": "other/photocraft"})):
            with self.subTest(key=key):
                self.assert_refused(event={**self.event, key: value})

    def test_wrong_producer_identity_status_and_manual_retries_refuse(self):
        for key, value in (("head_repository", {"full_name": "other/photocraft"}),
                           ("head_branch", "release"), ("head_branch", "refs/tags/main"),
                           ("path", ".github/workflows/release.yml"), ("status", "in_progress"),
                           ("conclusion", "failure"), ("conclusion", "cancelled"),
                           ("event", "workflow_dispatch"), ("event", "pull_request"),
                           ("run_attempt", 2)):
            event = copy.deepcopy(self.event)
            event["workflow_run"][key] = value
            with self.subTest(key=key, value=value):
                self.assert_refused(event=event)

    def test_ids_are_not_coerced_from_strings_booleans_or_fractional_values(self):
        for key in ("id", "run_attempt"):
            for value in (None, False, True, 0, -1, 1.5, "1", "1;evil", [], {}):
                event = copy.deepcopy(self.event)
                event["workflow_run"][key] = value
                with self.subTest(key=key, value=value):
                    self.assert_refused(event=event)

    def test_missing_metadata_never_contacts_api(self):
        for key in self.event["workflow_run"]:
            event = copy.deepcopy(self.event)
            del event["workflow_run"][key]
            with self.subTest(key=key):
                self.assert_refused(event=event)

    def test_redirect_handler_refuses_instead_of_forwarding_authorization(self):
        _, scope, error = self.execute()
        self.assertIsNone(error)
        with self.assertRaisesRegex(RuntimeError, "redirects are forbidden"):
            scope["NoRedirect"]().redirect_request(None, None, 302, "found", {}, "https://untrusted.invalid")

    def test_failed_http_request_is_not_retried_or_reported_successful(self):
        for error in (TimeoutError("synthetic timeout"), urllib.error.URLError("synthetic failure")):
            with self.subTest(error=type(error).__name__):
                opener, _, caught = self.execute(failure=error)
                self.assertIs(caught, error)
                opener.return_value.open.assert_called_once()
        response = Response()
        response.status = 202
        opener, _, error = self.execute(response=response)
        self.assertIsNotNone(error)
        opener.return_value.open.assert_called_once()


class TrustedSourcePreparation(unittest.TestCase):
    def test_no_cache_actions_or_cache_setup_options_in_app_trusted_closure(self):
        # Required R3 static guard for app workflows only; activation must extend
        # this check to every reviewed immutable shared trusted callee.
        for name in ("nightly-sign.yml", "nightly-dispatch.yml"):
            source = (ROOT / ".github/workflows" / name).read_text()
            executable = "\n".join(line for line in source.splitlines()
                                   if not line.lstrip().startswith("#"))
            with self.subTest(workflow=name):
                self.assertNotRegex(executable, r"(?im)^\s*(?:-\s*)?uses:\s*[^\n]*cache")
                self.assertNotRegex(executable, r"(?im)^\s*[^\s:#]*cache[^\s:#]*:")
                self.assertNotRegex(executable, r"(?im)^\s*(?:environment|secrets):")
                self.assertNotIn("secrets.", executable)


if __name__ == "__main__":
    unittest.main(verbosity=2)
