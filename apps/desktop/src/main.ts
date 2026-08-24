import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import "./styles.css";

type Action = "service_start" | "service_stop" | "service_restart";
type ServiceState = "ready" | "stopped" | "unknown";
type Theme = "light" | "dark";
type View = "overview" | "chat" | "files" | "modules";

type ModuleInfo = {
  id: string;
  name: string;
  version: string;
  state: string;
  dependencies: string[];
  operations: string[];
};

type ObjectMetadata = {
  id: string;
  kind: string;
  media_type: string;
  filename: string | null;
  size: number;
  created_at_millis: number;
};

type ConversationMessage = {
  role: string;
  content: string;
  created_at_millis: number;
};

type Conversation = {
  id: string;
  title: string;
  created_at_millis: number;
  updated_at_millis: number;
  messages: ConversationMessage[];
};

const pageCopy: Record<View, { title: string; description: string }> = {
  overview: { title: "Overview", description: "Your local Hologram workspace" },
  chat: { title: "Chat", description: "Conversation threads saved to local history" },
  files: { title: "Files", description: "Add, browse, and download stored content" },
  modules: { title: "Modules", description: "Capabilities available to Hologram" },
};

const notice = document.querySelector<HTMLDivElement>("#notice")!;
const modules = document.querySelector<HTMLDivElement>("#modules")!;
const objects = document.querySelector<HTMLDivElement>("#objects")!;
const moduleCount = document.querySelector<HTMLElement>("#module-count")!;
const objectCount = document.querySelector<HTMLElement>("#object-count")!;
const threadCount = document.querySelector<HTMLElement>("#thread-count")!;
const navModuleCount = document.querySelector<HTMLElement>("#nav-module-count")!;
const navObjectCount = document.querySelector<HTMLElement>("#nav-object-count")!;
const navThreadCount = document.querySelector<HTMLElement>("#nav-thread-count")!;
const content = document.querySelector<HTMLElement>(".content")!;
const threads = document.querySelector<HTMLDivElement>("#threads")!;
const threadSearch = document.querySelector<HTMLInputElement>("#thread-search")!;
const chatTitle = document.querySelector<HTMLElement>("#chat-title")!;
const chatDetail = document.querySelector<HTMLElement>("#chat-detail")!;
const chatEmpty = document.querySelector<HTMLDivElement>("#chat-empty")!;
const chatScroll = document.querySelector<HTMLDivElement>("#chat-scroll")!;
const messages = document.querySelector<HTMLDivElement>("#messages")!;
const chatForm = document.querySelector<HTMLFormElement>("#chat-form")!;
const chatInput = document.querySelector<HTMLTextAreaElement>("#chat-input")!;
const serviceTitle = document.querySelector<HTMLElement>("#service-title")!;
const serviceDescription = document.querySelector<HTMLElement>("#service-description")!;
const themeToggle = document.querySelector<HTMLButtonElement>("#theme-toggle")!;
const themeIcon = document.querySelector<HTMLElement>("#theme-icon")!;
const themeLabel = document.querySelector<HTMLElement>("#theme-label")!;
const systemTheme = window.matchMedia("(prefers-color-scheme: dark)");
let currentState: ServiceState = "unknown";
let isBusy = false;
let noticeTimer: number | undefined;
let themePreference = storedTheme();
let conversations: Conversation[] = [];
let activeThreadId: string | null = null;
let chatBusy = false;

function storedTheme(): Theme | null {
  try {
    const value = window.localStorage.getItem("hologram-theme");
    return value === "light" || value === "dark" ? value : null;
  } catch {
    return null;
  }
}

function applyTheme(theme: Theme, remember = false) {
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.colorScheme = theme;
  const nextTheme = theme === "light" ? "dark" : "light";
  const nextLabel = nextTheme === "dark" ? "Dark mode" : "Light mode";
  themeIcon.textContent = theme === "light" ? "☾" : "☀";
  themeLabel.textContent = nextLabel;
  themeToggle.setAttribute("aria-label", `Use ${nextLabel.toLowerCase()}`);
  themeToggle.title = `Use ${nextLabel.toLowerCase()}`;
  document.querySelector<HTMLMetaElement>('meta[name="theme-color"]')!.content = theme === "light" ? "#f7f7f5" : "#161615";
  if (remember) {
    themePreference = theme;
    try {
      window.localStorage.setItem("hologram-theme", theme);
    } catch {
      // The selected theme still applies for this session.
    }
  }
}

