// Vynz YouTube JS helper, run under deno by src/ytweb.rs.
//
// Protocol: one JSON request per stdin line, one JSON reply per stdout line.
//   {"id":1,"op":"solve","player":"<player id>","n":["..."],"sig":["..."]}
//     -> {"id":1,"ok":true,"sts":20779,"n":{"in":"out"},"sig":{"in":"out"}}
//   {"id":2,"op":"pot","video":"<video id>","visitor":"<visitorData>","ua":"..."}
//     -> {"id":2,"ok":true,"token":"...","valid_until":1234}
//   {"id":3,"op":"reset"}  drop the cached minter (its tokens were rejected)
//   {"id":4,"op":"info"}   minter debug info
// Errors: {"id":N,"ok":false,"error":"..."}
//
// The EJS solver (yt-dlp/ejs, Unlicense) parses the player once per player version;
// its "preprocessed player" output is cached on disk (CACHE_DIR) so later starts only
// evaluate it. Files are concatenated in front of this script by the Rust side:
// ejs lib + glue + ejs core + pot.js + host.js.

const CACHE_DIR = Deno.args[0] ?? ".";

// pot.js installs a jsdom window (window, document, location, navigator...) on the global
// object when it mints. EJS must not see those, so solver work runs against a snapshot of
// the pristine globals taken here, before any mint. Solver code is synchronous, so no other
// request can interleave while the globals are swapped.
const PRISTINE = new Map(
  Reflect.ownKeys(globalThis).map((k) => [k, Reflect.getOwnPropertyDescriptor(globalThis, k)]),
);
function pristine(fn) {
  const saved = new Map();
  for (const k of Reflect.ownKeys(globalThis)) {
    const now = Reflect.getOwnPropertyDescriptor(globalThis, k);
    const orig = PRISTINE.get(k);
    const same = orig && now && orig.value === now.value && orig.get === now.get;
    if (same || !now.configurable) continue;
    saved.set(k, now);
    delete globalThis[k];
    if (orig) Reflect.defineProperty(globalThis, k, orig);
  }
  try {
    return fn();
  } finally {
    for (const [k, d] of saved) {
      delete globalThis[k];
      Reflect.defineProperty(globalThis, k, d);
    }
  }
}
const solvers = new Map(); // player id -> {n, sig, sts}

function playerUrl(id) {
  return `https://www.youtube.com/s/player/${id}/player_ias.vflset/en_US/base.js`;
}

function cachePath(id) {
  return `${CACHE_DIR}/ejs-${id}.js`;
}

function loadSolver(id, pre, sts) {
  const r = { n: null, sig: null };
  pristine(() => Function("_result", pre)(r));
  if (typeof r.n !== "function" || typeof r.sig !== "function") {
    throw new Error("EJS: solver functions missing");
  }
  const s = { n: r.n, sig: r.sig, sts };
  solvers.set(id, s);
  return s;
}

async function solver(id) {
  const hit = solvers.get(id);
  if (hit) return hit;
  if (!/^[a-zA-Z0-9_-]{4,40}$/.test(id)) throw new Error("bad player id");
  try {
    const cached = JSON.parse(await Deno.readTextFile(cachePath(id)));
    return loadSolver(id, cached.pre, cached.sts);
  } catch (_) {
    // not cached yet
  }
  const resp = await fetch(playerUrl(id));
  if (!resp.ok) throw new Error(`player HTTP ${resp.status}`);
  const player = await resp.text();
  const m = player.match(/signatureTimestamp[=:](\d+)/);
  const sts = m ? Number(m[1]) : 0;
  const out = pristine(() => jsc.default({
    type: "player",
    player,
    requests: [],
    output_preprocessed: true,
  }));
  if (out.type !== "result" || !out.preprocessed_player) {
    throw new Error(`EJS: ${out.error ?? "no preprocessed player"}`);
  }
  try {
    await Deno.writeTextFile(cachePath(id), JSON.stringify({ sts, pre: out.preprocessed_player }));
  } catch (_) {
    // cache dir not writable: keep in memory only
  }
  return loadSolver(id, out.preprocessed_player, sts);
}

async function handle(req) {
  if (req.op === "solve") {
    const s = await solver(req.player);
    const n = {};
    const sig = {};
    pristine(() => {
      for (const c of req.n ?? []) n[c] = String(s.n(c));
      for (const c of req.sig ?? []) sig[c] = String(s.sig(c));
    });
    return { ok: true, sts: s.sts, n, sig };
  }
  if (req.op === "pot") {
    const r = await globalThis.vynzPot.mint(req.video, req.visitor, req.ua);
    return { ok: true, token: r.token, valid_until: r.validUntil };
  }
  if (req.op === "ping") return { ok: true };
  if (req.op === "reset") {
    globalThis.vynzPot.reset();
    return { ok: true };
  }
  if (req.op === "info") return { ok: true, pot: globalThis.vynzPot.info?.() ?? null };
  throw new Error(`unknown op ${req.op}`);
}

const enc = new TextEncoder();
function reply(obj) {
  Deno.stdout.writeSync(enc.encode(JSON.stringify(obj) + "\n"));
}

reply({ ready: true });
let buf = "";
const dec = new TextDecoder();
for await (const chunk of Deno.stdin.readable) {
  buf += dec.decode(chunk, { stream: true });
  let i;
  while ((i = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, i).trim();
    buf = buf.slice(i + 1);
    if (!line) continue;
    let req;
    try {
      req = JSON.parse(line);
    } catch (_) {
      continue;
    }
    // Requests run concurrently; replies carry the id.
    handle(req).then(
      (r) => reply({ id: req.id, ...r }),
      (e) => reply({ id: req.id, ok: false, error: String(e?.message ?? e) }),
    );
  }
}
