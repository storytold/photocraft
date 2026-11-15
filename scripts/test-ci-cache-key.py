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


def literal_scalar(source):
    # Only the literal scalar spellings used by this cache call are supported. Reject aliases,
    # expressions, escapes and other YAML encodings instead of silently guessing their value.
    match = re.fullmatch(r'''(?:"([^"\\]*)"|'([^']*)'|([A-Za-z0-9_./@-]+))[ \t]*(?:#.*)?''', source.strip())
    if match is None:
        raise AssertionError(f'unsupported cache input scalar: {source!r}')
    return next(value for value in match.groups() if value is not None)


def cache_inputs(text):
    # Read the selected literal mapping; unsupported shapes fail closed.
    starts = list(re.finditer(r'(?m)^      -[ \t]+', text))
    matches = []
    for index, start in enumerate(starts):
        end = starts[index + 1].start() if index + 1 < len(starts) else len(text)
        block = text[start.end():end]
        uses = re.findall(r'(?m)^(?:        )?uses:[ \t]*(.*)$', block)
        if not uses or literal_scalar(uses[0]) != CACHE_ACTION:
            continue
        if len(uses) != 1:
            raise AssertionError('cache step has duplicate uses fields')
        header = re.search(r'(?m)^        with:[ \t]*(?:#.*)?$', block)
        if header is None:
            raise AssertionError('cache inputs must be a literal mapping')
        fields = {}
        for line in block[header.end():].splitlines():
            if not line.strip() or line.lstrip().startswith('#'):
                continue
            indent = len(line) - len(line.lstrip(' '))
            if indent <= 8:
                break
            field = re.fullmatch(r'          ([A-Za-z_][A-Za-z0-9_-]*):[ \t]*(.*)', line)
            if field is None or field[1] in fields:
                raise AssertionError(f'unsupported or duplicate cache input: {line!r}')
            fields[field[1]] = literal_scalar(field[2])
        matches.append(fields)
    if len(matches) != 1:
        raise AssertionError(f'expected one selected cache step, found {len(matches)}')
    return matches[0]


def environment_pairs(lines, indent):
    """Literal environment keys at one mapping level; continuation lines are not new variables."""
    pairs = {}
    for line in lines:
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        if len(line) - len(line.lstrip(' ')) > indent:
            continue
        field = re.fullmatch(r' {' + str(indent) + r'}([A-Za-z_][A-Za-z0-9_]*):[ \t]*(.*?)[ \t]*', line)
        if field is None or field[1] in pairs:
            raise AssertionError(f'unsupported or duplicate environment key: {line!r}')
        pairs[field[1]] = field[2]
    return pairs


def workflow_environment(workflow=None):
    """Read through comment/blank lines, stopping at the next column-zero YAML key."""
    text = WORKFLOW if workflow is None else workflow
    block = re.search(r'^env:\n((?:(?:[ ].*|#.*)?\n)+)', text, re.M)
    if block is None:
        raise AssertionError('no workflow-level env block')
    return environment_pairs(block.group(1).splitlines(), 2)


def job_environment(job_text):
    """The job's `env:` block, as KEY: value pairs (comments and blank lines skipped)."""
    lines = job_text.splitlines()
    try:
        start = lines.index('    env:')
    except ValueError:
        return {}
    block = []
    for line in lines[start + 1:]:
        if not line.strip() or line.lstrip().startswith('#'):
            continue
        if len(line) - len(line.lstrip(' ')) <= 4:
            break
        block.append(line)
    return environment_pairs(block, 6)


def hashed(environment):
    """The variables rust-cache hashes, exactly as src/config.ts filters them."""
    matched = {name: value for name, value in environment.items()
               if name.startswith(PREFIXES) and value}
    for name, value in matched.items():
        if '${{' in value:
            raise ValueError(f'unresolved cache environment expression: {name}')
    return matched


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
        self.cache = cache_inputs(self.text)

    def test_restores_the_gate_entry_through_the_shared_action(self):
        self.assertEqual(self.cache['job'], 'gate')
        self.assertEqual(self.cache['align-environment'], 'true')

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
        self.assertEqual(self.cache['save-if'], 'false')

    def test_cache_inputs_are_parsed_not_matched_in_comments(self):
        changed = self.text.replace('save-if: "false"', 'save-if: "true" # save-if: "false"', 1)
        self.assertEqual(cache_inputs(changed)['save-if'], 'true')

    def test_cache_input_mapping_rejects_unsupported_or_duplicate_fields(self):
        for changed in (
            self.text.replace('        with:\n          cache-version:', '        with: |\n          cache-version:', 1),
            self.text.replace('          job: gate', '          job: gate\n          job: other', 1),
        ):
            with self.subTest(workflow=changed):
                with self.assertRaises(AssertionError):
                    cache_inputs(changed)

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


class EnvironmentReaderTests(unittest.TestCase):
    def test_workflow_comments_and_blanks_do_not_hide_later_keys(self):
        text = ('env:\n  CARGO_TERM_COLOR: always\n  # keep reading\n\n'
                '# column-zero comments are not YAML keys\n  EMPTY:\n  RUSTFLAGS: -Dwarnings\n'
                'jobs:\n  ignored: true\n')
        self.assertEqual(workflow_environment(text),
                         {'CARGO_TERM_COLOR': 'always', 'EMPTY': '', 'RUSTFLAGS': '-Dwarnings'})

    def test_job_comments_and_blanks_do_not_hide_later_keys(self):
        text = ('    env:\n      CARGO_INCREMENTAL: 0\n    # still inside the mapping\n\n'
                '      EMPTY:\n      RUSTFLAGS: -Dwarnings\n    steps:\n      - run: true\n')
        self.assertEqual(job_environment(text),
                         {'CARGO_INCREMENTAL': '0', 'EMPTY': '', 'RUSTFLAGS': '-Dwarnings'})

    def test_unsupported_or_duplicate_environment_keys_fail_closed(self):
        for body in ("  'RUSTFLAGS': -Dwarnings\n", '  RUSTFLAGS: a\n  RUSTFLAGS: b\n'):
            with self.subTest(body=body):
                with self.assertRaises(AssertionError):
                    workflow_environment('env:\n' + body + 'jobs: {}\n')

    def test_runtime_expression_after_a_comment_is_not_dropped(self):
        text = 'env:\n  PHOTOCRAFT_NO_PRINT: "1"\n  # cache-sensitive below\n\n  RUSTFLAGS: ${{ vars.EXTRA }}\njobs: {}\n'
        with self.assertRaisesRegex(ValueError, 'unresolved'):
            hashed(workflow_environment(text))


class KeyTests(unittest.TestCase):
    def test_unresolved_matching_environment_is_rejected(self):
        for prefix in PREFIXES:
            for value in ('${{ vars.EXTRA }}', 'value-${{ vars.EXTRA }}'):
                with self.subTest(prefix=prefix, value=value):
                    with self.assertRaisesRegex(ValueError, 'unresolved'):
                        hashed({prefix + '_EXTRA': value})
        self.assertEqual(hashed({'PHOTOCRAFT_FULL_SWEEP': '${{ inputs.full_sweep }}'}), {})

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