applyTheme(themePreference ?? (systemTheme.matches ? "dark" : "light"));

function syncControls() {
  document.querySelectorAll<HTMLButtonElement>(".command-button").forEach((button) => {
    const chatAction = button.id === "send-message" || button.id === "new-thread";
    button.disabled = isBusy || (chatBusy && chatAction) || (button.dataset.when !== undefined && button.dataset.when !== currentState);
  });
  chatInput.disabled = currentState !== "ready" || chatBusy;
  threadSearch.disabled = currentState !== "ready";
}

function setBusy(busy: boolean) {
  isBusy = busy;
  syncControls();
}

function setState(value: ServiceState) {
  currentState = value;
  document.querySelectorAll<HTMLElement>(".state-dot").forEach((dot) => {
    dot.className = `state-dot ${value}`;
  });
  document.querySelectorAll<HTMLElement>(".state-label").forEach((label) => {
    label.textContent = value === "ready" ? "Ready" : value === "stopped" ? "Stopped" : "Checking";
  });
  serviceTitle.textContent = value === "ready" ? "Hologram is ready" : value === "stopped" ? "Hologram is stopped" : "Connecting to Hologram";
  serviceDescription.textContent = value === "ready"
    ? "Your modules and local files are available."
    : value === "stopped"
      ? "Start Hologram to use your local workspace."
      : "Checking that your modules and files are available.";
  syncControls();
}

function showNotice(message: string, tone: "neutral" | "success" | "error" = "neutral", persistent = false) {
  window.clearTimeout(noticeTimer);
  notice.textContent = message;
  notice.className = `notice ${tone}`;
  if (!persistent) {
    noticeTimer = window.setTimeout(() => notice.classList.add("is-hidden"), 3200);
  }
}

function showView(view: View) {
  document.querySelectorAll<HTMLElement>("[data-page]").forEach((page) => {
    page.hidden = page.dataset.page !== view;
  });
  document.querySelectorAll<HTMLButtonElement>(".nav-item").forEach((item) => {
    const active = item.dataset.view === view;
    item.classList.toggle("active", active);
    active ? item.setAttribute("aria-current", "page") : item.removeAttribute("aria-current");
  });
  document.querySelector("#page-title")!.textContent = pageCopy[view].title;
  document.querySelector("#page-description")!.textContent = pageCopy[view].description;
  content.classList.toggle("chat-content", view === "chat");
  if (view === "chat" && currentState === "ready") {
    window.setTimeout(() => chatInput.focus(), 0);
  }
}

async function execute(action: Action) {
  setBusy(true);
  const present = action === "service_start" ? "Starting" : action === "service_stop" ? "Stopping" : "Restarting";
  showNotice(`${present} Hologram…`, "neutral", true);
  try {
    await invoke<string>(action);
    if (action === "service_stop") {
      setState("stopped");
      renderModules([], true);
      renderObjects([], true);
      renderChatUnavailable();
      showNotice("Hologram has stopped.", "success");
    } else {
      await refresh();
      showNotice(action === "service_start" ? "Hologram is ready." : "Hologram restarted successfully.", "success");
    }
  } catch (error) {
    showNotice(friendlyError(error), "error", true);
    setState("stopped");
  } finally {
    setBusy(false);
  }
}

async function refresh() {
  try {
    await invoke<string>("service_status");
    setState("ready");
    const results = await Promise.all([refreshModules(), refreshObjects(), refreshChatThreads(true)]);
    showNotice(results.every(Boolean) ? "Everything is up to date." : "Some items couldn’t be loaded.", results.every(Boolean) ? "success" : "error");
  } catch {
    setState("stopped");
    showNotice("Start Hologram to use your local workspace.", "neutral");
    renderModules([], true);
    renderObjects([], true);
    renderChatUnavailable();
  }
}

async function refreshObjects() {
  try {
    const result = await invoke<string>("objects_list");
    renderObjects(JSON.parse(result) as ObjectMetadata[]);
    return true;
  } catch (error) {
    setObjectCount("—");
    objects.innerHTML = emptyState("!", "Files couldn’t be loaded", "Check that Hologram is ready, then try again.", "Try again", "retry-objects");
    objects.querySelector(".retry-objects")!.addEventListener("click", () => void refreshObjects());
    syncControls();
    showNotice(friendlyError(error), "error", true);
    return false;
  }
}

