// The names log: which name pointed at which bytes, when, witnessed by whom. Append-only, hash-chained, signed.
//
// A model name ("Qwen/Qwen3-0.6B") is a claim nobody can check from the bytes; this log makes it checkable. Every
// entry records one observation and the sha256 of the entry before it (its canonical JSON: keys sorted, no spaces),
// so history cannot be rewritten without breaking every later link. The head (seq, sha256 of the last entry) is
// signed with the witness's Ed25519 key; its public half travels with it. A second witness running this code over
// the same names records the same observations; where two logs disagree about one (name, revision, path), one of
// them is wrong, and the bytes decide.
//
// Entry kinds
//   witnessed  name -> revision -> {path: sha256} as Hugging Face served it (LFS files by their LFS sha256)
//   indexed    name@revision -> the model's OCI index and tensor table
//   replaced   same name, revision and path, a different sha256 than an earlier witness: a silent replacement
//   moved      same name, a new revision (an update; recorded so "what did it point to on day D" has an answer)
//
// State: names.log.jsonl, names.key (PKCS#8 PEM, mode 0600, never published), names.head.json (signed head).
import { createHash, generateKeyPairSync, createPrivateKey, createPublicKey, sign, verify as edVerify } from "node:crypto";
import { existsSync, readFileSync, appendFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export const FORMAT = "hologram.names-log/v1";

export function canon(v) {
  if (Array.isArray(v)) return `[${v.map(canon).join(",")}]`;
  if (v && typeof v === "object") return `{${Object.keys(v).sort().map((k) => `${JSON.stringify(k)}:${canon(v[k])}`).join(",")}}`;
  return JSON.stringify(v);
}
const sha = (s) => createHash("sha256").update(s).digest("hex");
const signed = (h) => canon({ format: h.format, seq: h.seq, head: h.head, witness: h.witness, public_key: h.public_key });

export class NamesLog {
  constructor(state, witness = process.env.NAMES_WITNESS || "gethologram.ai/tensor-index") {
    this.path = join(state, "names.log.jsonl"); this.keyPath = join(state, "names.key"); this.headPath = join(state, "names.head.json");
    this.witness = witness;
    if (!existsSync(this.keyPath)) {
      const { privateKey } = generateKeyPairSync("ed25519");
      writeFileSync(this.keyPath, privateKey.export({ type: "pkcs8", format: "pem" }), { mode: 0o600, flag: "wx" });
    }
    this.key = createPrivateKey(readFileSync(this.keyPath));
    this.pub = createPublicKey(this.key).export({ type: "spki", format: "der" }).subarray(-32).toString("hex");
    this.seq = 0; this.prev = null; this.latest = new Map();   // "name\0rev\0path" -> sha256; "name" -> rev
    if (existsSync(this.path)) for (const line of readFileSync(this.path, "utf8").split("\n")) {
      if (!line) continue;
      const e = JSON.parse(line); this.track(e); this.seq = e.seq; this.prev = sha(canon(e));
    }
  }
  track(e) {
    if (e.kind !== "witnessed" && e.kind !== "replaced") return;
    for (const [p, s] of Object.entries(e.files || {})) this.latest.set(`${e.name}\0${e.revision}\0${p}`, s);
    this.latest.set(e.name, e.revision);
  }
  append(kind, fields) {
    const e = { format: FORMAT, seq: this.seq + 1, kind, at: new Date().toISOString().slice(0, 19) + "Z", witness: this.witness, prev: this.prev, ...fields };
    const line = canon(e);
    appendFileSync(this.path, line + "\n");
    this.seq = e.seq; this.prev = sha(line); this.track(e);
    return e;
  }
  // Record what the source serves under a name now. Returns the alerts this observation raised.
  witnessFiles(name, revision, files) {
    const alerts = [], key = (p) => `${name}\0${revision}\0${p}`;
    const changed = Object.fromEntries(Object.entries(files).filter(([p, s]) => this.latest.has(key(p)) && this.latest.get(key(p)) !== s));
    if (Object.keys(changed).length) {
      const before = Object.fromEntries(Object.keys(changed).map((p) => [p, this.latest.get(key(p))]));
      alerts.push(this.append("replaced", { name, revision, files: changed, before }));
    }
    const prior = this.latest.get(name);
    if (prior && prior !== revision) this.append("moved", { name, revision, previous: prior });
    const fresh = Object.fromEntries(Object.entries(files).filter(([p]) => !this.latest.has(key(p))));
    if (Object.keys(fresh).length) this.append("witnessed", { name, revision, files: fresh });
    return alerts;
  }
  indexed(name, revision, index, table) { return this.append("indexed", { name, revision, index, table }); }
  signHead() {
    const h = { format: FORMAT, seq: this.seq, head: this.prev, witness: this.witness, public_key: this.pub, at: new Date().toISOString().slice(0, 19) + "Z" };
    h.signature = sign(null, Buffer.from(signed(h)), this.key).toString("hex");
    writeFileSync(this.headPath, JSON.stringify(h, null, 1));
    return h;
  }
}

// Replay a log against its signed head: [entries, problems]; no problems means intact.
export function verifyLog(text, head) {
  const problems = [], entries = []; let prev = null;
  text.split("\n").filter(Boolean).forEach((line, i) => {
    const e = JSON.parse(line);
    if (e.seq !== i + 1) problems.push(`entry ${i + 1}: sequence ${e.seq}`);
    if (e.prev !== prev) problems.push(`entry ${i + 1}: prev link broken`);
    prev = sha(canon(e)); entries.push(e);
  });
  if (head.seq !== entries.length || head.head !== prev) problems.push(`head says ${head.seq}/${head.head}, log ends at ${entries.length}/${prev}`);
  try {
    const key = createPublicKey({ key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(head.public_key, "hex")]), format: "der", type: "spki" });
    if (!edVerify(null, Buffer.from(signed(head)), key, Buffer.from(head.signature, "hex"))) problems.push("head signature does not verify");
  } catch { problems.push("head signature does not verify"); }
  return [entries, problems];
}

// What `name` pointed to at ISO time `when`, or as of entry `upto` (exact). Default: the latest.
export function pointedAt(entries, name, { when, upto } = {}) {
  let rev = null, files = {}, index = null;
  for (const e of entries) {
    if (e.name !== name || (when && e.at > when) || (upto && e.seq > upto)) continue;
    if (e.kind === "witnessed" || e.kind === "replaced" || e.kind === "moved") {
      if (e.revision !== rev) { rev = e.revision; files = {}; index = null; }
      Object.assign(files, e.files || {});
    } else if (e.kind === "indexed" && e.revision === rev) index = e.index;
  }
  return { name, revision: rev, files, index };
}
