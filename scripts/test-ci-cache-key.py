"""The `test (without heif)` job restores the gate leg's dependency cache.

The key itself is the shared action's (craft-common-actions `actions/rust-cache`), so this checks
the caller side only: the job passes the gate's entry name, never saves, aligns the environment that
rust-cache hashes, and keeps the HEIF-disabled command it had. The recorded keys below are from CI
run 43259488164 at head ebdf906eb561a6d129fbe686fdbc260828ae8f26: job 131326987385 (gate) and job
131326987527 (test (without heif)). They were the same lockfile and toolchain; the profile fragment
and the environment hash differed, and this job restored nothing.
"""
from pathlib import Path
import hashlib
import re
import tomllib
import unittest

ROOT = Path(__file__).resolve().parents[1]
# Temporary exact ref for craft-common-actions#240; restore @main with the workflow after it lands.
CACHE_ACTION = 'storytold/craft-common-actions/actions/rust-cache@4d6d029582837014919554f99e24a5cd3a961ba5'
WORKFLOW = (ROOT / '.github/workflows/ci.yml').read_text(encoding='utf-8')

GATE_KEY = ('v1-rust-133a530327fd50970e187efbd0c6976d175319b8511aed506db5f52db749b9b0'
            '-gate-Linux-x64-95e50156-d7643e46')
RECORDED_PROFILE = '133a530327fd50970e187efbd0c6976d175319b8511aed506db5f52db749b9b0'
RECORDED_FILE_HASH = '7dbd81dc4c6a93e1ed78c223cb5c03727fada07a1106dd0a8cab53e7e29bfe76'
RECORDED_LOCK = 'd7643e46'
RECORDED_WITHOUT_HEIF = (f'v1-rust-{RECORDED_FILE_HASH}-gate-Linux-x64-79b1c3ad-{RECORDED_LOCK}')
# From the same two jobs' logs: `rustc -vV`, and the variables rust-cache hashed (it hashes every
# CARGO*, CC*, CFLAGS*, CXX*, CMAKE* and RUST* variable with a value).
RUST = '1.99.1 x86_64-unknown-linux-gnu 58cf8f9c35b355199727627d07ae34a4e2846d67'
GATE_VARIABLES = {'CARGO_INCREMENTAL': '0', 'CARGO_NET_RETRY': '10',
                  'CARGO_TERM_COLOR': 'always', 'RUST_BACKTRACE': '1'}
TOOLCHAIN_EXPORT = {'CARGO_HOME': '/home/runner/.cargo'}
PREFIXES = ('CARGO', 'CC', 'CFLAGS', 'CXX', 'CMAKE', 'RUST')


def job(name):
    """One top-level job's text from the workflow."""
    start = re.search(rf'^  {re.escape(name)}:\n', WORKFLOW, re.M)
    if start is None:
        raise AssertionError(f'no job {name!r} in .github/workflows/ci.yml')
    rest = WORKFLOW[start.end():]
    end = re.search(r'^  [a-z][a-z0-9-]*:\n', rest, re.M)
    return rest[:end.start()] if end else rest


def workflow_environment():
    """The `env:` block at the top of the workflow, as KEY: value pairs."""
    block = re.search(r'^env:\n((?:  [A-Za-z_][A-Za-z0-9_]*:.*\n)+)', WORKFLOW, re.M)
    if block is None:
        raise AssertionError('no workflow-level env block')
    return dict(re.findall(r'^  ([A-Za-z_][A-Za-z0-9_]*):\s*(.*?)\s*$', block.group(1), re.M))


def job_environment(job_text):
    """The job's `env:` block, as KEY: value pairs (comments and blank lines skipped)."""
    lines = job_text.splitlines()
    try:
        start = lines.index('    env:')
    except ValueError:
        return {}
    pairs = {}
    for line in lines[start + 1:]:
        if line.strip() and not line.startswith('      '):
            break
        found = re.match(r'^      ([A-Za-z_][A-Za-z0-9_]*):\s*(.*?)\s*$', line)
        if found:
            pairs[found.group(1)] = found.group(2)
    return pairs


def hashed(environment):
    """The variables rust-cache hashes, exactly as src/config.ts filters them."""
    return {name: value for name, value in environment.items()
            if name.startswith(PREFIXES) and value and not value.startswith('${{')}


def cache_key(profile, job_name, environment, lock=RECORDED_LOCK):
    hasher = hashlib.sha1()
    hasher.update(RUST.encode())
    for name in sorted(environment):
        # rust-cache hashes a variable only when it has a value; an empty one is dropped.
        if environment[name]:
            hasher.update(f'{name}={environment[name]}'.encode())
    return f'v1-rust-{profile}-{job_name}-Linux-x64-{hasher.hexdigest()[:8]}-{lock}'


def profile_key(manifest):
    profiles = tomllib.loads(manifest).get('profile', {})
    import json
    return hashlib.sha256(json.dumps(profiles, sort_keys=True, separators=(',', ':')).encode()).hexdigest()


