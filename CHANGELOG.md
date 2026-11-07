# PhotoCraft changelog

Published release notes are the record of changes shipped in each version. They include the
merged pull requests and contributors; unreleased pull requests are not evidence that a fix
has shipped.

- [Latest release and notes](https://github.com/storytold/photocraft/releases/latest)
- [All releases](https://github.com/storytold/photocraft/releases)
- [Changes on main since the latest tag](https://github.com/storytold/photocraft/compare/v0.9.0...main)

## Published versions

| Version | Published | Release notes |
|---|---|---|
| 0.9.0 | 2026-10-14 | [PhotoCraft v0.9.0](https://github.com/storytold/photocraft/releases/tag/v0.9.0) |
| 0.8.2 | 2026-10-12 | [PhotoCraft v0.8.2](https://github.com/storytold/photocraft/releases/tag/v0.8.2) |
| 0.7.0 | 2026-10-11 | [PhotoCraft v0.7.0](https://github.com/storytold/photocraft/releases/tag/v0.7.0) |

Earlier versions remain available in [all releases](https://github.com/storytold/photocraft/releases).

## Maintaining release notes

The release workflow already generates notes from merged pull requests when it creates a
draft release. [`.github/release.yml`](.github/release.yml) groups those notes using the
repository's existing labels and includes unlabeled changes in Other changes.

When publishing a release, add its version and publication date to the table and update the
comparison link above. Review the generated notes and add the measured scorecard changes as
described in [Releasing PhotoCraft](docs/releasing.md). Never list an open PR as a shipped fix.
