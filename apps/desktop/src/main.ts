import { invoke } from "@tauri-apps/api/core";
import "./styles.css";

type Action = "daemon_start" | "daemon_stop" | "daemon_restart" | "daemon_status";

type ModuleInfo = {
  id: string;
  name: string;
  version: string;
  state: string;
  dependencies: string[];
  operations: string[];
};

const output = document.querySelector<HTMLPreElement>("#output")!;
const state = document.querySelector<HTMLSpanElement>("#state")!;
const buttons = [...document.querySelectorAll<HTMLButtonElement>("button")];
const modules = document.querySelector<HTMLDivElement>("#modules")!;

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
    await refreshModules();
  } catch (error) {
    output.textContent = String(error);
    setState("stopped");
    renderModules([]);
  }
}

async function refreshModules() {
  try {
    const result = await invoke<string>("modules_list");
    renderModules(JSON.parse(result) as ModuleInfo[]);
  } catch (error) {
    modules.innerHTML = `<p class="empty">${escapeHtml(String(error))}</p>`;
  }
}

function renderModules(items: ModuleInfo[]) {
  if (items.length === 0) {
    modules.innerHTML = '<p class="empty">No modules are available.</p>';
    return;
  }
  modules.innerHTML = items
    .map(
      (module) => `<article class="module-card">
        <div><span class="module-state">${escapeHtml(module.state)}</span><h3>${escapeHtml(module.name)}</h3></div>
        <code>${escapeHtml(module.id)}</code>
        <p>${module.operations.length} operation${module.operations.length === 1 ? "" : "s"} · v${escapeHtml(module.version)}</p>
      </article>`,
    )
    .join("");
}

function escapeHtml(value: string) {
  return value.replace(/[&<>'"]/g, (character) => {
    const entities: Record<string, string> = {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      "'": "&#39;",
      '"': "&quot;",
    };
    return entities[character];
  });
}

document.querySelector("#start")!.addEventListener("click", () => execute("daemon_start"));
document.querySelector("#restart")!.addEventListener("click", () => execute("daemon_restart"));
document.querySelector("#stop")!.addEventListener("click", () => execute("daemon_stop"));
document.querySelector("#refresh-modules")!.addEventListener("click", () => void refreshModules());

void refresh();