class WithoutHeifJobTests(unittest.TestCase):
    def setUp(self):
        self.text = job('test-without-heif')

    def test_restores_the_gate_entry_through_the_shared_action(self):
        self.assertIn(f'uses: {CACHE_ACTION}', self.text)
        self.assertIn('job: gate', self.text)
        self.assertIn('align-environment: true', self.text)

    def test_the_gate_and_restore_explicitly_select_the_same_cache_version(self):
        versions = []
        for text in (job('gate'), self.text):
            found = re.findall(r'^[ \t]+cache-version:[ \t]*([^ \t\r\n#]+)[ \t]*(?:#.*)?$', text, re.M)
            self.assertEqual(len(found), 1, 'both jobs must name one cache-version explicitly')
            self.assertRegex(found[0], r'^[A-Za-z0-9._-]+$')
            versions.append(found[0])
        self.assertEqual(*versions, 'the restore must follow a gate namespace bump')

    def test_never_saves_a_cache(self):
        # Only the default branch saves, and this job must not write over the gate's entry.
        self.assertIn('save-if: "false"', self.text)

    def test_does_not_carry_its_own_copy_of_the_key(self):
        for copied in ("prefix-key:", "shared-key:", "hashFiles('Cargo.toml')",
                       'step-security/rust-cache'):
            with self.subTest(copied=copied):
                self.assertNotIn(copied, self.text)

    def test_keeps_the_heif_disabled_command_and_runner(self):
        self.assertIn('cargo nextest run -p photocraft-io --locked --profile ci', self.text)
        self.assertNotIn('--features heif', self.text)
        # The gate's runner: a rust-cache entry is keyed on the OS.
        self.assertIn('runs-on: ubuntu-24.04', self.text)

    def test_the_gate_inputs_add_no_unmatched_linux_cache_variables(self):
        gate = job('gate')
        self.assertRegex(gate, r'(?m)^      dev-debug: ""[ \t]*$')
        # Linux inherits env + linux-env, not windows-env. Read only the literal assignments this
        # caller uses; a new expression/encoding needs an explicit cache-contract review.
        for name in ('env', 'linux-env'):
            found = re.search(rf'^      {name}:[ \t]*(.*)$', gate, re.M)
            if found is None:
                continue
            value = found[1].strip()
            if value in ('|', '|-', '|+'):
                lines = []
                for line in gate[found.end() + 1:].splitlines():
                    if line.strip() and len(line) - len(line.lstrip(' ')) <= 6:
                        break
                    lines.append(line.strip())
                value = '\n'.join(lines)
            else:
                value = value.strip('"\'')
            for assignment in value.splitlines():
                if not assignment.strip():
                    continue
                key, separator, _ = assignment.partition('=')
                self.assertEqual(separator, '=', f'{name} is not a literal assignment')
                self.assertRegex(key, r'^[A-Za-z_][A-Za-z0-9_]*$')
                self.assertFalse(key.startswith(PREFIXES), f'{name} adds unmatched cache input {key}')

    def test_hashes_exactly_the_gate_legs_variables(self):
        environment = {**workflow_environment(), **job_environment(self.text)}
        self.assertEqual(hashed(environment), GATE_VARIABLES)


class KeyTests(unittest.TestCase):
    def test_the_alignment_reaches_the_gate_env_hash(self):
        without_home = cache_key(RECORDED_PROFILE, 'gate', GATE_VARIABLES)
        self.assertEqual(without_home, GATE_KEY)
        # What the job's toolchain action exported, and what a key built from this manifest gave.
        self.assertEqual(cache_key(RECORDED_PROFILE, 'gate', {**GATE_VARIABLES, **TOOLCHAIN_EXPORT}),
                         GATE_KEY.replace('95e50156', '79b1c3ad'))
        self.assertEqual(cache_key(RECORDED_FILE_HASH, 'gate', {**GATE_VARIABLES, **TOOLCHAIN_EXPORT}),
                         RECORDED_WITHOUT_HEIF)

    def test_an_empty_cargo_home_is_not_hashed(self):
        # rust-cache hashes `KEY=VALUE` only for a non-empty value, which is what the action's
        # `align-environment` uses to match a leg that never had CARGO_HOME.
        self.assertEqual(cache_key(RECORDED_PROFILE, 'gate', {**GATE_VARIABLES, 'CARGO_HOME': ''}),
                         GATE_KEY)
        self.assertNotEqual(cache_key(RECORDED_PROFILE, 'gate', {**GATE_VARIABLES, **TOOLCHAIN_EXPORT}),
                            GATE_KEY)

    def test_the_manifest_hash_is_not_the_profile_hash(self):
        raw = (ROOT / 'Cargo.toml').read_bytes()
        self.assertNotEqual(profile_key(raw.decode('utf-8')),
                            hashlib.sha256(hashlib.sha256(raw).digest()).hexdigest())


if __name__ == '__main__':
    unittest.main()
