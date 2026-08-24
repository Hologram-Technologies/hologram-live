import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

type Action = "daemon_start" | "daemon_stop" | "daemon_restart" | "daemon_status";

const output = document.querySelector<HTMLPreElement>("#output")!;
const state = document.querySelector<HTMLSpanElement>("#state")!;
const buttons = [...document.querySelectorAll<HTMLButtonElement>("button")];

function setBusy(busy: boolean) {
  buttons.forEach((button) => {
    button.disabled = busy;
  });
}

function setState(value: "ready" | "stopped" | "unknown") {
  state.className = `state ${value}`;
  state.textContent = value === "ready" ? "Running" : value === "stopped" ? "Stopped" : "Checking";
}

async function execute(action: Action) {
  setBusy(true);
  output.textContent = "Working…";
  try {
    const result = await invoke<string>(action);
    output.textContent = result.trim() || "Command completed.";
    if (action === "daemon_stop") {
      setState("stopped");
    } else {
      await refresh();
    }
  } catch (error) {
    output.textContent = String(error);
    setState("stopped");
  } finally {
    setBusy(false);
  }
}

async function refresh() {
  try {
    const result = await invoke<string>("daemon_status");
    output.textContent = result.trim();
    setState("ready");
  } catch (error) {
    output.textContent = String(error);
    setState("stopped");
  }
}

document.querySelector("#start")!.addEventListener("click", () => execute("daemon_start"));
document.querySelector("#restart")!.addEventListener("click", () => execute("daemon_restart"));
document.querySelector("#stop")!.addEventListener("click", () => execute("daemon_stop"));

void refresh();
