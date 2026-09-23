// Proof that hub-account only accepts a token Privy actually signed for this app.
//
//   node --test deploy/hub/qa/hub-account.test.mjs
//
// Every case mints a real ES256 token with a locally generated P-256 key, so a passing run means the verifier
// agrees with a real signer, not with a fixture someone wrote to match it.

import test from "node:test";
import assert from "node:assert/strict";
import { generateKeyPairSync, sign as signBytes, createHmac } from "node:crypto";
import { join } from "node:path";
import { verifyAccessToken, createApp } from "../hub-account.mjs";

const { publicKey, privateKey } = generateKeyPairSync("ec", { namedCurve: "P-256" });
const other = generateKeyPairSync("ec", { namedCurve: "P-256" });
const APP_ID = "test-app-id";
const NOW = 1_800_000_000;

const b64 = (buf) => Buffer.from(buf).toString("base64").replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const part = (obj) => b64(JSON.stringify(obj));

function mint({ header = {}, claims = {}, key = privateKey, encoding = "ieee-p1363" } = {}) {
  const h = part({ alg: "ES256", typ: "JWT", ...header });
  const p = part({ iss: "privy.io", aud: APP_ID, sub: "did:privy:abc123", iat: NOW - 10, exp: NOW + 3600, ...claims });
  const sig = signBytes("sha256", Buffer.from(`${h}.${p}`), { key, dsaEncoding: encoding });
  return `${h}.${p}.${b64(sig)}`;
}
const check = (token) => verifyAccessToken(token, { appId: APP_ID, publicKey, now: NOW });

test("a token this app signed is accepted", () => {
  const r = check(mint());
  assert.equal(r.ok, true);
  assert.equal(r.did, "did:privy:abc123");
});

test("a DER-encoded signature is refused", () => {
  // The trap this verifier exists to survive: Node signs DER by default, JOSE is raw r‖s. If the verifier were
  // written without dsaEncoding it would accept this and reject every real Privy token.
  assert.equal(check(mint({ encoding: "der" })).ok, false);
});

test("an expired token is refused", () => {
  assert.equal(check(mint({ claims: { exp: NOW - 120 } })).ok, false);
});

test("a token still inside the clock-skew allowance is accepted", () => {
  assert.equal(check(mint({ claims: { exp: NOW - 30 } })).ok, true);
});

test("a token for another app is refused", () => {
  assert.equal(check(mint({ claims: { aud: "someone-elses-app" } })).ok, false);
});

test("a token from another issuer is refused", () => {
  assert.equal(check(mint({ claims: { iss: "evil.example" } })).ok, false);
});

test("a token signed by a different key is refused", () => {
  assert.equal(check(mint({ key: other.privateKey })).ok, false);
});

test("alg none is refused", () => {
  const h = part({ alg: "none", typ: "JWT" });
  const p = part({ iss: "privy.io", aud: APP_ID, sub: "did:privy:abc123", exp: NOW + 3600 });
  assert.equal(check(`${h}.${p}.`).ok, false);
});

test("an HS256 token signed with the public key as the secret is refused", () => {
  // Algorithm confusion: the public key is public, so if the verifier trusted the header's alg this would pass.
  const h = part({ alg: "HS256", typ: "JWT" });
  const p = part({ iss: "privy.io", aud: APP_ID, sub: "did:privy:abc123", exp: NOW + 3600 });
  const pem = publicKey.export({ type: "spki", format: "pem" });
  const sig = createHmac("sha256", pem).update(`${h}.${p}`).digest();
  assert.equal(check(`${h}.${p}.${b64(sig)}`).ok, false);
});

test("a tampered payload is refused", () => {
  const [h, , s] = mint().split(".");
  const p = part({ iss: "privy.io", aud: APP_ID, sub: "did:privy:someone-else", exp: NOW + 3600 });
  assert.equal(check(`${h}.${p}.${s}`).ok, false);
});

