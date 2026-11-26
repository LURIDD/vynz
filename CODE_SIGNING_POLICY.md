# Code signing policy

Windows release binaries of Vynz (`vynz.exe` and `vynz-windows-x86_64-setup.exe`)
are signed so that users can verify that they were built from this repository
and have not been modified.

Free code signing provided by [SignPath.io](https://about.signpath.io),
certificate by [SignPath Foundation](https://signpath.org).

> Status: the application to the SignPath Foundation is pending. Releases up to
> and including v0.3.3 are **not** signed. Use the `SHA256SUMS` file attached to
> each release to verify downloads.

## How releases are built and signed

- Every release is built from a tagged commit on `main` by the
  [GitHub Actions workflow](.github/workflows/build.yml), on GitHub-hosted
  runners only. No binaries are built or signed on a personal machine.
- The workflow submits `vynz.exe` to SignPath, then packages the signed
  executable into the portable zip and the Inno Setup installer, and submits the
  installer for signing as well.
- Every signing request is approved manually by an approver (see below) before
  a release is published.
- Only binaries built from this repository are signed. Third-party components
  shipped next to them, such as `libmpv-2.dll` from
  [mpv-winbuild-cmake](https://github.com/shinchiro/mpv-winbuild-cmake), are
  distributed unchanged and are not signed by this project.

## Team roles

Vynz is maintained by a single person, who holds every role:

| Role | Members |
| --- | --- |
| Committers and reviewers | [LURIDD](https://github.com/LURIDD) |
| Approvers | [LURIDD](https://github.com/LURIDD) |

Pull requests from other contributors are reviewed by a committer before they
are merged. Multi-factor authentication is enabled on the GitHub and SignPath
accounts of every team member.

## Privacy policy

Vynz has no telemetry, analytics, advertising or user accounts of its own. It
does not send any personal data to the developer. Listening history, likes,
playlists and settings are stored only on the user's computer.

To do its job, the program connects to the following services. Each connection
is made only to fulfil something the user does in the program (searching,
playing, opening lyrics, importing a playlist) or to keep the program working:

| Service | Purpose | When |
| --- | --- | --- |
| YouTube / YouTube Music (Google) | search, browsing, audio streams, cover images | when the user searches, browses or plays music |
| SoundCloud | search and audio streams | when the user searches or plays SoundCloud tracks |
| Spotify (public playlist pages) | reading track lists to import | only when the user imports a Spotify playlist |
| [LRCLIB](https://lrclib.net), [Unison](https://unison.betterlyrics.org) | synced lyrics | when lyrics are shown for a song |
| Google Translate | lyric translation | only if the user turns translation on |
| GitHub | checking for, downloading and verifying (SHA-256) Vynz updates | at startup; a version can be skipped from the update window |
| GitHub, PyPI | first-run setup of the resolver environment (`uv`, Python, `yt-dlp`, `ytmusicapi`, `deno`) and keeping `yt-dlp` up to date | on first run and every few days |
| Discord (local only) | showing the current song as Discord status | only while the Discord desktop app is running; can be turned off in Settings; talks to the local Discord client over IPC, not to the internet |

Requests to these services are subject to their own privacy policies:
[Google](https://policies.google.com/privacy),
[SoundCloud](https://soundcloud.com/pages/privacy),
[Spotify](https://www.spotify.com/legal/privacy-policy/),
[GitHub](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement),
[PyPI](https://policies.python.org/pypi.org/Privacy-Notice/),
[Discord](https://discord.com/privacy).

Apart from the cases listed above, this program will not transfer any
information to other networked systems unless specifically requested by the
user or the person installing or operating it.

## Installation and removal

The installer installs Vynz for the current user only
(`%LOCALAPPDATA%\Programs\Vynz`) and does not require administrator rights or
change system settings. Vynz can be removed from Windows "Installed apps"; the
portable zip can simply be deleted. User data lives in `%LOCALAPPDATA%\vynz` and
`%APPDATA%\vynz`.

## Reporting

If you believe a signed Vynz binary is malicious or was not built from this
repository, please open an issue or contact SignPath at
[support@signpath.io](mailto:support@signpath.io).
