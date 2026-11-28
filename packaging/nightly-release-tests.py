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
import base64
import hashlib
import unittest
from unittest.mock import patch
import urllib.error

ROOT = Path(__file__).resolve().parents[1]
DISPATCHER = ROOT / ".github/workflows/nightly-dispatch.yml"


def inline_scripts(path):
    """Read literal run blocks, not a substitute for YAML/actionlint validation."""
    return [textwrap.dedent(block) for block in re.findall(
        r"^        run: \|\n((?:          .*\n|\n)+)", path.read_text(), re.MULTILINE)]


def assert_nightly_serialization(source):
    """Fail closed unless this app's plain root concurrency block retains its contract.

    This is a deliberately narrow source guard, not a general YAML parser; actionlint
    still checks all other syntax. Expressions, aliases and alternate block shapes
    refuse rather than delegating queue-value validation to the old linter.
    """
    lines = source.splitlines()
    roots = [index for index, line in enumerate(lines) if line.startswith("concurrency:")]
    if len(roots) != 1 or lines[roots[0]] != "concurrency:":
        raise ValueError("one plain root concurrency block is required")
    values = {}
    for line in lines[roots[0] + 1:]:
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if not line[0].isspace():
            break
        row = re.fullmatch(r"  (group|cancel-in-progress|queue): ([^#]+?)(?: +#.*)?", line)
        if row is None or row[1] in values:
            raise ValueError("malformed or duplicate nightly concurrency entry")
        values[row[1]] = row[2].rstrip()
    if values != {"group": "photocraft-nightly-release", "cancel-in-progress": "false", "queue": "max"}:
        raise ValueError("nightly serialization must use its stable group, max queue and no cancellation")


def literal_off(condition):
    expression = " ".join(condition.split())
    if expression.startswith("${{") and expression.endswith("}}"):
        expression = expression[3:-2].strip()
    if expression == "false":
        return True
    if not expression.startswith("false && "):
        return False
    # A top-level OR could override the leading false; parentheses must balance.
    expression = re.sub(r"'(?:''|[^'])*'", "''", expression)
    depth = 0
    for token in re.findall(r"\(|\)|\|\|", expression):
        if token == "(":
            depth += 1
        elif token == ")":
            depth -= 1
            if depth < 0:
                return False
        elif depth == 0:
            return False
    return depth == 0


def job_conditions(jobs):
    """Each job-level `if:` as (block indicator or None, the whole expression YAML would read).

    A narrow source reader, not a YAML parser: every following line indented deeper than the
    job keys continues the value, for block scalars and plain multi-line scalars alike, so a
    continuation line can never hide from the guard checks.
    """
    lines = jobs.splitlines()
    found = []
    for index, line in enumerate(lines):
        head = re.fullmatch(r"    if:(?: (.*))?", line)
        if head is None:
            continue
        value = (head[1] or "").strip()
        continuation = []
        for follow in lines[index + 1:]:
            if follow.strip() and len(follow) - len(follow.lstrip(" ")) <= 4:
                break
            continuation.append(follow.strip())
        indicator = value if re.fullmatch(r"[>|][+-]?", value) else None
        parts = continuation if indicator else [value] + continuation
        found.append((indicator, " ".join(part for part in parts if part)))
    return found


NIGHTLY_PLATFORMS = ("windows-x86_64", "linux-x86_64", "macos-aarch64")
# Each platform's one trust field in policy v3, and the public stable identity it must name.
NIGHTLY_TRUST = {
    "windows-x86_64": ("publisher_subject",
                       "CN=Learning Machines Inc, O=Learning Machines Inc, L=Atlanta, S=Georgia, C=US"),
    "linux-x86_64": ("fingerprint", "2B76741B414AAD6CD33E631213A005C0B9D7C3ED"),
    "macos-aarch64": ("apple_team_id", "DJ6XS33FX8"),
}
LINUX_KEY = ROOT / ".github/nightly-linux-signing-key.asc"


def openpgp_primary_fingerprints(armored):
    """Return the v4 fingerprints of every primary public-key packet in one armored block.

    A narrow reader for the committed public key, not a general OpenPGP parser: anything
    other than one well-formed public-key block refuses.
    """
    lines = armored.strip().splitlines()
    if lines[:1] != ["-----BEGIN PGP PUBLIC KEY BLOCK-----"] or lines[-1:] != ["-----END PGP PUBLIC KEY BLOCK-----"]:
        raise ValueError("exactly one armored public key block is required")
    body = lines[1:-1]
    if "" not in body:
        raise ValueError("armor header separator missing")
    body = body[body.index("") + 1:]
    if not body or not body[-1].startswith("="):
        raise ValueError("armor checksum missing")
    data = base64.b64decode("".join(body[:-1]), validate=True)
    fingerprints, offset = [], 0
    while offset < len(data):
        header = data[offset]
        if not header & 0x80:
            raise ValueError("invalid packet header")
        if header & 0x40:
            tag, offset = header & 0x3F, offset + 1
            first = data[offset]
            if first < 192:
                length, offset = first, offset + 1
            elif first < 224:
                length, offset = ((first - 192) << 8) + data[offset + 1] + 192, offset + 2
            elif first == 255:
                length, offset = int.from_bytes(data[offset + 1:offset + 5], "big"), offset + 5
            else:
                raise ValueError("partial packet lengths are not a public key")
        else:
            tag, kind, offset = (header >> 2) & 0x0F, header & 0x03, offset + 1
            if kind == 3:
                raise ValueError("indeterminate packet length")
            size = 1 << kind
            length, offset = int.from_bytes(data[offset:offset + size], "big"), offset + size
        packet = data[offset:offset + length]
        if len(packet) != length:
            raise ValueError("truncated packet")
        if tag == 5 or tag == 7:
            raise ValueError("secret key material must never be committed")
        if tag == 6:
            if packet[:1] != b"\x04":
                raise ValueError("only v4 primary keys are expected")
            fingerprints.append(hashlib.sha1(b"\x99" + length.to_bytes(2, "big") + packet).hexdigest().upper())
        offset += length
    return fingerprints


def producer_platforms(source):
    """Read the one producer job's literal platform list; a narrow source guard, not YAML.

    One producer run builds every listed platform for one all-or-nothing release, so the
    list must be a compact JSON list of distinct known platforms that keeps Windows.
    """
    rows = re.findall(r"(?m)^      platforms: (.*)$", source)
    if len(rows) != 1:
        raise ValueError("exactly one producer platform list is required")
    literal = re.fullmatch(r"'(\[[^']*\])'", rows[0])
    if literal is None:
        raise ValueError("the platform list must be one single-quoted literal")
    platforms = json.loads(literal[1])
    if not isinstance(platforms, list) or json.dumps(platforms, separators=(",", ":")) != literal[1]:
        raise ValueError("the platform list must be compact JSON")
    if len(set(platforms)) != len(platforms) or not set(platforms) <= set(NIGHTLY_PLATFORMS):
        raise ValueError("platforms must be distinct known nightly targets")
    if "windows-x86_64" not in platforms:
        raise ValueError("the Windows row stays in the producer")
    return platforms