async function uploadFile() {
  const path = await open({ multiple: false, directory: false, title: "Add a file to Hologram" });
  if (path === null) return;
  setBusy(true);
  showNotice("Adding your file…", "neutral", true);
  try {
    await invoke<string>("file_put", { path });
    await refreshObjects();
    showNotice("File added successfully.", "success");
  } catch (error) {
    showNotice(friendlyError(error), "error", true);
  } finally {
    setBusy(false);
  }
}

async function downloadObject(item: ObjectMetadata) {
  const path = await save({ defaultPath: safeFilename(item), title: "Save file" });
  if (path === null) return;
  setBusy(true);
  showNotice("Saving your file…", "neutral", true);
  try {
    await invoke<string>("object_get", { id: item.id, output: path });
    showNotice("File saved successfully.", "success");
  } catch (error) {
    showNotice(friendlyError(error), "error", true);
  } finally {
    setBusy(false);
  }
}

async function refreshModules() {
  try {
    const result = await invoke<string>("modules_list");
    renderModules(JSON.parse(result) as ModuleInfo[]);
    return true;
  } catch (error) {
    setModuleCount("—");
    modules.innerHTML = emptyState("!", "Modules couldn’t be loaded", "Check that Hologram is ready, then try again.", "Try again", "retry-modules");
    modules.querySelector(".retry-modules")!.addEventListener("click", () => void refreshModules());
    syncControls();
    showNotice(friendlyError(error), "error", true);
    return false;
  }
}

async function refreshChatThreads(selectFirst = false) {
  try {
    const result = await invoke<string>("history_list");
    conversations = JSON.parse(result) as Conversation[];
    setThreadCount(String(conversations.length));
    if (activeThreadId !== null && !conversations.some((item) => item.id === activeThreadId)) {
      activeThreadId = null;
    }
    if (activeThreadId === null && selectFirst && conversations.length > 0) {
      activeThreadId = conversations[0].id;
    }
    renderThreadList();
    const active = conversations.find((item) => item.id === activeThreadId);
    if (active !== undefined) renderConversation(active);
    else renderNewChat();
    return true;
  } catch (error) {
    setThreadCount("—");
    threads.innerHTML = '<div class="thread-empty">Chats couldn’t be loaded.</div>';
    renderChatUnavailable("Chat history couldn’t be loaded.");
    showNotice(friendlyError(error), "error", true);
    return false;
  }
}

function renderThreadList() {
  const query = threadSearch.value.trim().toLocaleLowerCase();
  const filtered = conversations.filter((item) => item.title.toLocaleLowerCase().includes(query));
  if (filtered.length === 0) {
    threads.innerHTML = `<div class="thread-empty">${query === "" ? "No chats yet." : "No matching chats."}</div>`;
    return;
  }
  threads.innerHTML = filtered.map((conversation) => {
    const lastMessage = conversation.messages.at(-1)?.content.replace(/\s+/g, " ").trim() || "No messages yet";
    return `<button class="thread-item${conversation.id === activeThreadId ? " active" : ""}" data-thread-id="${escapeHtml(conversation.id)}" type="button">
      <span class="thread-glyph" aria-hidden="true">◌</span>
      <span><strong>${escapeHtml(conversation.title)}</strong><small>${escapeHtml(lastMessage)}</small></span>
      <time>${escapeHtml(formatThreadTime(conversation.updated_at_millis))}</time>
    </button>`;
  }).join("");
  threads.querySelectorAll<HTMLButtonElement>(".thread-item").forEach((button) => {
    button.addEventListener("click", () => void selectThread(button.dataset.threadId!));
  });
}

async function selectThread(id: string) {
  if (chatBusy || id === activeThreadId) return;
  setChatBusy(true);
  try {
    const result = await invoke<string>("history_get", { id });
    const conversation = JSON.parse(result) as Conversation;
    activeThreadId = id;
    const index = conversations.findIndex((item) => item.id === id);
    if (index >= 0) conversations[index] = conversation;
    renderThreadList();
    renderConversation(conversation);
  } catch (error) {
    showNotice(friendlyError(error), "error", true);
  } finally {
    setChatBusy(false);
  }
}

function startNewThread() {
  if (currentState !== "ready" || chatBusy) return;
  activeThreadId = null;
  threadSearch.value = "";
  renderThreadList();
  renderNewChat();
  chatInput.focus();
}

function renderNewChat() {
  chatTitle.textContent = "New chat";
  chatDetail.textContent = "Messages are saved to this thread";
  messages.hidden = true;
  messages.innerHTML = "";
  setChatWelcome(
    "Chat with Hologram",
    "Start a conversation. For now, Hologram will repeat your message and save both sides to this thread.",
    "Conversations stay in your local history.",
  );
}

