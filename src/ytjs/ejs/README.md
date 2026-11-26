# Vendored yt-dlp EJS challenge solver

Source: <https://github.com/yt-dlp/ejs>, `main` branch at commit
`c47fb1b5` (2026-11-25, after release 0.8.0; 0.8.0 fails on player `b485e332`
with "a.addOnDisposeCallback is not a function").

Built with `EJS_BUILD_BUNDLER=deno EJS_BUILD_INSTALLER=deno python hatch_build.py`
(deno 2.9.8, esbuild 0.28.2). Files are the unmodified `dist/` outputs.

| File                    | SHA-256                                                            |
| ----------------------- | ------------------------------------------------------------------ |
| `yt.solver.lib.min.js`  | `e6fe4b2737342acc9d89fa5f92d396327f70e4eb440502559dd10698db4eeb92` |
| `yt.solver.core.min.js` | `0d53402acfb7633a0cef0f7721a4c6afcaf535e69b9929b3735cd91d86b4d304` |

Licenses: EJS is released under the Unlicense (`LICENSE`). The lib bundle contains
astring 1.9.0 (MIT) and meriyah 7.4.2 (ISC); their license texts are kept in the
header of `yt.solver.lib.min.js`.

The bundles are glued together the same way as in EJS' QuickJS test harness
(PR yt-dlp/ejs#86): a `require()` shim resolving `meriyah`/`astring` from the lib
bundle, a `structuredClone` shim, and `jsc.default(input)` as entry point.

To update: rebuild from upstream, replace both files, update this table and run
`cargo test --lib deobfuscate::tests::ejs_tests`.