# The pinned nightly-apps.json PhotoCraft row has Windows, Linux and macOS targets.
# These are the corresponding names from the pinned nightly_signer_settings.py.
NIGHTLY_SIGNER_SECRETS = {
    "APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "APPLE_ID", "APPLE_PASSWORD",
    "APPLE_TEAM_ID", "KEYCHAIN_PASSWORD", "LINUX_SIGNING_KEY", "AZURE_CERT_PROFILE",
    "AZURE_CLIENT_ID", "AZURE_CLIENT_SECRET", "AZURE_SIGNING_ACCOUNT",
    "AZURE_SIGNING_ENDPOINT", "AZURE_TENANT_ID",
}


def assert_nightly_caller_secrets(source, signer):
    """Check the callers' plain block layout without adding a YAML dependency to CI.

    This is a narrow source guard, not a YAML parser. Noncanonical job/key syntax
    fails closed; the shared signer preflight separately parses YAML.
    """
    lines = [line for line in source.splitlines() if line.strip() and not line.lstrip().startswith("#")]
    roots = [line for line in lines if line == line.lstrip()]
    if (not roots or roots[-1] != "jobs:" or
            any(not re.fullmatch(r"(?:name|on|permissions|concurrency):(?: .*)?", line)
                for line in roots[:-1])):
        raise ValueError("expected one plain root jobs block")
    job = "release" if signer else "unsigned"
    start = lines.index("jobs:")
    if any(re.search(r"\bsecrets\b", line, re.IGNORECASE) for line in lines[:start]):
        raise ValueError("no secret references outside the signer mapping")
    if lines[start + 1:start + 2] != [f"  {job}:"]:
        raise ValueError(f"expected one plain {job} job")
    keys, mapping = set(), {}
    field = None
    for line in lines[start + 2:]:
        if not line.startswith("      "):
            match = re.fullmatch(r"    (if|permissions|uses|with|secrets):(?: .+)?", line)
            if match is None or match[1] in keys:
                raise ValueError("expected unique plain caller job keys and no environment binding")
            field = match[1]
            keys.add(field)
            if field == "secrets" and (not signer or line != "    secrets:"):
                raise ValueError("only the signer may declare a plain secrets mapping")
        elif field == "secrets":
            match = re.fullmatch(r"      ([A-Z][A-Z0-9_]*): \$\{\{ secrets\.([A-Z][A-Z0-9_]*) \}\}", line)
            if match is None or match[1] in mapping:
                raise ValueError("expected unique literal secret references")
            mapping[match[1]] = match[2]
            continue
        if field != "secrets" and re.search(r"\bsecrets\b", line, re.IGNORECASE):
            raise ValueError("no secret references outside the signer mapping")
    if signer and mapping != {name: name for name in NIGHTLY_SIGNER_SECRETS}:
        raise ValueError("the signer must forward exactly the target-specific same-name references")