test("a token with no subject is refused", () => {
  assert.equal(check(mint({ claims: { sub: "" } })).ok, false);
});

test("a token issued in the future is refused", () => {
  assert.equal(check(mint({ claims: { iat: NOW + 600 } })).ok, false);
});

test("junk is refused without throwing", () => {
  for (const junk of ["", "x", "a.b", "a.b.c", "....", null, undefined, 42, "a.b.c.d"]) {
    assert.equal(check(junk).ok, false);
  }
});

// ---- the service
async function call(app, { method = "GET", path = "/api/account/me", token, body } = {}) {
  const chunks = body === undefined ? [] : [Buffer.from(JSON.stringify(body))];
  const req = Object.assign(
    (async function* () { yield* chunks; })(),
    { method, url: path, headers: token ? { authorization: `Bearer ${token}` } : {} },
  );
  return new Promise((resolve) => {
    const res = {
      req,
      writeHead(status, headers) { this.status = status; this.headers = headers; },
      end(text) { resolve({ status: this.status, headers: this.headers, body: text ? JSON.parse(text) : null }); },
    };
    req.res = res;
    app(req, res);
  });
}

test("health answers without a token and says whether sign-in is configured", async () => {
  const off = await call(createApp({ publicKey: null, appId: "" }), { path: "/api/account/health" });
  assert.equal(off.status, 200);
  assert.equal(off.body.configured, false);
  const on = await call(createApp({ publicKey, appId: APP_ID }), { path: "/api/account/health" });
  assert.equal(on.body.configured, true);
});

test("every other route refuses an anonymous caller", async () => {
  const app = createApp({ publicKey, appId: APP_ID });
  for (const path of ["/api/account/me", "/api/account/saved", "/api/account/request", "/api/account/publisher-request"]) {
    const r = await call(app, { path, method: "POST" });
    assert.equal(r.status, 401, path);
  }
});

test("no account response carries a CORS header", async () => {
  const app = createApp({ publicKey, appId: APP_ID });
  for (const r of [await call(app, { path: "/api/account/health" }), await call(app, { token: mint() })]) {
    const names = Object.keys(r.headers).map((h) => h.toLowerCase());
    assert.ok(!names.some((h) => h.startsWith("access-control-")), names.join(","));
    assert.equal(r.headers["cache-control"], "no-store");
  }
});

// A token the service will accept against the real clock, since the service does not take a `now`.
const live = () => {
  const t = Math.floor(Date.now() / 1000);
  return mint({ claims: { iat: t - 10, exp: t + 3600 } });
};

test("a saved model round-trips and a request is recorded, issuing nothing", async () => {
  const { mkdtemp, readFile } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const dir = await mkdtemp(join(tmpdir(), "hub-account-"));
  process.env.HUB_ACCOUNT_STATE = dir;
  const app = createApp({ publicKey, appId: APP_ID });
  const token = live();

  assert.equal((await call(app, { method: "POST", path: "/api/account/saved", token, body: { model: "hexgrad/Kokoro-82M" } })).status, 200);
  assert.deepEqual((await call(app, { token })).body.saved, ["hexgrad/Kokoro-82M"]);
  assert.equal((await call(app, { method: "POST", path: "/api/account/saved", token, body: { model: "not a model id" } })).status, 400);
  assert.deepEqual((await call(app, { method: "DELETE", path: "/api/account/saved/hexgrad%2FKokoro-82M", token })).body.saved, []);

  const pub = await call(app, { method: "POST", path: "/api/account/publisher-request", token, body: { note: "I maintain two models" } });
  assert.equal(pub.status, 202);
  assert.equal(pub.body.issued, false);

  // What lands on disk names a hash, never a person.
  const line = JSON.parse(await readFile(join(dir, "publisher-requests.jsonl"), "utf8"));
  assert.match(line.did, /^[0-9a-f]{64}$/);
  assert.ok(!JSON.stringify(line).includes("did:privy:"));
});