function renderConversation(conversation: Conversation, pending = false) {
  chatTitle.textContent = conversation.title;
  chatDetail.textContent = `${conversation.messages.length} saved message${conversation.messages.length === 1 ? "" : "s"}`;
  if (conversation.messages.length === 0) {
    messages.hidden = true;
    setChatWelcome(
      "Start this conversation",
      "Send a message and the echo module will answer with the same text.",
      "This thread is stored in your local history.",
    );
    return;
  }
  chatEmpty.hidden = true;
  messages.hidden = false;
  messages.innerHTML = conversation.messages.map((message) => messageMarkup(message)).join("") + (pending
    ? '<article class="message assistant pending"><span class="message-author">Hologram</span><div class="typing" aria-label="Hologram is responding"><i></i><i></i><i></i></div></article>'
    : "");
  window.requestAnimationFrame(() => {
    chatScroll.scrollTop = chatScroll.scrollHeight;
  });
}

function messageMarkup(message: ConversationMessage) {
  const role = message.role === "user" ? "user" : "assistant";
  const author = role === "user" ? "You" : "Hologram";
  return `<article class="message ${role}">
    <span class="message-author">${author}</span>
    <div>${escapeHtml(message.content)}</div>
    <time>${escapeHtml(formatMessageTime(message.created_at_millis))}</time>
  </article>`;
}

function setChatWelcome(title: string, copy: string, status: string) {
  chatEmpty.hidden = false;
  chatEmpty.querySelector("h2")!.textContent = title;
  chatEmpty.querySelector("p")!.textContent = copy;
  chatEmpty.querySelector("div")!.lastChild!.textContent = ` ${status}`;
}

function renderChatUnavailable(copy = "Start Hologram to load your chat threads.") {
  conversations = [];
  activeThreadId = null;
  setThreadCount("—");
  threads.innerHTML = '<div class="thread-empty">Hologram is stopped.</div>';
  chatTitle.textContent = "Chat";
  chatDetail.textContent = "Conversation history is unavailable";
  messages.hidden = true;
  messages.innerHTML = "";
  setChatWelcome("Chat is unavailable", copy, "Your saved threads will appear when Hologram is ready.");
  syncControls();
}

async function sendChatMessage() {
  const text = chatInput.value.trim();
  if (text === "" || currentState !== "ready" || chatBusy) return;
  setChatBusy(true);
  chatInput.value = "";
  resizeComposer();
  showNotice("Sending message…", "neutral", true);
  try {
    let conversation = conversations.find((item) => item.id === activeThreadId);
    if (conversation === undefined) {
      const result = await invoke<string>("history_create", { title: threadTitle(text) });
      conversation = JSON.parse(result) as Conversation;
      activeThreadId = conversation.id;
      conversations.unshift(conversation);
      setThreadCount(String(conversations.length));
      renderThreadList();
    }
    const optimistic: Conversation = {
      ...conversation,
      messages: [...conversation.messages, { role: "user", content: text, created_at_millis: Date.now() }],
    };
    renderConversation(optimistic, true);
    const result = await sendChatWithCompatibilityRecovery(conversation.id, text);
    const updated = JSON.parse(result) as Conversation;
    activeThreadId = updated.id;
    renderConversation(updated);
    await refreshChatThreads();
    showNotice("Reply saved to this thread.", "success");
  } catch (error) {
    chatInput.value = text;
    resizeComposer();
    showNotice(friendlyError(error), "error", true);
    const active = conversations.find((item) => item.id === activeThreadId);
    if (active !== undefined) renderConversation(active);
    else renderNewChat();
  } finally {
    setChatBusy(false);
    chatInput.focus();
  }
}

async function sendChatWithCompatibilityRecovery(id: string, content: string) {
  try {
    return await invoke<string>("chat_send", { id, content });
  } catch (error) {
    if (!isMissingChatCapability(error)) throw error;
    showNotice("Updating the local chat demo…", "neutral", true);
    await invoke<string>("service_restart");
    setState("ready");
    await refreshModules();
    return invoke<string>("chat_send", { id, content });
  }
}

function isMissingChatCapability(error: unknown) {
  const message = String(error).replaceAll("\\", "");
  return message.includes("LIVE_CAPABILITY_MISSING") && message.includes("chat.send");
}

function setChatBusy(busy: boolean) {
  chatBusy = busy;
  threads.setAttribute("aria-busy", String(busy));
  syncControls();
}