class NightlyCallerSecrets(unittest.TestCase):
    SIGNER = ROOT / ".github/workflows/nightly-sign.yml"
    UNSIGNED = ROOT / ".github/workflows/nightly.yml"

    def test_signer_forwards_exactly_the_target_specific_secret_names(self):
        assert_nightly_caller_secrets(self.SIGNER.read_text(), signer=True)

    def test_unsigned_caller_has_no_secret_inputs(self):
        assert_nightly_caller_secrets(self.UNSIGNED.read_text(), signer=False)

    def test_signer_refuses_noncanonical_or_mismatched_forwarding(self):
        source = self.SIGNER.read_text()
        start = "    secrets:\n"
        key = "      LINUX_SIGNING_KEY: ${{ secrets.LINUX_SIGNING_KEY }}\n"
        self.assertIn(start, source)
        self.assertIn(key, source)
        for case, bad in {
            "missing": source.replace(key, ""),
            "extra": source.replace(key, key + "      UNUSED: ${{ secrets.UNUSED }}\n"),
            "different name": source.replace(key, key.replace("secrets.LINUX_SIGNING_KEY", "secrets.APPLE_ID")),
            "duplicate name": source.replace(key, key + key),
            "duplicate map": source.replace(start, start + start),
            "inherit": source.replace(start, "    secrets: inherit\n"),
            "YAML anchor": source.replace(start, "    secrets: &forwarded\n"),
            "YAML alias": source.replace(key, key.replace("${{ secrets.LINUX_SIGNING_KEY }}", "*forwarded")),
            "environment binding": source.replace(start, "    environment: release\n" + start),
        }.items():
            with self.subTest(case=case), self.assertRaises(ValueError):
                assert_nightly_caller_secrets(bad, signer=True)

    def test_unsigned_refuses_a_secret_map_or_reference(self):
        source = self.UNSIGNED.read_text()
        for extra in ("    secrets: inherit\n", "    secrets: {}\n", "    'secrets': {}\n",
                      "      unintended: ${{ secrets['UNUSED'] }}\n"):
            with self.subTest(extra=extra), self.assertRaises(ValueError):
                assert_nightly_caller_secrets(source + extra, signer=False)


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
        self.assertEqual(json.loads(request.data), {"ref": "refs/heads/nightly-signer", "inputs": {
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
    def test_sonar_classifies_only_the_exact_packaging_fixture_runners(self):
        source = (ROOT / "sonar-project.properties").read_text()
        self.assertEqual(re.findall(r"(?m)^sonar.sources=(.*)$", source), ["."])
        self.assertEqual(re.findall(r"(?m)^sonar.tests=(.*)$", source), ["packaging"])
        inclusions = re.findall(r"(?m)^sonar.test.inclusions=(.*)$", source)
        self.assertEqual(len(inclusions), 1)
        entries = inclusions[0].split(",")
        expected = {"packaging/nightly-tests.py", "packaging/nightly-archive-tests.py",
                    "packaging/macos/nightly_plist_tests.py", "packaging/macos/nightly_sign_tests.py",
                    "packaging/windows/nightly-package-tests.ps1",
                    "packaging/windows/nightly-signing-tests.ps1", "packaging/nightly-release-tests.py"}
        self.assertEqual(set(entries), expected, "classify exact fixtures, never production helpers or wildcard scope")
        self.assertEqual(len(entries), len(expected), "duplicate fixture classification")

    def test_policy_example_keeps_explicit_v3_and_no_actor_authority(self):
        policy = json.loads((ROOT / "docs/nightly-signing-policy.example.json").read_text())
        expected = {"schema", "repository", "repository_id", "producer_workflow_id",
                    "producer_workflow_path", "producer_tooling_revision", "producer_workflow_ref",
                    "signer_workflow_path", "signer_tooling_revision", "release_branch", "manual_actors",
                    "automatic_signer_actors", "max_age_seconds", "candidate_policy",
                    "max_artifact_bytes", "platforms"}
        self.assertEqual(set(policy), expected, "no single-platform v2 fields or implicit migration")
        self.assertEqual(policy["schema"], "craft-nightly-release-policy/v3")
        self.assertEqual(policy["manual_actors"], [])
        self.assertEqual(policy["automatic_signer_actors"], [])
        for key in ("repository_id", "producer_workflow_id", "producer_tooling_revision",
                    "producer_workflow_ref", "signer_tooling_revision", "max_age_seconds",
                    "max_artifact_bytes"):
            with self.subTest(unresolved=key):
                self.assertIsNone(policy[key], "the example must not invent production authority")

    def test_policy_platforms_are_exactly_what_the_producer_builds(self):
        policy = json.loads((ROOT / "docs/nightly-signing-policy.example.json").read_text())
        built = producer_platforms((ROOT / ".github/workflows/nightly.yml").read_text())
        self.assertEqual(built, list(NIGHTLY_PLATFORMS), "one release carries Windows, Linux and macOS")
        self.assertEqual(set(policy["platforms"]), set(built), "a built platform without policy, or the reverse")
        for platform, row in policy["platforms"].items():
            field, _ = NIGHTLY_TRUST[platform]
            with self.subTest(platform=platform):
                self.assertEqual(set(row), {"unsigned_artifact_name", "trust"})
                self.assertEqual(set(row["trust"]), {field}, "each platform has only its own trust anchor")

    def test_candidate_example_has_exact_v3_fields_without_provider_scope_claim(self):
        candidate = json.loads((ROOT / "docs/nightly-signing-policy.example.json").read_text())["candidate_policy"]
        self.assertEqual(set(candidate), {"schema", "epoch", "repository", "branch", "required_workflows", "sonar"})
        self.assertEqual(candidate["schema"], "craft-nightly-candidate-policy/v3")
        self.assertIsNone(candidate["required_workflows"])
        self.assertEqual(set(candidate["sonar"]), {"organization", "project", "app_id", "check_name",
                         "details_url", "issue_types", "accepted_conditions", "required_conditions", "debt"})
        self.assertTrue(all(value is None for value in candidate["sonar"].values()), "real identity/debt is unresolved")

    def test_scheduled_dispatch_transport_remains_explicitly_off(self):
        for name in ("nightly-dispatch.yml",):
            source = (ROOT / ".github/workflows" / name).read_text()
            jobs = source.split("\njobs:\n", 1)[1]
            headers = re.findall(r"(?m)^  (?![\s#])[^\n]+", jobs)
            conditions = job_conditions(jobs)
            with self.subTest(workflow=name):
                self.assertTrue(headers)
                self.assertEqual(len(conditions), len(headers), "every job needs a literal-off condition")
                for _, expression in conditions:
                    self.assertTrue(literal_off(expression), "nightly activation requires reviewed test update")

    def test_one_producer_and_one_signer_job_carry_every_platform(self):
        for name, job in (("nightly.yml", "unsigned"), ("nightly-sign.yml", "release")):
            jobs = (ROOT / ".github/workflows" / name).read_text().split("\njobs:\n", 1)[1]
            with self.subTest(workflow=name):
                self.assertEqual(re.findall(r"(?m)^  ([^\s#][^:]*):", jobs), [job],
                                 "a platform is a list entry, not a separately activatable job")
        signer = (ROOT / ".github/workflows/nightly-sign.yml").read_text()
        self.assertNotRegex(signer, r"(?m)^\s*platforms?:", "the release policy, not the caller, admits platforms")

    def test_malformed_or_narrowed_platform_lists_are_rejected(self):
        source = (ROOT / ".github/workflows/nightly.yml").read_text()
        current = "platforms: '[\"windows-x86_64\",\"linux-x86_64\",\"macos-aarch64\"]'"
        self.assertIn(current, source)
        for new in ("platforms: '[\"linux-x86_64\"]'",
                    "platforms: '[\"linux-x86_64\",\"macos-aarch64\"]'",
                    "platforms: '[\"windows-x86_64\",\"linux-x86_64\",\"linux-x86_64\"]'",
                    "platforms: '[\"windows-x86_64\",\"macos-x86_64\"]'",
                    "platforms: '[\"windows-x86_64\",\"linux-arm64\"]'",
                    "platforms: '[\"windows-x86_64\", \"linux-x86_64\"]'",
                    "platforms: [\"windows-x86_64\",\"linux-x86_64\"]",
                    "platforms: ${{ vars.NIGHTLY_PLATFORMS }}",
                    "platforms: '{\"windows-x86_64\":1}'",
                    current + "\n      " + current):
            with self.subTest(change=new), self.assertRaises(ValueError):
                producer_platforms(source.replace(current, new))
        with self.assertRaises(ValueError):
            producer_platforms(source.replace("      " + current + "\n", ""))

    def test_continuation_lines_of_any_scalar_form_reach_the_guard_checks(self):
        source = (ROOT / ".github/workflows/nightly-dispatch.yml").read_text()
        guard = re.search(r"(?m)^    if: >-\n(?:      [^\n]*\n)+", source)[0]
        for replacement in ("    if: false && github.event_name == 'workflow_run'\n      || true\n",
                            "    if: false && github.event_name == 'workflow_run'\n     || true\n",
                            "    if: >-\n      false && github.event_name == 'workflow_run'\n        || true\n",
                            "    if: |\n      false && github.event_name == 'workflow_run'\n      || true\n"):
            jobs = source.replace(guard, replacement).split("\njobs:\n", 1)[1]
            with self.subTest(replacement=replacement):
                (_, expression), = job_conditions(jobs)
                self.assertTrue(expression.endswith("|| true"))
                self.assertFalse(literal_off(expression))
        for name, workflow in (("nightly.yml", "nightly.yml"), ("nightly-sign.yml", "nightly-sign.yml")):
            source = (ROOT / ".github/workflows" / workflow).read_text()
            expected = NightlyEnabledCallers.GUARDS[name]
            guard = re.search(r"(?m)^    if: >-\n(?:      [^\n]*\n)+", source)[0]
            for replacement in (guard + "     || true\n", "    if: " + expected + "\n      || true\n"):
                jobs = source.replace(guard, replacement).split("\njobs:\n", 1)[1]
                with self.subTest(workflow=name, replacement=replacement[-30:]):
                    (indicator, expression), = job_conditions(jobs)
                    self.assertFalse(indicator == ">-" and " ".join(expression.split()) == expected)

    def test_dormancy_guard_rejects_activation_and_top_level_or(self):
        for condition in ("true", "vars.ENABLE_NIGHTLY", "false || true", "false && true || true",
                          "false && (true || true", "false && true) || true"):
            with self.subTest(condition=condition):
                self.assertFalse(literal_off(condition))
        self.assertTrue(literal_off("${{ false }}"))
        self.assertTrue(literal_off("false && (github.event_name == 'schedule' || github.event_name == 'workflow_dispatch')"))

    def test_no_nightly_schedule_is_registered(self):
        for name, event in (("nightly.yml", "workflow_dispatch"),
                            ("nightly-dispatch.yml", "workflow_run"),
                            ("nightly-sign.yml", "workflow_dispatch")):
            source = (ROOT / ".github/workflows" / name).read_text()
            events = source.split("\non:\n", 1)[1].split("\npermissions:", 1)[0]
            triggers = [line for line in events.splitlines() if line.startswith("  ")
                        and not line.startswith("   ") and line.strip() and not line.lstrip().startswith("#")]
            with self.subTest(workflow=name):
                self.assertEqual(triggers, [f"  {event}:"], "schedule/activation needs explicit review")

    def test_signer_serialization_contract_is_enforced(self):
        assert_nightly_serialization((ROOT / ".github/workflows/nightly-sign.yml").read_text())

    def test_weakened_or_malformed_serialization_is_rejected(self):
        source = (ROOT / ".github/workflows/nightly-sign.yml").read_text()
        for old, new in (
            ("queue: max", "queue: single"),
            ("queue: max", "queue: invalid"),
            ("queue: max", "queue: ${{ github.run_id }}"),
            ("queue: max", "queue:"),
            ("queue: max", "queue: max\n  queue: single"),
            ("cancel-in-progress: false", "cancel-in-progress: true"),
            ("cancel-in-progress: false", "cancel-in-progress: ${{ github.event_name == 'push' }}"),
            ("cancel-in-progress: false\n", ""),
            ("group: photocraft-nightly-release", "group: ${{ github.run_id }}"),
            ("group: photocraft-nightly-release", "group: release-${{ github.ref }}"),
            ("concurrency:\n", "concurrency: ${{ github.run_id }}\n"),
            ("concurrency:\n", "  concurrency:\n"),
        ):
            with self.subTest(change=new), self.assertRaises(ValueError):
                assert_nightly_serialization(source.replace(old, new))

    def test_no_cache_actions_or_cache_setup_options_in_app_trusted_closure(self):
        # Required R3 static guard for app workflows only; activation must extend
        # this check to every reviewed immutable shared trusted callee.
        for name in ("nightly.yml", "nightly-sign.yml", "nightly-dispatch.yml"):
            source = (ROOT / ".github/workflows" / name).read_text()
            executable = "\n".join(line for line in source.splitlines()
                                   if not line.lstrip().startswith("#"))
            with self.subTest(workflow=name):
                self.assertNotRegex(executable, r"(?im)^\s*(?:-\s*)?uses:\s*[^\n]*cache")
                self.assertNotRegex(executable, r"(?im)^\s*[^\s:#]*cache[^\s:#]*:")
                self.assertEqual(workflow_environment_problems(source), [], "app callers must not bind an environment")
                if name != "nightly-sign.yml":
                    self.assertNotRegex(executable, r"(?im)^\s*secrets:")
                    self.assertNotIn("secrets.", executable)

    def test_generic_closure_recognizes_environment_keys_in_each_caller(self):
        for name in ("nightly.yml", "nightly-sign.yml", "nightly-dispatch.yml"):
            source = (ROOT / ".github/workflows" / name).read_text()
            job_field = "    if: >-\n"
            self.assertEqual(workflow_environment_problems(source), [], name)
            for key in ("environment:", "environment :"):
                with self.subTest(workflow=name, key=key):
                    with_binding = source.replace(job_field, f"    {key} release\n" + job_field, 1)
                    self.assertNotEqual(source, with_binding)
                    self.assertRegex("\n".join(workflow_environment_problems(with_binding)),
                                     r"(?m)^line \d+: direct environment key$")

    def test_generic_closure_ignores_environment_text_in_dispatch_script(self):
        source = DISPATCHER.read_text()
        with_annotation = source.replace("          import json\n",
                                         '          environment : str = "release"\n          import json\n', 1)
        self.assertNotEqual(source, with_annotation)
        self.assertEqual(workflow_environment_problems(with_annotation), [])


class NightlyTrustAnchors(unittest.TestCase):
    """The public stable identities each platform's signed nightly must carry (all three platforms)."""

    def setUp(self):
        self.policy = json.loads((ROOT / "docs/nightly-signing-policy.example.json").read_text())

    def test_each_platform_row_names_its_fixed_artifact_kind_and_stable_identity(self):
        self.assertEqual(set(self.policy["platforms"]), set(NIGHTLY_TRUST))
        for platform, (field, value) in NIGHTLY_TRUST.items():
            row = self.policy["platforms"][platform]
            with self.subTest(platform=platform):
                self.assertEqual(row["unsigned_artifact_name"], f"nightly-unsigned-{platform}",
                                 "the artifact kind is fixed by platform, never a caller pattern")
                self.assertEqual(row["trust"], {field: value}, "trust must be stable's public identity")

    def test_trust_values_have_the_shapes_the_shared_signer_accepts(self):
        rows = self.policy["platforms"]
        self.assertRegex(rows["macos-aarch64"]["trust"]["apple_team_id"], r"^[A-Z0-9]{10}$")
        self.assertRegex(rows["linux-x86_64"]["trust"]["fingerprint"], r"^[0-9A-F]{40}$")
        subject = rows["windows-x86_64"]["trust"]["publisher_subject"]
        self.assertTrue(0 < len(subject) <= 2048 and not set(subject) & {"\r", "\n", "\0"})

    def test_committed_linux_key_is_exactly_the_one_stable_public_key(self):
        self.assertFalse(LINUX_KEY.is_symlink(), "the key file must be a regular committed file")
        armored = LINUX_KEY.read_text(encoding="ascii")
        self.assertEqual(armored.count("-----BEGIN PGP"), 1, "exactly one armored block")
        self.assertEqual(openpgp_primary_fingerprints(armored), [NIGHTLY_TRUST["linux-x86_64"][1]],
                         "exactly one primary key, and it is the stable release key the policy trusts")

    def test_key_reader_refuses_secret_or_extra_material(self):
        armored = LINUX_KEY.read_text(encoding="ascii")
        for change in (armored.replace("PUBLIC KEY BLOCK", "PRIVATE KEY BLOCK"),
                       armored.replace("-----END PGP PUBLIC KEY BLOCK-----\n", ""),
                       "Comment: extra\n" + armored):
            with self.subTest(change=change[:40]), self.assertRaises(ValueError):
                openpgp_primary_fingerprints(change)

    def test_both_callers_use_one_shared_pin_in_uses_and_tooling_revision(self):
        pins = set()
        for name, callee in (("nightly.yml", "nightly-unsigned.yml"), ("nightly-sign.yml", "nightly-release.yml")):
            source = (ROOT / ".github/workflows" / name).read_text()
            uses = re.findall(r"(?m)^    uses: storytold/craft-common-actions/\.github/workflows/"
                              + re.escape(callee) + r"@(\S+)$", source)
            tooling = re.findall(r"(?m)^      tooling-revision: (\S+)$", source)
            with self.subTest(workflow=name):
                self.assertEqual(len(uses), 1)
                self.assertEqual(tooling, uses, "the defining workflow SHA and tooling input must match")
            pins.update(uses)
        self.assertEqual(len(pins), 1, "producer and signer must pin the same shared commit")
        self.assertRegex(pins.pop(), r"^[0-9a-f]{40}$", "pin a full immutable commit, never a branch or placeholder")


class NightlyEnabledCallers(unittest.TestCase):
    """Producer and signer run only in their exact context, for the reviewed manual actor."""

    GUARDS = {
        "nightly.yml": "github.repository == 'storytold/photocraft' && github.ref == 'refs/heads/main' && "
                       "github.ref_type == 'branch' && (github.event_name == 'schedule' || "
                       "github.event_name == 'workflow_dispatch')",
        "nightly-sign.yml": "github.repository == 'storytold/photocraft' && github.event_name == 'workflow_dispatch' && "
                            "github.ref == 'refs/heads/nightly-signer' && github.ref_type == 'branch'",
    }

    def test_enabled_jobs_keep_exact_context_guards(self):
        for name, expected in self.GUARDS.items():
            jobs = (ROOT / ".github/workflows" / name).read_text().split("\njobs:\n", 1)[1]
            guards = job_conditions(jobs)
            with self.subTest(workflow=name):
                self.assertEqual(len(guards), 1)
                indicator, expression = guards[0]
                self.assertEqual(indicator, ">-")
                self.assertEqual(" ".join(expression.split()), expected, "guards may only change with review")

    def test_manual_actors_are_the_release_managers_in_caller_and_policy(self):
        source = (ROOT / ".github/workflows/nightly.yml").read_text()
        policy = json.loads(INSTALLED_POLICY.read_text())
        self.assertEqual(reviewed_manual_actors(source, policy), RELEASE_MANAGERS)
        self.assertEqual(policy["automatic_signer_actors"], [], "no automatic signer without a schedule")

    def test_an_actor_outside_the_release_managers_is_refused(self):
        source = (ROOT / ".github/workflows/nightly.yml").read_text()
        policy = json.loads(INSTALLED_POLICY.read_text())
        caller = json.dumps(RELEASE_MANAGERS, separators=(",", ":"))
        widened = sorted(RELEASE_MANAGERS + ["echelon-robot"], key=str.lower)
        for change in ("caller", "policy", "both", "order", "case"):
            mutated_source, mutated_policy = source, copy.deepcopy(policy)
            if change in ("caller", "both"):
                mutated_source = source.replace(caller, json.dumps(widened, separators=(",", ":")))
            if change in ("policy", "both"):
                mutated_policy["manual_actors"] = widened
            if change == "order":
                mutated_policy["manual_actors"] = list(reversed(RELEASE_MANAGERS))
            if change == "case":
                mutated_policy["manual_actors"] = [login.lower() for login in RELEASE_MANAGERS]
            with self.subTest(change=change), self.assertRaises(ValueError):
                reviewed_manual_actors(mutated_source, mutated_policy)


# Mirrors the storytold release-managers team (id 19837241) minus echelon-robot, which only has
# read access to photocraft. Update by PR, in the caller and the policy, when the team changes.
RELEASE_MANAGERS = ["bflatastic", "dylanmtaylor", "echelon", "FredApps", "skylartaylor"]


def reviewed_manual_actors(caller_source, policy):
    """The one reviewed manual-actor list, identical in the producer caller and the policy."""
    rows = re.findall(r"(?m)^      manual-actors: '(\[[^']*\])'$", caller_source)
    if len(rows) != 1:
        raise ValueError("exactly one literal manual-actors list is required")
    caller = json.loads(rows[0])
    if caller != policy["manual_actors"]:
        raise ValueError("caller and policy must grant the same actors in the same order")
    if caller != RELEASE_MANAGERS:
        raise ValueError("manual actors must be exactly the reviewed release managers")
    if caller != sorted(caller, key=str.lower) or len({login.lower() for login in caller}) != len(caller):
        raise ValueError("manual actors must be distinct and sorted case-insensitively")
    if set(caller) & {login for login in policy["automatic_signer_actors"]}:
        raise ValueError("manual and automatic actors overlap")
    return caller


INSTALLED_POLICY = ROOT / ".github/nightly-signing-policy.json"


def caller_pin(name):
    return re.findall(r"(?m)^      tooling-revision: ([0-9a-f]{40})$", (ROOT / ".github/workflows" / name).read_text())[0]


class NightlyInstalledPolicy(unittest.TestCase):
    """The installed v3 policy matches the callers, the repository and its documented sources."""

    def setUp(self):
        self.assertFalse(INSTALLED_POLICY.is_symlink())
        self.policy = json.loads(INSTALLED_POLICY.read_text())

    def test_exact_v3_fields_and_repository_identity(self):
        example = json.loads((ROOT / "docs/nightly-signing-policy.example.json").read_text())
        self.assertEqual(set(self.policy), set(example))
        self.assertEqual(self.policy["schema"], "craft-nightly-release-policy/v3")
        self.assertEqual(self.policy["repository"], "storytold/photocraft")
        self.assertEqual(self.policy["release_branch"], SIGNER_BRANCH, "the signer runs on its dedicated branch")
        # Both read from the GitHub API (the repository and nightly.yml's registration on main).
        for key in ("repository_id", "producer_workflow_id"):
            with self.subTest(key=key):
                self.assertIs(type(self.policy[key]), int)
                self.assertGreater(self.policy[key], 0)
        self.assertNotEqual(self.policy["producer_workflow_id"],
                            self.policy["candidate_policy"]["required_workflows"][0]["workflow_id"],
                            "the producer is not its own quality workflow")

    def test_paths_and_tooling_match_the_callers(self):
        pin = caller_pin("nightly.yml")
        self.assertEqual(caller_pin("nightly-sign.yml"), pin)
        self.assertEqual(self.policy["producer_workflow_path"], ".github/workflows/nightly.yml")
        self.assertEqual(self.policy["signer_workflow_path"], ".github/workflows/nightly-sign.yml")
        self.assertEqual(self.policy["producer_tooling_revision"], pin)
        self.assertEqual(self.policy["signer_tooling_revision"], pin)
        self.assertEqual(self.policy["producer_workflow_ref"],
                         "storytold/craft-common-actions/.github/workflows/nightly-unsigned.yml@" + pin)

    def test_platforms_equal_the_reviewed_example_and_producer(self):
        example = json.loads((ROOT / "docs/nightly-signing-policy.example.json").read_text())
        self.assertEqual(self.policy["platforms"], example["platforms"])
        built = producer_platforms((ROOT / ".github/workflows/nightly.yml").read_text())
        self.assertEqual(set(self.policy["platforms"]), set(built))

    def test_bounds(self):
        self.assertIs(type(self.policy["max_age_seconds"]), int)
        self.assertTrue(0 < self.policy["max_age_seconds"] <= 604800)
        self.assertIs(type(self.policy["max_artifact_bytes"]), int)
        self.assertTrue(0 < self.policy["max_artifact_bytes"] <= 16 * 1024 ** 3)

    def test_candidate_workflows_run_on_every_main_push(self):
        candidate = self.policy["candidate_policy"]
        self.assertEqual(set(candidate), {"schema", "epoch", "repository", "branch", "required_workflows", "sonar"})
        self.assertEqual(candidate["schema"], "craft-nightly-candidate-policy/v3")
        self.assertEqual((candidate["repository"], candidate["branch"]), ("storytold/photocraft", "main"))
        self.assertTrue(isinstance(candidate["epoch"], str) and 0 < len(candidate["epoch"]) <= 128)
        rows = candidate["required_workflows"]
        self.assertTrue(rows)
        for row in rows:
            with self.subTest(workflow=row["workflow_path"]):
                self.assertEqual(set(row), {"workflow_id", "workflow_path", "jobs"})
                self.assertIs(type(row["workflow_id"]), int)
                self.assertNotEqual(row["workflow_path"], self.policy["producer_workflow_path"])
                source = (ROOT / row["workflow_path"]).read_text()
                push = re.search(r"(?m)^  push:\n((?:    .*\n)+)", source)
                self.assertIsNotNone(push, "a required workflow must run on main pushes")
                self.assertIn("    branches: [main]\n", push[1])
                self.assertNotRegex(push[1], r"paths", "a path filter would leave candidates without a run")
                names = [job["name"] for job in row["jobs"]]
                self.assertEqual(len(names), len(set(names)))
                for job in row["jobs"]:
                    self.assertEqual(set(job), {"name", "app_id", "applicability"})
                    self.assertEqual((job["app_id"], job["applicability"]), (15368, "required"))

    def test_sonar_identity_and_reviewed_scope(self):
        sonar = self.policy["candidate_policy"]["sonar"]
        properties = dict(line.split("=", 1) for line in (ROOT / "sonar-project.properties").read_text().splitlines()
                          if line and not line.startswith("#"))
        self.assertEqual(sonar["organization"], properties["sonar.organization"])
        self.assertEqual(sonar["project"], properties["sonar.projectKey"])
        self.assertEqual((sonar["app_id"], sonar["check_name"]), (12526, "SonarCloud Code Analysis"))
        self.assertEqual(sonar["details_url"], "https://sonarcloud.io/dashboard?id=" + sonar["project"] + "&branch=main")
        accepted = {"bugs": "BUG", "vulnerabilities": "VULNERABILITY", "code_smells": "CODE_SMELL",
                    "security_rating": "VULNERABILITY", "new_security_rating": "VULNERABILITY"}
        self.assertEqual({accepted[c["metric"]] for c in sonar["accepted_conditions"]}, set(sonar["issue_types"]))
        metrics = [c["metric"] for c in sonar["accepted_conditions"] + sonar["required_conditions"]]
        self.assertEqual(len(metrics), len(set(metrics)))
        keys = [item["key"] for item in sonar["debt"]]
        self.assertEqual(len(keys), len(set(keys)))
        for item in sonar["debt"]:
            with self.subTest(issue=item["key"]):
                self.assertEqual(set(item), {"key", "rule", "component", "type", "severity", "hash"})
                self.assertTrue(item["component"].startswith(sonar["project"] + ":"))
                self.assertIn(item["type"], sonar["issue_types"])
                self.assertRegex(item["hash"], r"^[0-9a-f]{32}$")


SIGNER_BRANCH = "nightly-signer"
SHARED = "storytold/craft-common-actions/.github/workflows/"
# App workflows reviewed to call the shared stable release.yml. That callee must carry the
# stable-only environment guard (dry runs and non-release refs bind no environment); the
# nightly-signer settings scan checks the resolved callee before creating or advancing the branch.
REVIEWED_STABLE_RELEASE_CALLERS = {"release.yml"}


def app_workflows():
    return {path.name: path.read_text() for path in sorted((ROOT / ".github/workflows").glob("*.y*ml"))}


WORKFLOW_KEY = re.compile(r'''("(?:[^"\\]|\\.)*"|'(?:[^']|'')*'|[^\s'"#{}\[\],:&*!|>%@`?-][^:#,{}]*?|-[^\s:#][^:#,{}]*?)\s*:(?:\s+(.*))?$''')


def _workflow_key(text):
    if text.startswith('"'):
        return None if "\\" in text else text[1:-1]
    if text.startswith("'"):
        return text[1:-1].replace("''", "'")
    return text


def _workflow_value(text):
    """A one-line scalar value without its comment and quotes; None when escaped or unterminated."""
    text = text.strip()
    if text.startswith('"'):
        quoted = re.match(r'"((?:[^"\\]|\\.)*)"\s*(?:#.*)?$', text)
        return None if quoted is None or "\\" in quoted.group(1) else quoted.group(1)
    if text.startswith("'"):
        quoted = re.match(r"'((?:[^']|'')*)'\s*(?:#.*)?$", text)
        return None if quoted is None else quoted.group(1).replace("''", "'")
    return re.split(r"\s+#", text, maxsplit=1)[0].strip()


def _strip_comment(text):
    """(text without its comment, unterminated) using a quote-aware scan.

    As in YAML, a quote opens a quoted scalar only where a scalar starts (line start or after
    `:`, `-`, `,`, `{` or `[`), and a `#` starts a comment only outside quotes, at the start or
    after whitespace.
    """
    quote, index = None, 0
    while index < len(text):
        char = text[index]
        if quote == '"':
            if char == "\\":
                index += 2
                continue
            if char == '"':
                quote = None
        elif quote == "'":
            if char == "'":
                if text[index + 1:index + 2] == "'":
                    index += 2
                    continue
                quote = None
        elif char in "\"'" and text[:index].rstrip()[-1:] in ("", ":", "-", ",", "{", "["):
            quote = char
        elif char == "#" and (index == 0 or text[index - 1] in " \t"):
            return text[:index].rstrip(), False
        index += 1
    return text.rstrip(), quote is not None


FLOW_QUOTED = re.compile(r"\"(?:[^\"\\]|\\.)*\"|'(?:[^']|'')*'")
FLOW_FORBIDDEN_KEYS = {"environment", "uses"}


def workflow_entries(source):
    """Every mapping entry in a workflow, plus the YAML forms this reader refuses.

    A narrow reader, not a YAML parser (none ships with the runner's Python). Each entry is
    (line, key, value, multiline): plain, single- and double-quoted keys and one-line values in
    block and simple one-line flow mappings. Block scalar bodies are skipped, and an entry whose
    value is a block scalar or continues on the next lines is marked multiline. It refuses whatever
    could hide a key from it: escaped double-quoted keys, explicit `?` keys, merge keys, anchors,
    aliases, tags, tab indentation and nested or multi-line flow.
    """
    lines = source.splitlines()
    entries, refused, block = [], [], None

    def continues(index, indent):
        for follow in lines[index + 1:]:
            stripped = follow.strip()
            if not stripped or stripped.startswith("#"):
                continue
            deeper = len(follow) - len(follow.lstrip(" ")) > indent
            return deeper and not stripped.startswith("- ") and WORKFLOW_KEY.match(stripped) is None
        return False

    for index, raw in enumerate(lines):
        number = index + 1
        stripped = raw.strip()
        indent = len(raw) - len(raw.lstrip(" "))
        if block is not None:
            if not stripped or indent > block:
                continue
            block = None
        if not stripped or stripped.startswith("#"):
            continue
        if "\t" in raw[:indent + 1]:
            refused.append(f"line {number}: tab indentation"); continue
        body, unterminated = _strip_comment(stripped)
        if unterminated:
            refused.append(f"line {number}: unterminated or multi-line quoted scalar"); continue
        if not body:
            continue
        while body.startswith("- "):
            body = body[2:].lstrip()
        if body == "?" or body.startswith("? "):
            refused.append(f"line {number}: explicit key"); continue
        match = WORKFLOW_KEY.match(body)
        value = body
        if match is not None:
            key, value = _workflow_key(match.group(1)), match.group(2) or ""
            if key is None:
                refused.append(f"line {number}: escaped double-quoted key"); continue
            if key == "<<":
                refused.append(f"line {number}: merge key"); continue
        plain = re.sub(r"\$\{\{.*?\}\}", "", value).strip()
        bare = re.sub(r"\"(?:[^\"\\]|\\.)*\"|'(?:[^']|'')*'", "''", plain)
        if re.match(r"[&*!]", plain):
            refused.append(f"line {number}: anchor, alias or tag")
        is_block = re.fullmatch(r"[|>][0-9+-]*", plain) is not None
        if match is not None:
            entries.append((number, key, _workflow_value(value),
                            is_block or ((value == "" or not is_block) and continues(index, indent))))
        if "{" in bare and bare != "{}":
            flow = re.fullmatch(r"\{([^{}\[\]]*)\}", FLOW_QUOTED.sub("''", value.strip()))
            if any(set(scalar[1:-1]) & set("#{}[],") for scalar in FLOW_QUOTED.findall(value)):
                refused.append(f"line {number}: flow mapping quoted scalar with # or flow punctuation")
            elif flow is None:
                refused.append(f"line {number}: nested or multi-line flow mapping")
            elif not re.fullmatch(r"\{([^{}\[\]]*)\}", value.strip()):
                refused.append(f"line {number}: nested or multi-line flow mapping")
            else:
                for item in flow.group(1).split(","):
                    entry = WORKFLOW_KEY.match(item.strip())
                    key = _workflow_key(entry.group(1)) if entry else None
                    if key is None:
                        refused.append(f"line {number}: unreadable flow mapping entry")
                    elif key in FLOW_FORBIDDEN_KEYS:
                        refused.append(f"line {number}: {key} in a flow mapping (must be block style)")
                    else:
                        entries.append((number, key, _workflow_value(entry.group(2) or ""), False))
        if match is not None and is_block:
            block = indent
    return entries, refused


def workflow_keys(source):
    """(line, key) for every mapping entry, plus the refused YAML forms."""
    entries, refused = workflow_entries(source)
    return [(number, key) for number, key, _, _ in entries], refused


def workflow_environment_problems(source):
    keys, refused = workflow_keys(source)
    return refused + [f"line {line}: direct environment key" for line, key in keys if key.casefold() == "environment"]


SHARED_CALLEE = re.compile(r"storytold/craft-common-actions/\.github/workflows/([\w.-]+\.ya?ml)@(\S+)", re.IGNORECASE)
INSPECTED_KEYS = {"environment", "uses"}


def trust_closure_problems(workflows):
    """The nightly-signer trust-closure scan over app workflow sources; empty when the tree is safe.

    Every key it inspects comes from workflow_entries(), never a regex over the raw text.
    """
    problems, stable_callers, signer_calls = [], set(), 0
    for name, source in workflows.items():
        entries, refused = workflow_entries(source)
        problems.extend(name + " uses YAML the scan cannot read (" + reason + ")" for reason in refused)
        for number, key, value, multiline in entries:
            if key not in INSPECTED_KEYS:
                continue
            if key == "environment":
                problems.append(f"{name} binds an environment directly (line {number})")
                continue
            if value is None or multiline:
                problems.append(f"{name} has a uses value the scan cannot read (line {number})")
                continue
            callee = SHARED_CALLEE.fullmatch(value)
            if callee is None:
                if re.search(r"craft-common-actions", value, re.IGNORECASE) and re.search(r"workflows/(nightly-)?release\.ya?ml", value, re.IGNORECASE):
                    problems.append(f"{name} has an unrecognised shared release caller (line {number})")
                continue
            workflow, ref = callee.group(1), callee.group(2)
            if workflow.lower() in ("nightly-release.yml", "nightly-release.yaml"):
                if name == "nightly-sign.yml":
                    signer_calls += 1
                if name != "nightly-sign.yml" or not re.fullmatch(r"[0-9a-f]{40}", ref):
                    problems.append(name + " calls nightly-release.yml@" + ref)
            if workflow.lower() in ("release.yml", "release.yaml"):
                stable_callers.add(name)
                if ref != "main" and not re.fullmatch(r"[0-9a-f]{40}", ref):
                    problems.append(name + " calls release.yml@" + ref + "; use @main or a reviewed full SHA")
    if stable_callers != REVIEWED_STABLE_RELEASE_CALLERS:
        problems.append("shared release.yml callers are " + ", ".join(sorted(stable_callers)))
    if signer_calls != 1:
        problems.append("nightly-sign.yml must call nightly-release.yml exactly once")
    return problems


class NightlySignerBranchTrustClosure(unittest.TestCase):
    """On refs/heads/nightly-signer, only the pinned signer caller may reach an environment."""

    def test_trust_closure_scan_passes_on_this_tree(self):
        self.assertEqual(trust_closure_problems(app_workflows()), [])

    def test_trust_closure_scan_refuses_each_route_into_an_environment(self):
        tree = app_workflows()
        bad = {
            "a workflow binds an environment": {**tree, "extra.yml": "jobs:\n  x:\n    runs-on: ubuntu-latest\n    environment: release\n"},
            "a double-quoted environment key": {**tree, "extra.yml": 'jobs:\n  x:\n    runs-on: ubuntu-latest\n    "environment": release\n'},
            "a single-quoted environment key": {**tree, "extra.yml": "jobs:\n  x:\n    runs-on: ubuntu-latest\n    'environment' : release\n"},
            "an environment mapping": {**tree, "extra.yml": "jobs:\n  x:\n    runs-on: ubuntu-latest\n    environment:\n      name: release\n"},
            "an environment in a flow mapping": {**tree, "extra.yml": "jobs:\n  x: {runs-on: ubuntu-latest, environment: release}\n"},
            "an escaped key": {**tree, "extra.yml": 'jobs:\n  x:\n    runs-on: ubuntu-latest\n    "environ\\u006dent": release\n'},
            "a merge key": {**tree, "extra.yml": "base: &b\n  runs-on: ubuntu-latest\njobs:\n  x:\n    <<: *b\n"},
            "an explicit key": {**tree, "extra.yml": "jobs:\n  x:\n    ? environment\n    : release\n"},
            "a nested flow mapping": {**tree, "extra.yml": "jobs:\n  x: {steps: [{run: a}]}\n"},
            "a double-quoted uses key copying the signer": {**tree, "extra.yml": 'jobs:\n  x:\n    "uses": ' + SHARED + "nightly-release.yml@" + "a" * 40 + "\n"},
            "a single-quoted uses key": {**tree, "extra.yml": "jobs:\n  x:\n    'uses': " + SHARED + "nightly-release.yml@" + "a" * 40 + "\n"},
            "a quoted uses value": {**tree, "extra.yml": 'jobs:\n  x:\n    uses: "' + SHARED + "nightly-release.yml@" + "a" * 40 + '"\n'},
            "uses in a flow mapping": {**tree, "extra.yml": "jobs:\n  x: {uses: " + SHARED + "nightly-release.yml@" + "a" * 40 + "}\n"},
            "a differently cased owner": {**tree, "extra.yml": "jobs:\n  x:\n    uses: Storytold/Craft-Common-Actions/.github/workflows/nightly-release.yml@" + "a" * 40 + "\n"},
            "a folded uses value": {**tree, "extra.yml": "jobs:\n  x:\n    uses: >-\n      " + SHARED + "nightly-release.yml@" + "a" * 40 + "\n"},
            "a continued uses value": {**tree, "extra.yml": "jobs:\n  x:\n    uses: " + SHARED + "\n      nightly-release.yml@" + "a" * 40 + "\n"},
            "a quoted second signer call": {**tree, "nightly-sign.yml": tree["nightly-sign.yml"] + "  again:\n    \"uses\": " + SHARED + "nightly-release.yml@" + "a" * 40 + "\n"},
            "a hash inside a quoted flow scalar": {**tree, "extra.yml": 'jobs:\n  x: {name: "a} # b", runs-on: ubuntu-latest, environment: release, steps: []}\n'},
            "a single-quoted hash in a flow scalar": {**tree, "extra.yml": "jobs:\n  x: {name: 'a # b', runs-on: ubuntu-latest}\n"},
            "a comma inside a quoted flow scalar": {**tree, "extra.yml": 'jobs:\n  x: {name: "a, environment: release", runs-on: ubuntu-latest}\n'},
            "a bracket inside a quoted flow scalar": {**tree, "extra.yml": 'jobs:\n  x: {name: "[a]", runs-on: ubuntu-latest}\n'},
            "an unterminated quote": {**tree, "extra.yml": 'jobs:\n  x:\n    name: "a\n      b"\n    runs-on: ubuntu-latest\n'},
            "environment in any flow mapping": {**tree, "extra.yml": "jobs:\n  x:\n    runs-on: ubuntu-latest\n    with: {environment: release}\n"},
            "uses in any flow mapping": {**tree, "extra.yml": "jobs:\n  x:\n    steps:\n      - {uses: actions/checkout@v4}\n"},
            "another caller of the signer": {**tree, "extra.yml": "jobs:\n  x:\n    uses: " + SHARED + "nightly-release.yml@" + "a" * 40 + "\n"},
            "a mutable signer pin": {**tree, "nightly-sign.yml": re.sub(r"nightly-release\.yml@[0-9a-f]{40}", "nightly-release.yml@main", tree["nightly-sign.yml"])},
            "a new stable release caller": {**tree, "extra.yml": "jobs:\n  x:\n    uses: " + SHARED + "release.yml@main\n"},
            "a stable caller on a branch or tag ref": {**tree, "release.yml": tree["release.yml"].replace(SHARED + "release.yml@main", SHARED + "release.yml@v1")},
        }
        for label, workflows in bad.items():
            with self.subTest(case=label):
                self.assertTrue(trust_closure_problems(workflows))

    def test_signer_dispatch_guard_and_policy_name_the_same_branch(self):
        sign = (ROOT / ".github/workflows/nightly-sign.yml").read_text()
        (_, guard), = job_conditions(sign.split("\njobs:\n", 1)[1])
        self.assertIn("github.ref == 'refs/heads/" + SIGNER_BRANCH + "'", guard)
        self.assertNotIn("refs/heads/release", sign, "the signer no longer runs on the stable branch")
        self.assertEqual(re.findall(r"(?m)^      app-name: (\S+)$", sign), ["photocraft"], "the shared signer needs the reviewed registry app")
        dispatch = inline_scripts(DISPATCHER)[0]
        self.assertEqual(re.findall(r"'ref': '([^']+)'", dispatch), ["refs/heads/" + SIGNER_BRANCH])
        self.assertEqual(json.loads(INSTALLED_POLICY.read_text())["release_branch"], SIGNER_BRANCH)
        self.assertEqual(json.loads((ROOT / "docs/nightly-signing-policy.example.json").read_text())["release_branch"],
                         SIGNER_BRANCH)


if __name__ == "__main__":
    unittest.main(verbosity=2)
