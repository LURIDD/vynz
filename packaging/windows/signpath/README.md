# SignPath setup

Windows binaries are signed by [SignPath Foundation](https://signpath.org) from
the GitHub Actions workflow (see [CODE_SIGNING_POLICY.md](../../../CODE_SIGNING_POLICY.md)).

SignPath project settings expected by `.github/workflows/build.yml`:

| Setting | Value |
| --- | --- |
| Project slug | `vynz` |
| Signing policy slug | `release-signing` (manual approval, GitHub-hosted runners only) |
| Artifact configuration `exe` | [`exe.xml`](exe.xml) |
| Artifact configuration `setup` | [`setup.xml`](setup.xml) |
| Repository variable | `SIGNPATH_ORGANIZATION_ID` |
| Repository secret | `SIGNPATH_API_TOKEN` (CI user with submitter role) |

Signing only runs for `v*` tags and only when `SIGNPATH_ORGANIZATION_ID` is set,
so builds keep working unsigned until the project is approved.
