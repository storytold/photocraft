# Windows release-ref nightly: source preparation

**Not activated or qualified.** No production signing policy is installed. The producer and
signer workflows are dormant `.yml` registrations, not an activated signing pipeline.
All three jobs remain literally disabled. There is no
schedule or platform secret grant. The unsigned reusable caller's maximum OIDC/attestation
permissions are for its isolated attester; its compiler narrows to contents and Actions read
only, with no OIDC, attestation write, release environment or platform signing secrets.
Shared tooling is pinned to public preview `646d4c0619df19a6e76cb4d669f878e4a1e005d7`
([shared draft #330](https://github.com/storytold/craft-common-actions/pull/330)); this is
reviewed source availability, not provider, queue, native signing or crypto qualification.
Existing stable workflows are unchanged.
This is not a hosted no-key test, a native signing result or a working nightly feed.

## Ownership and interface

- `.github/workflows/nightly.yml`: dormant default-main registration for the future unsigned producer,
  named `PhotoCraft unsigned nightly`; no signing secrets may be passed or inherited.
- `.github/workflows/nightly-dispatch.yml`: dormant completion transport only. On activation it accepts
  only a successful same-repository/main first scheduled producer attempt, then sends only
  `producer_run_id` and `producer_run_attempt` to the fixed `nightly-sign.yml` on `release`.
  No checkout, artifact parsing, application code or release environment is used here.
  Its filters are not authorization: a writer can edit main transport.
- `.github/workflows/nightly-sign.yml`: dormant top-level registration, intended to be identical on
  main/release after reviewed promotion. It rejects wrong repository/event/actual branch,
  including same-name tags. Its constant whole-run concurrency is distinct from stable,
  cancellation is off and `queue: max` must be hosted-qualified. No credentials can be reached
  by this preparation.
- Shared reusable entrypoints are `nightly-unsigned.yml` and `nightly-release.yml` in
  `storytold/craft-common-actions`. Both must resolve to a **real reviewed public immutable
  commit**; the defining workflow SHA and tooling input must match. The release callee's
  request inputs are consistently `producer_run_id` and `producer_run_attempt`, while
  `tooling-revision` keeps its existing hyphen spelling. Confirm the final landed interface
  before any future pin update or activation. Unsigned inputs are `tooling-revision`, `app-name`,
  `manual-actors` (default `[]`) and `craft-fonts-ref`; PhotoCraft's title is reviewed source,
  not an `app-title` input. The reusable caller grants maximum permissions only; each
  callee job must narrow them independently.
  Shared code owns attestation validation, candidate quality, archive bounds, native signing,
  role-separated receipts and publication. App code must not duplicate those parsers.
- The existing `packaging/windows/sign.ps1 -Required` is the app-owned Windows adapter.
  Invoke it only from the immutable **release execution SHA** checkout, never the producer's
  main source or extracted archive. Shared Windows tooling must own the flat GUI-only ZIP,
  PE/inventory checks and post-sign verification; do not execute `nightly.ps1`, `package.ps1`
  or the payload beside keys. No new wrapper is needed. `sign.ps1` is unchanged here.

## Missing real configuration (all activation blockers)

`.github/nightly-signing-policy.json` is deliberately absent: there is no authorizing root,
empty debt set, invented workflow ID or guessed publisher subject. The adjacent
[`nightly-signing-policy.example.json`](nightly-signing-policy.example.json) is intentionally
invalid (`null` unresolved values), not installable configuration. Install only independently
read, reviewed real values under schema `craft-nightly-release-policy/v1`:

| Field | Required source of authority |
|---|---|
| `repository`, `repository_id` | Exact PhotoCraft repository and authenticated numeric ID |
| `producer_workflow_id`, `producer_workflow_path` | Real default-branch registration ID and `.github/workflows/nightly.yml` |
| `producer_tooling_revision`, `producer_workflow_ref` | Approved shared unsigned producer/attester commit and exact reusable path at that commit |
| `signer_workflow_path`, `signer_tooling_revision` | `.github/workflows/nightly-sign.yml` and reviewed public shared signer commit |
| `release_branch`, `platform` | `release`, `windows-x86_64` only |
| `manual_actors` | Empty by default; any authority needs normal release review |
| `max_age_seconds`, `max_artifact_bytes`, `unsigned_artifact_name` | Reviewed bounded age/size and fixed unsigned artifact kind/prefix; actual upload/discovery name is that prefix + `-` + authenticated producer run ID + `-` + authenticated producer attempt, never a caller pattern or substitution |
| `candidate_policy` | Full reviewed `craft-nightly-candidate-policy/v3` with real required workflow/job/check/app IDs, Sonar organization/project/issue types/conditions and exact approved debt fingerprints; provider configuration is trusted as resolved, not a pinned scan inventory |
| `windows_publisher_subject` | Exact independently reviewed protected policy string used by both signer and fresh credentialless Windows publisher; nonempty, at most 2048 characters, no CR/LF/NUL; verified native subject must match exactly, never an artifact/caller input |

Every `workflow_dispatch`, including the signer's first attempt, requires explicit reviewed
`manual_actors` authority. Before future automatic dispatch is activated, its bot must be
reviewed and listed explicitly; there is no implicit bot trust. This preparation does not
populate or authorize that policy.

### Post-publication recovery limitation

At the pinned `646d4c0619df19a6e76cb4d669f878e4a1e005d7` baseline, a failure after the
release becomes public (receipt write, ownership upload, runner loss or timeout) leaves the
recorded attempt unsuccessful; every later nightly refuses until a release manager handles
reviewed manual reconciliation. Re-running does not repair the failed recorded attempt.
Authenticated reconciliation must be separately designed and qualified **before activation**;
no such update is pinned or promised here. Public-state editing/deletion is not newly
authorized by this procedure.

Confirm the shared parser's final exact fields; unknown/missing/null values must refuse.
Do not derive policy, code/tool paths, trust anchors or ignored debt from a candidate archive.
The reviewed Windows publisher subject and independently pinned signing tools are also
unresolved; configure them only through the reviewed shared/release contract, not a guessed
app value or an override from main. The unchanged stable `release.yml` forwards Windows/Azure
secret names from caller scope, while the nightly signer forwards none. Before activation,
verify the existing `release` environment holds the required names and that repository or
organization scopes do not expose signing copies to main. Environment-only missing values
must fail closed; repository/org-only values or same-name scope collisions are blockers,
not permission to copy keys or change settings. No secret names/scopes or protection settings
were audited by this source change; no secret values were read.

## Bounded integration and acceptance

1. The callers now reference the same reviewed public preview commit in `uses` and
   `tooling-revision`. Keep callers/dispatcher literally off through independent qualification
   and while assembling real policy. Future shared pin updates require review of the exact
   public immutable source and interface. Do not claim dormant source as a working pipeline.
   Do not temporarily call old same-run signing or use mutable `@main`/`@release` tooling.
   Keep compiler permissions contents and Actions read only, with no OIDC, attestation write,
   release environment or platform secrets. Only the isolated unsigned attester receives
   its required Actions-read, attestation-write and OIDC, never platform keys. Re-run the
   no-cache static check over **all** trusted jobs/callees, including setup-action cache options.
2. Register on main through normal review. Promote identical signer/policy/adapter contents
   only during an already-intended stable sync, following [the release trust-closure review](releasing.md#cutting-a-release).
   A release push can build/update a stable draft or refuse an already-published version;
   this source change authorizes no push, skip-CI workaround or dummy version bump.
3. Hosted **no-platform-key**, separately authorized tests must establish actual release
   dispatch context/permissions, queue retention/displacement, re-runs/current actor denial,
   exact attestation `invocationId` run/attempt and immutable reusable identity. Test wrong
   repository/ref/tag/event/actor, substituted run/attempt/artifact/transport ZIP/inner package,
   missing/expired artifacts and all digest/claim mismatches. Transport archive SHA, unsigned
   package SHA and final signed SHA are different roles, never interchangeable.
4. Candidate-v3 quality must prove exact C required latest runs/jobs and take the Sonar
   snapshot first. Require coherent latest main-C revision/SCM at collection boundaries,
   complete paginated branch issues scoped to reviewed issue types with count/end reread,
   and the same analysis's gate and exact reviewed debt fingerprints. Profiles, rules,
   scanner configuration and warnings are **trusted as resolved by the provider**, like
   main CI; configuration changes can hide issues. This is not immutable scan-inventory
   proof, full scan-scope enforcement or a warning-free-scan guarantee. The example has no
   `sonar.scope` field; its exact Sonar fields match the shared candidate-v3 parser.

   Debt may shrink only with analysis-driven FIXED lifecycle proof **and** independently
   qualified exact-C source-hash absence; missing/manual/REMOVED/unchanged-source proofs
   refuse. Cloud source-hash normalization remains unqualified, so a vanished approved debt
   item currently **refuses**, rather than counting absence as a fix. Qualify the real hash
   semantics and lifecycle APIs without new credentials before claiming that path works.

   Current main M may advance past C only by verified ancestry; C must strictly advance the
   previous authenticated publication P. Retained-snapshot revalidation currently refuses
   once the latest Sonar analysis advances past C: no qualified historical issue/proof API
   resolves that race. Do not promise publication survives a newer analysis. Repeat
   admission/pause/quality/ancestry/newest-run checks before keys and publication.
5. Separately authorize first native Windows signing/publication, signature/publisher/timestamp
   checks, Launcher install beside stable, launch/profile/version independence, rollback and
   uninstall preservation, and corrupt/unsigned/wrong-publisher rejection. No Linux/macOS
   coverage follows from Windows. Keep production/native/publication/prune switches off until
   their own acceptance. Nightly assets remain prerelease, `make_latest:false`, with separate
   nightly tags/reservations. Authenticated interrupted-publication reconciliation remains
   the [preactivation blocker described above](#post-publication-recovery-limitation), not a
   capability supplied by this pin.

Routine updates ride real stable windows, never automatic main-to-release copying. Pause or
cancel via existing nightly workflow controls without pushing release; urgent repairs need
one explicitly authorized bounded maintenance push. See the shared-identity incident note in
[the release guide](releasing.md#release-ref-nightly-preparation). No daily witness renewal or
per-run human approval is introduced by candidate-v3; provider gaps remain blockers.

## Local regression checks

`python3 packaging/nightly-release-tests.py` executes the **actual inline dispatcher** with
synthetic, locally generated event fixtures and a mocked HTTPS opener. It verifies fixed
routing, refusal before contact and no retry; it does not contact GitHub or prove admission,
cryptography, native signing, hosted concurrency, or the shared candidate-v3 implementation.
The same suite covers all three callers for the no-cache/no-platform-secret boundary,
requires every nightly job to stay explicitly off (a top-level OR cannot override false),
and refuses any registered nightly schedule. It also rejects changed root serialization: the fixed PhotoCraft nightly group, `queue: max`,
and `cancel-in-progress: false` must remain literal. Mutation regressions reject invalid queue
values, cancellation and request-dependent groups. Packaging lint runs this suite in its
independent contents-read-only `nightly-source-contract` job, including config-only changes;
it does not rely on the shared Bash packaging hook to run the guard.

The existing `.github/actionlint.yaml` suppresses only actionlint 1.7.12's anchored exact
unsupported-queue message for `.github/workflows/nightly-sign.yml`. This operator-requested
compatibility exception is not a blanket syntax waiver: other files/messages still fail,
and the mandatory source guard supplies the missing queue-value/serialization checks.
No tool fork or shared lint change is needed. This same-PR nonprivileged CI hook is the
narrow correction requested for the dormant source; stable/release execution is unchanged.
Run through the normal task allocator; report actionlint/PowerShell/native/hosted checks
separately and never treat a missing runtime as a pass.
