// Vendors the Privy browser SDK at a pinned version, the way scripts/vendor-kit.mjs vendors the brand kit.
//
//   node apps/model-hub/web/scripts/vendor-privy.mjs           rebuild vendor/privy from the pinned version
//   node apps/model-hub/web/scripts/vendor-privy.mjs --check    verify what is committed still matches, byte for byte
//
// Why vendor at all: the hub never loads a script from someone else's origin at runtime. A visitor's browser talks
// to gethologram.ai for code and to Privy only for the sign-in itself. Pinning it here also means a build is
// reproducible and a version bump is a reviewable commit with a hash in it, not a silent change under everyone.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const VERSION = "0.76.2";   // @privy-io/js-sdk-core
const ESBUILD = "0.25.12";

const SITE = join(dirname(fileURLToPath(import.meta.url)), "..");
const OUT = join(SITE, "vendor", "privy");
const FILE = `privy-${VERSION}.mjs`;
const check = process.argv.includes("--check");

// Email and the two OAuth providers are the hub's only ways in, so the phone-number tables that ship for SMS login
// are dead weight — a sixth of the bundle. Stubbed rather than deleted: if a future version reaches for them, the
// error says exactly that instead of behaving strangely.
const PHONE_STUB = `const off=()=>{throw new Error("phone login is not enabled on this hub")};
export const parsePhoneNumberWithError=off;export const parsePhoneNumber=off;export const parsePhoneNumberFromString=()=>undefined;
export const isValidPhoneNumber=()=>false;export const isPossiblePhoneNumber=()=>false;export const validatePhoneNumberLength=()=>"NOT_A_NUMBER";
export const AsYouType=class{input(s){return s}};export const getCountries=()=>[];export const getCountryCallingCode=()=>"";
export const getExampleNumber=()=>undefined;export const formatIncompletePhoneNumber=(s)=>s;export const formatPhoneNumber=(s)=>s;
export const formatPhoneNumberIntl=(s)=>s;export const findPhoneNumbersInText=()=>[];export const searchPhoneNumbersInText=()=>[];
export const parseIncompletePhoneNumber=(s)=>s;export const Metadata=class{};export default {};`;

const work = await mkdtemp(join(tmpdir(), "vendor-privy-"));
try {
  execFileSync("npm", ["init", "-y"], { cwd: work, stdio: "ignore", shell: process.platform === "win32" });
  execFileSync("npm", ["install", "--no-fund", "--no-audit", `@privy-io/js-sdk-core@${VERSION}`, `esbuild@${ESBUILD}`], { cwd: work, stdio: "inherit", shell: process.platform === "win32" });

  const esbuild = await import(pathToFileURL(join(work, "node_modules", "esbuild", "lib", "main.js")).href);
  const built = await esbuild.build({
    stdin: { contents: 'export {default as Privy, LocalStorage} from "@privy-io/js-sdk-core";\n', resolveDir: work, sourcefile: "hub-privy.mjs", loader: "js" },
    bundle: true, format: "esm", platform: "browser", target: "es2022",
    minify: true, legalComments: "none", write: false,
    plugins: [{
      name: "no-phone",
      setup(build) {
        build.onResolve({ filter: /^libphonenumber-js(\/.*)?$/ }, () => ({ path: "phone", namespace: "stub" }));
        build.onLoad({ filter: /^phone$/, namespace: "stub" }, () => ({ contents: PHONE_STUB, loader: "js" }));
      },
    }],
  });

  // esbuild keeps a helper process alive; the temporary tree cannot be removed under Windows until it stops.
  await esbuild.stop?.();

  const bytes = Buffer.from(built.outputFiles[0].contents);
  const sha256 = createHash("sha256").update(bytes).digest("hex");
  const stamp = { package: "@privy-io/js-sdk-core", version: VERSION, esbuild: ESBUILD, file: FILE, bytes: bytes.length, sha256 };

  if (check) {
    const path = join(OUT, FILE);
    if (!existsSync(path)) throw new Error(`vendor/privy/${FILE} is missing: run scripts/vendor-privy.mjs`);
    const have = createHash("sha256").update(await readFile(path)).digest("hex");
    if (have !== sha256) throw new Error(`vendor/privy/${FILE} does not match a fresh build of ${VERSION}\n  committed ${have}\n  rebuilt   ${sha256}`);
    const stamped = JSON.parse(await readFile(join(OUT, "VENDORED.json"), "utf8"));
    if (stamped.sha256 !== sha256) throw new Error("VENDORED.json disagrees with the file next to it");
    console.log(`privy ${VERSION} matches: ${(bytes.length / 1024).toFixed(0)} KB, sha256 ${sha256.slice(0, 16)}…`);
  } else {
    await mkdir(OUT, { recursive: true });
    await writeFile(join(OUT, FILE), bytes);
    await writeFile(join(OUT, "VENDORED.json"), `${JSON.stringify(stamp, null, 2)}\n`);
    console.log(`vendored privy ${VERSION}: ${(bytes.length / 1024).toFixed(0)} KB, sha256 ${sha256.slice(0, 16)}…`);
  }
} finally {
  await rm(work, { recursive: true, force: true });
}