function resizeComposer() {
  chatInput.style.height = "auto";
  chatInput.style.height = `${Math.min(chatInput.scrollHeight, 150)}px`;
}

function threadTitle(message: string) {
  const title = message.replace(/\s+/g, " ").trim();
  return title.length <= 48 ? title : `${title.slice(0, 47).trimEnd()}…`;
}

function formatThreadTime(value: number) {
  const date = new Date(value);
  const today = new Date();
  return date.toDateString() === today.toDateString()
    ? date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })
    : date.toLocaleDateString([], { month: "short", day: "numeric" });
}

function formatMessageTime(value: number) {
  return new Date(value).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

function renderModules(items: ModuleInfo[], unavailable = false) {
  setModuleCount(unavailable ? "—" : String(items.length));
  if (items.length === 0) {
    modules.innerHTML = unavailable
      ? emptyState("◇", "Hologram is stopped", "Start Hologram to see its available modules.", "Go to Overview", "open-overview")
      : emptyState("◇", "No modules are enabled", "Enabled modules will appear here when they are available.");
    modules.querySelector(".open-overview")?.addEventListener("click", () => showView("overview"));
    syncControls();
    return;
  }
  modules.innerHTML = items.map((module) => `<article class="module-row">
    <div class="module-description">
      <div><strong>${escapeHtml(module.name)}</strong><span class="module-state">${escapeHtml(module.state)}</span></div>
      <code>${escapeHtml(module.id)}</code>
      <p>${module.operations.length} operation${module.operations.length === 1 ? "" : "s"} · Version ${escapeHtml(module.version)}</p>
    </div>
  </article>`).join("");
}

function renderObjects(items: ObjectMetadata[], unavailable = false) {
  setObjectCount(unavailable ? "—" : String(items.length));
  if (items.length === 0) {
    objects.innerHTML = unavailable
      ? emptyState("□", "Hologram is stopped", "Start Hologram to browse your local files.", "Go to Overview", "open-overview")
      : emptyState("+", "Add your first file", "Files you add to Hologram will appear here.", "Add file", "empty-upload");
    objects.querySelector(".open-overview")?.addEventListener("click", () => showView("overview"));
    objects.querySelector(".empty-upload")?.addEventListener("click", () => void uploadFile());
    syncControls();
    return;
  }
  objects.innerHTML = items.map((item, index) => `<article class="object-row">
    <div class="object-description">
      <div><strong>${escapeHtml(item.filename ?? "Unnamed file")}</strong><span class="kind">${escapeHtml(item.kind)}</span></div>
      <code title="${escapeHtml(item.id)}">${escapeHtml(item.id)}</code>
      <p>${escapeHtml(item.media_type)} · ${formatBytes(item.size)}</p>
    </div>
    <div class="object-actions">
      <button class="secondary command-button rename-object" data-when="ready" data-index="${index}">Rename</button>
      <button class="secondary command-button download-object" data-when="ready" data-index="${index}">Download</button>
    </div>
  </article>`).join("");
  objects.querySelectorAll<HTMLButtonElement>(".rename-object").forEach((button) => {
    button.addEventListener("click", () => beginObjectRename(items, Number(button.dataset.index)));
  });
  objects.querySelectorAll<HTMLButtonElement>(".download-object").forEach((button) => {
    button.addEventListener("click", () => void downloadObject(items[Number(button.dataset.index)]));
  });
  syncControls();
}

function beginObjectRename(items: ObjectMetadata[], index: number) {
  const item = items[index];
  const row = objects.querySelectorAll<HTMLElement>(".object-row")[index];
  if (item === undefined || row === undefined) return;
  const description = row.querySelector<HTMLElement>(".object-description")!;
  const currentName = item.filename ?? "";
  description.innerHTML = `<form class="rename-form">
    <label class="visually-hidden" for="rename-file-${index}">File name</label>
    <input id="rename-file-${index}" maxlength="255" autocomplete="off" placeholder="Enter a file name" value="${escapeHtml(currentName)}" />
    <button class="primary command-button rename-save" data-when="ready" type="submit">Save</button>
    <button class="secondary command-button rename-cancel" data-when="ready" type="button">Cancel</button>
  </form>
  <code title="${escapeHtml(item.id)}">${escapeHtml(item.id)}</code>
  <p>${escapeHtml(item.media_type)} · ${formatBytes(item.size)}</p>`;
  row.querySelector<HTMLElement>(".object-actions")!.hidden = true;
  const form = row.querySelector<HTMLFormElement>(".rename-form")!;
  const input = form.querySelector<HTMLInputElement>("input")!;
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    void renameObject(item, input.value);
  });
  form.querySelector<HTMLButtonElement>(".rename-cancel")!.addEventListener("click", () => renderObjects(items));
  input.addEventListener("keydown", (event) => {
    if (event.key === "Escape") renderObjects(items);
  });
  input.focus();
  input.select();
}

