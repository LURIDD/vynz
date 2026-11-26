// Vynz PO token minter (MIT).
// Port of the rustypipe-botguard flow (https://codeberg.org/ThetaDev/rustypipe-botguard, MIT,
// Copyright ThetaDev) to plain JavaScript for deno. Uses bgutils-js (MIT, LuanRT) and
// @thetadevcode/jsdom-minimal (MIT). See ../THIRD_PARTY.txt.
//
// Defines globalThis.vynzPot = { mint(videoId, visitorData, userAgent) -> {token, validUntil} }.
import { BG } from "bgutils-js";
import { JSDOM } from "@thetadevcode/jsdom-minimal";

// Botguard must see a WebKit UA, other UAs give invalid tokens (rustypipe-botguard note).
const DEFAULT_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.10 Safari/605.1.1";
const GOOG_API_KEY = "AIzaSyDyT5W0Jh49F30Pqqtyfdf7pDLFKLJoAnw";
const REQUEST_KEY = "O43z0dpjhgX20SCx4KAo";
const JNN = "https://www.youtube.com/api/jnn/v1/";
// GenerateIT randomly hands out integrity tokens of 63..68 bytes. Only the short ones
// (<= 64 bytes, giving 88/89-byte PO tokens) are accepted by googlevideo past the first ~1 MiB
// of a stream; longer ones give HTTP 403 (verified on daMS-E6Fbis, 2026-11-26; the
// rustypipe-botguard binary has the same behaviour on fresh inits). So re-run Botguard until
// a short integrity token comes back.
const MAX_GOOD_IT_LEN = 64;
const MAX_ATTEMPTS = 12;
// Re-create the minter this many seconds before the integrity token expires.
const REFRESH_MARGIN = 600;

let state = null; // { minter, createdAt, lifetime }
let initPromise = null;

function headers() {
  return {
    "content-type": "application/json+protobuf",
    "x-goog-api-key": GOOG_API_KEY,
    "x-user-agent": "grpc-web-javascript/0.1",
    "user-agent": DEFAULT_UA,
  };
}

async function jnn(method, body) {
  const r = await fetch(JNN + method, { method: "POST", headers: headers(), body: JSON.stringify(body) });
  if (!r.ok) throw new Error(`jnn ${method}: HTTP ${r.status}`);
  return await r.json();
}

function descramble(s) {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = (bin.charCodeAt(i) + 97) & 0xff;
  return new TextDecoder().decode(out);
}

function firstStr(a) {
  return Array.isArray(a) ? a.find((x) => typeof x === "string" && x.length > 0) : undefined;
}

function parseChallenge(c) {
  if (!Array.isArray(c) || c.length < 6) throw new Error("challenge: array len < 6");
  const script = firstStr(c[1]);
  const url = script ? undefined : firstStr(c[2]);
  if (!script && !url) throw new Error("challenge: interpreterJs");
  if (typeof c[4] !== "string" || typeof c[5] !== "string") throw new Error("challenge: program/globalName");
  return { script, url, program: c[4], globalName: c[5] };
}

async function getChallenge() {
  const resp = await jnn("Create", [REQUEST_KEY]);
  if (!Array.isArray(resp)) throw new Error("challenge: array expected");
  let ch;
  if (typeof resp[1] === "string") ch = parseChallenge(JSON.parse(descramble(resp[1])));
  else if (Array.isArray(resp[0])) ch = parseChallenge(resp[0]);
  else throw new Error("challenge: invalid format");
  if (!ch.script) {
    let u;
    try { u = new URL("https:" + ch.url); } catch { u = new URL(ch.url); }
    const host = u.hostname.replace(/^www\./, "");
    if (host !== "google.com" && host !== "youtube.com") throw new Error("challenge: invalid domain " + host);
    const r = await fetch(u, { headers: { "user-agent": DEFAULT_UA } });
    if (!r.ok) throw new Error(`interpreter: HTTP ${r.status}`);
    ch.script = await r.text();
  }
  return ch;
}

function installDom(userAgent) {
  const dom = new JSDOM(
    '<!DOCTYPE html><html lang="en"><head><title></title></head><body></body></html>',
    { url: "https://www.youtube.com/", referrer: "https://www.youtube.com/", userAgent },
  );
  const w = dom.window;
  for (const [k, v] of Object.entries({ window: w, document: w.document, location: w.location, origin: w.origin })) {
    try { Object.defineProperty(globalThis, k, { value: v, writable: true, configurable: true }); }
    catch { try { globalThis[k] = v; } catch { /* ignore */ } }
  }
  // Replace the host navigator (deno's says "Deno/x") with the jsdom one.
  try { Object.defineProperty(globalThis, "navigator", { value: w.navigator, configurable: true, writable: true }); }
  catch { /* ignore */ }
}

