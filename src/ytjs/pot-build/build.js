// Rebuild ../pot.js:  npm install && npm run build
import { readFileSync, writeFileSync, rmSync, readdirSync } from "node:fs";

// Fallback when esbuild-plugin-license finds no LICENSE file (e.g. LICENCE.md, or none shipped).
function licenseOf(d) {
  if (d.licenseText) return d.licenseText;
  const dir = "node_modules/" + d.packageJson.name;
  const f = readdirSync(dir).find((n) => /^licen[cs]e/i.test(n));
  if (f) return readFileSync(dir + "/" + f, "utf8");
  const a = d.packageJson.author;
  const who = typeof a === "string" ? a : a?.name;
  if (d.packageJson.license === "ISC")
    return `ISC License\n\nCopyright (c) ${who}\n\nPermission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted, provided that the above copyright notice and this permission notice appear in all copies.\n\nTHE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.\n(license file not shipped in the npm package; standard ISC text per package.json)`;
  return "(no license file shipped; license: " + d.packageJson.license + ")";
}
import * as esbuild from "esbuild";
import esbuildPluginLicense from "esbuild-plugin-license";

const OUT = "../pot.js";
const NOTICE = "../THIRD_PARTY.txt";
const cfg = { entryPoints: ["entry.js"], bundle: true, minify: true, legalComments: "none", format: "iife", target: "es2022" };

const tmpDeps = "deps.tmp.txt";
await esbuild.build({
  ...cfg,
  outfile: "/dev/null",
  plugins: [esbuildPluginLicense({
    thirdParty: {
      output: {
        file: tmpDeps,
        template: (deps) => deps.map((d) =>
          `${"=".repeat(72)}\n${d.packageJson.name} ${d.packageJson.version} -- ${d.packageJson.license}\n${"=".repeat(72)}\n${licenseOf(d).trim()}\n`).join("\n"),
      },
    },
  })],
});
const deps = readFileSync(tmpDeps, "utf8");
rmSync(tmpDeps);
writeFileSync(NOTICE,
  "Third-party software bundled into src/ytjs/pot.js\n\n" +
  "pot.js is built from src/ytjs/pot-build/entry.js, a JavaScript port of the token flow of\n" +
  "rustypipe-botguard (https://codeberg.org/ThetaDev/rustypipe-botguard), MIT License,\n" +
  "Copyright (c) ThetaDev.\n\n" + deps);

await esbuild.build({ ...cfg, outfile: OUT,
  banner: { js: "/*! Vynz pot.js - YouTube PO token minter for deno. Bundles bgutils-js (MIT), @thetadevcode/jsdom-minimal (MIT) and their dependencies (MIT, ISC, BSD-2-Clause, Apache-2.0, MIT-0); full notices in THIRD_PARTY.txt. Flow ported from rustypipe-botguard (MIT). */" } });
console.log("built", OUT, readFileSync(OUT).length, "bytes");