async function renameObject(item: ObjectMetadata, filename: string) {
  const nextName = filename.trim();
  if (nextName.length === 0) {
    showNotice("Enter a file name.", "error", true);
    return;
  }
  setBusy(true);
  showNotice("Renaming your file…", "neutral", true);
  try {
    await invoke<string>("file_rename", { id: item.id, filename: nextName });
    await refreshObjects();
    showNotice(`Renamed to ${nextName}.`, "success");
  } catch (error) {
    showNotice(friendlyError(error), "error", true);
  } finally {
    setBusy(false);
  }
}

function setModuleCount(value: string) {
  moduleCount.textContent = value;
  navModuleCount.textContent = value;
}

function setObjectCount(value: string) {
  objectCount.textContent = value;
  navObjectCount.textContent = value;
}

function setThreadCount(value: string) {
  threadCount.textContent = value;
  navThreadCount.textContent = value;
}

function emptyState(symbol: string, title: string, copy: string, action?: string, actionClass?: string) {
  return `<article class="empty-state">
    <span class="empty-symbol" aria-hidden="true">${escapeHtml(symbol)}</span>
    <strong>${escapeHtml(title)}</strong>
    <p>${escapeHtml(copy)}</p>
    ${action === undefined ? "" : `<button class="secondary command-button ${actionClass}" data-when="${actionClass === "empty-upload" ? "ready" : currentState}">${escapeHtml(action)}</button>`}
  </article>`;
}

function safeFilename(item: ObjectMetadata) {
  const name = item.filename?.split(/[\\/]/).pop();
  return name || item.id.replace(":", "_");
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function friendlyError(error: unknown) {
  const message = String(error).replace(/daemon/gi, "Hologram").replace(/^Error:\s*/i, "");
  return message.trim() || "Hologram couldn’t complete that action.";
}

function escapeHtml(value: string) {
  return value.replace(/[&<>'"]/g, (character) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;",
  })[character]!);
}

document.querySelectorAll<HTMLButtonElement>(".nav-item").forEach((button) => {
  button.addEventListener("click", () => showView(button.dataset.view as View));
});
document.querySelectorAll<HTMLButtonElement>(".open-view").forEach((button) => {
  button.addEventListener("click", () => showView(button.dataset.target as View));
});
document.querySelector("#start")!.addEventListener("click", () => void execute("service_start"));
document.querySelector("#restart")!.addEventListener("click", () => void execute("service_restart"));
document.querySelector("#stop")!.addEventListener("click", () => void execute("service_stop"));
document.querySelector("#refresh-modules")!.addEventListener("click", () => void refreshModules());
document.querySelector("#refresh-objects")!.addEventListener("click", () => void refreshObjects());
document.querySelector("#upload-file")!.addEventListener("click", () => void uploadFile());
document.querySelector("#new-thread")!.addEventListener("click", startNewThread);
threadSearch.addEventListener("input", renderThreadList);
chatForm.addEventListener("submit", (event) => {
  event.preventDefault();
  void sendChatMessage();
});
chatInput.addEventListener("input", resizeComposer);
chatInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
    event.preventDefault();
    chatForm.requestSubmit();
  }
});
themeToggle.addEventListener("click", () => {
  const current = document.documentElement.dataset.theme === "dark" ? "dark" : "light";
  applyTheme(current === "light" ? "dark" : "light", true);
});
systemTheme.addEventListener("change", (event) => {
  if (themePreference === null) applyTheme(event.matches ? "dark" : "light");
});

void listen<ServiceState>("service-state-changed", (event) => {
  const previous = currentState;
  setState(event.payload);
  if (event.payload === "stopped") {
    renderModules([], true);
    renderObjects([], true);
    renderChatUnavailable();
    showNotice("Hologram has stopped.", "neutral");
  } else if (!isBusy && previous !== "ready") {
    void Promise.all([refreshModules(), refreshObjects(), refreshChatThreads(true)]);
    showNotice("Hologram is ready.", "success");
  }
});

setState("unknown");
void refresh();