function b64Decode(s) {
  s = s.replace(/-/g, "+").replace(/_/g, "/").replace(/=+$/, "");
  return Uint8Array.from(atob(s + "==".slice(0, (4 - (s.length % 4)) % 4)), (c) => c.charCodeAt(0));
}

// Port of protobuf::bytes_from_pb(.., 6): token must contain length-delimited field 6.
function validToken(tok) {
  let p;
  try { p = b64Decode(tok); } catch { return false; }
  let i = 0;
  const varint = () => {
    let r = 0, s = 0, n = 0;
    while (i < p.length) { const b = p[i++]; r += (b & 0x7f) * 2 ** s; s += 7; n++; if (!(b & 0x80)) break; }
    return n ? r : null;
  };
  while (i < p.length) {
    const key = varint();
    if (key === null) return false;
    const f = Math.floor(key / 8), w = key & 7;
    if (w === 0) varint();
    else if (w === 1) i += 8;
    else if (w === 5) i += 4;
    else if (w === 2) { const len = varint(); if (len === null) return false; if (f === 6) return i + len <= p.length; i += len; }
    else return false;
  }
  return false;
}

async function initOnce() {
  installDom(DEFAULT_UA);
  const createdAt = Math.floor(Date.now() / 1000);
  const ch = await getChallenge();
  new Function(ch.script)();
  const bg = await BG.BotGuardClient.create({ program: ch.program, globalName: ch.globalName, globalObj: globalThis });
  const webPoSignalOutput = [];
  const bgResponse = await bg.snapshot({ webPoSignalOutput });
  const resp = await jnn("GenerateIT", [REQUEST_KEY, bgResponse]);
  if (!Array.isArray(resp) || typeof resp[0] !== "string") throw new Error("GenerateIT: no integrity token");
  if (typeof resp[1] !== "number") throw new Error("GenerateIT: no lifetime");
  const itLen = b64Decode(resp[0]).length;
  const minter = await BG.WebPoMinter.create({ integrityToken: resp[0] }, webPoSignalOutput);
  const test = await minter.mintAsWebsafeString("Cgs4bFZSaUotYTYtQSiJnvu8BjIKCgJERRIEEgAgFw==");
  if (!validToken(test)) throw new Error("minter self-test failed");
  return { minter, createdAt, lifetime: resp[1], itLen };
}

async function init() {
  let err, best = null, failures = 0;
  for (let i = 0; i < MAX_ATTEMPTS; i++) {
    let s;
    try { s = await initOnce(); } catch (e) { err = e; if (++failures >= 3) break; continue; }
    s.attempts = i + 1;
    if (s.itLen <= MAX_GOOD_IT_LEN) return s;
    if (!best || s.itLen < best.itLen) best = s;
  }
  if (best) { best.degraded = true; return best; } // likely rejected by googlevideo
  throw err;
}

async function getState() {
  const now = Math.floor(Date.now() / 1000);
  if (state && state.createdAt + state.lifetime - REFRESH_MARGIN > now) return state;
  if (!initPromise) {
    initPromise = init().then((s) => (state = s), (e) => { throw e; })
      .finally(() => { initPromise = null; });
  }
  return await initPromise;
}

globalThis.vynzPot = {
  /**
   * Mint a GVS PO token bound to videoId.
   * visitorData and userAgent are accepted for API stability; Botguard always runs with a
   * WebKit UA (other UAs produce invalid tokens) and the token is bound to the video id.
   * @returns {Promise<{token: string, validUntil: number}>}
   */
  async mint(videoId, visitorData, userAgent) {
    if (typeof videoId !== "string" || !videoId) throw new Error("videoId required");
    const s = await getState();
    let ident = videoId;
    try { ident = decodeURIComponent(videoId); } catch { /* keep */ }
    const token = await s.minter.mintAsWebsafeString(ident);
    if (!validToken(token)) throw new Error("invalid po token");
    return { token, validUntil: s.createdAt + s.lifetime };
  },
  /** Drop the cached minter (next mint re-runs Botguard). */
  reset() { state = null; },
  /** Debug info about the cached minter. */
  info() { return state ? { validUntil: state.createdAt + state.lifetime, attempts: state.attempts, integrityTokenBytes: state.itLen, degraded: !!state.degraded } : null; },
  /** Unix seconds when the cached minter expires, or 0. */
  validUntil() { return state ? state.createdAt + state.lifetime : 0; },
};
