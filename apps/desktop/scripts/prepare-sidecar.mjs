import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const desktop = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repository = resolve(desktop, "../..");
const release = process.argv.includes("--release");
const cargoArgs = ["build", "--locked", "--bin", "hologram"];
if (release) cargoArgs.push("--release");
execFileSync("cargo", cargoArgs, { cwd: repository, stdio: "inherit" });

const rustc = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
const target = rustc.match(/^host: (.+)$/m)?.[1];
if (!target) throw new Error("rustc did not report a host target");

const extension = process.platform === "win32" ? ".exe" : "";
const profile = release ? "release" : "debug";
const source = join(repository, "target", profile, `hologram${extension}`);
const destination = join(desktop, "src-tauri", "binaries", `hologram-${target}${extension}`);
mkdirSync(dirname(destination), { recursive: true });
copyFileSync(source, destination);
console.log(`Prepared ${destination}`);
