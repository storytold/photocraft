# Runtime redistribution gate: BLOCKED

This is a validation-only draft, not a redistributable release. **No human legal review of the exact Qualcomm runtime terms has been completed.** This document is not legal advice or an approval.

The top-level MIT description of `Microsoft.ML.OnnxRuntime.QNN` must not be applied indiscriminately to Qualcomm binaries, DSP skeletons, signing catalogues or their accompanying PDF. Package signature/digest validation proves identity/integrity, not a redistribution right. Asset presence likewise does not grant a licence.

## Required review before any distribution change

A maintainer/legal reviewer must inspect the exact restored, digest-bound assets, including:

- QNN `LICENSE`, `Privacy.md`, `Qualcomm_LICENSE.pdf`, `ThirdPartyNotices.txt`.
- Managed ORT `LICENSE.txt`, `Privacy.md`, `ThirdPartyNotices.txt`.
- .NET self-contained runtime/host and transitive dependency licence/notice files.
- All app-local Qualcomm DLLs, V73/V81 stubs, DSP `.so` skeletons and `.cat` files; CRT/driver and target-device requirements.
- The intended distribution channel/scope, any Qualcomm agreement or flow-down obligations, privacy terms, export restrictions, attribution requirements and provenance. Models, conversion tools and calibration data need separate review and are not included here.

The future approval record must identify the reviewer/authority, review date, package version, exact archive and legal-file digests, intended distribution scope and conditions. A workflow flag, environment variable, empty approval file, signature check or green test is **not** such a record.

## Enforced now

The packaging script makes a complete ephemeral ARM64 publish and inventories native/DSP/notice assets. It **has no ZIP creation or approval-override option**. The workflow uploads only explicitly named JSON/log evidence; it never uploads the publish directory, NuGet archives, caches, DLLs, EXEs, SOs, CATs or PDF payload. Do not add a wildcard artifact path that captures those directories.

After actual review, distribution must be enabled by a separate reviewed source change with exact legal digests/scope and complete retained notices. The existing host package caps are still mandatory. Do not strip .NET files or native/legal assets merely to meet caps.
