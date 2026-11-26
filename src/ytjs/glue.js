// Glue between the EJS IIFE bundles (same shims as yt-dlp/ejs PR #86 QuickJS harness).
function require(name) {
  const result = (lib?.lib ?? lib)[name];
  if (!result) throw `Missing library: ${name}`;
  return result;
}
if (typeof structuredClone === "undefined") {
  globalThis.structuredClone = (x) => JSON.parse(JSON.stringify(x));
}
