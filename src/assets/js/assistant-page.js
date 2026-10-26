// 串口助手页：自己打开/关闭会话，通过数据 WS 收发，不依赖 debug.frame。

import { getConfig, ManagementClient } from "./api.js";
import { SerialSession } from "./serial.js";
import { fmtTime, bytesToHex, bytesToAscii, hexToBytes, download } from "./format.js";

const $ = (selector) => document.querySelector(selector);

const EOL = { none: "", crlf: "\r\n", lf: "\n", cr: "\r" };
const LS = {
  params: "ws2s.assistant.params.v1",
  history: "ws2s.assistant.history.v1",
  commands: "ws2s.assistant.commands.v1",
  encoding: "ws2s.assistant.encoding.v1",
};
const STATE_LABELS = {
  awaiting: ["等待连接", "waiting"],
  active: ["已连接", "ok"],
  grace: ["宽限中", "grace"],
  closed: ["已关闭", "error"],
  error: ["错误", "error"],
};

const state = {
  session: null,
  paused: false,
  displayHex: false,
  showTs: true,
  autoscroll: true,
  encoding: "utf-8",
  decoder: new TextDecoder("utf-8", { fatal: false }),
  rateWindow: [],
  history: [],
  commands: [],
  timer: null,
  timerSent: 0,
  timerCount: 0,
};

function loadJSON(key, fallback) {
  try {
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) : fallback;
  } catch {
    return fallback;
  }
}

function saveJSON(key, value) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    // localStorage 不可用时静默降级
  }
}

function setOpenResult(text, link = null) {
  const box = $("#assistant-open-result");
  box.textContent = text || "";
  if (link) {
    box.append(" ");
    const a = document.createElement("a");
    a.href = link.href;
    a.target = "_blank";
    a.rel = "noopener";
    a.textContent = link.text;
    box.appendChild(a);
  }
}

function setSendStatus(text, kind = "") {
  const box = $("#assistant-send-status");
  box.textContent = text || "";
  box.className = "status" + (kind ? " " + kind : "");
}

function showError(err) {
  const code = err?.code ? `${err.code}: ` : "";
  const message = err?.message || String(err);
  $("#assistant-last-error").textContent = code + message;
  if (err?.code === "PORT_BUSY") {
    setOpenResult(`${code}${message}。请先在管理页关闭占用会话，或关闭其他串口程序。`, {
      href: "/",
      text: "打开管理页",
    });
  } else {
    setOpenResult(code + message);
  }
}

function sessionBadge(session) {
  if (!session) return ["未连接", "waiting"];
  return STATE_LABELS[session.state] || [session.state || "-", "waiting"];
}

function canSend() {
  const session = state.session;
  return !!session && session.ws && session.ws.readyState === WebSocket.OPEN;
}

function updateButtons(client) {
  const session = state.session;
  $("#assistant-open").disabled = !client.connected || !!session;
  $("#assistant-close").disabled = !session;
  const canReconnect = !!session && client.connected && ["grace", "closed"].includes(session.state);
  $("#assistant-reconnect").disabled = !canReconnect;
  $("#assistant-send-btn").disabled = !canSend();
  $("#assistant-timer-start").disabled = !canSend() || !!state.timer;
  $("#assistant-timer-stop").disabled = !state.timer;
}

function updateStats() {
  const session = state.session;
  let rx = 0;
  let tx = 0;
  let drop = 0;
  if (session) {
    for (const frame of session.frames) {
      if (frame.dir === "RX") rx += frame.bytes.length;
      else tx += frame.bytes.length;
    }
    drop = session.counters?.rxDiscardedBytes ?? 0;
  }
  $("#assistant-counters").textContent = `RX ${rx} / TX ${tx} / DROP ${drop}`;
  $("#assistant-rx").textContent = String(rx);
  $("#assistant-tx").textContent = String(tx);
  $("#assistant-drop").textContent = String(drop);
}

function updateRate() {
  const cutoff = Date.now() - 1000;
  state.rateWindow = state.rateWindow.filter((entry) => entry.ts >= cutoff);
  const bytes = state.rateWindow.reduce((sum, entry) => sum + entry.n, 0);
  $("#assistant-rate").textContent = String(bytes);
}

function stopTimer() {
  if (state.timer) {
    clearInterval(state.timer);
    state.timer = null;
  }
  updateTimerStatus();
}

function updateTimerStatus() {
  const running = !!state.timer;
  const start = $("#assistant-timer-start");
  const stop = $("#assistant-timer-stop");
  const status = $("#assistant-timer-status");
  if (running) {
    status.textContent = `定时运行中 · 已发送 ${state.timerSent}${state.timerCount > 0 ? ` / ${state.timerCount}` : ""} 次`;
  } else if (state.timerSent > 0) {
    status.textContent = `定时已停止 · 共发送 ${state.timerSent} 次`;
  } else {
    status.textContent = "定时未运行";
  }
  start.disabled = !canSend() || running;
  stop.disabled = !running;
}

function startTimer() {
  if (!canSend()) {
    setSendStatus("数据 WS 未连接", "error");
    return;
  }
  const interval = Number($("#assistant-timer-interval").value);
  const count = Number($("#assistant-timer-count").value);
  if (!Number.isFinite(interval) || interval < 10) {
    setSendStatus("定时间隔至少 10ms", "error");
    return;
  }
  if (!Number.isFinite(count) || count < 0) {
    setSendStatus("定时次数必须是非负整数", "error");
    return;
  }
  stopTimer();
  state.timerSent = 0;
  state.timerCount = Math.floor(count);
  state.timer = setInterval(() => {
    if (!canSend()) {
      stopTimer();
      return;
    }
    if (!sendCurrent(true)) {
      stopTimer();
      return;
    }
    state.timerSent += 1;
    updateTimerStatus();
    if (state.timerCount > 0 && state.timerSent >= state.timerCount) {
      stopTimer();
    }
  }, Math.floor(interval));
  updateTimerStatus();
}

function renderSessionState(client) {
  const session = state.session;
  const [label, cls] = sessionBadge(session);
  const badge = $("#assistant-state");
  badge.textContent = label;
  badge.className = "badge " + cls;
  if (session) {
    const err = session.lastError;
    if (err) {
      $("#assistant-last-error").textContent = `${err.code}: ${err.message}`;
    } else if (session.closeCode != null && session.state !== "active") {
      $("#assistant-last-error").textContent =
        `close ${session.closeCode}${session.closeReason ? " " + session.closeReason : ""}`;
    }
    if (!["active", "awaiting"].includes(session.state)) {
      stopTimer();
    }
  } else {
    $("#assistant-last-error").textContent = "";
    stopTimer();
  }
  updateStats();
  updateButtons(client);
}

function decodeFrame(frame) {
  try {
    return state.decoder.decode(frame.bytes);
  } catch {
    return bytesToAscii(frame.bytes);
  }
}

function lineElement(frame) {
  const line = document.createElement("div");
  line.className = "assistant-line";
  const ts = document.createElement("span");
  ts.className = "ts";
  ts.textContent = state.showTs ? fmtTime(frame.ts) : "";
  line.appendChild(ts);
  const dir = document.createElement("span");
  dir.className = "dir " + (frame.dir === "RX" ? "rx" : "tx");
  dir.textContent = frame.dir;
  line.appendChild(dir);
  const body = document.createElement("span");
  body.className = "body";
  body.textContent = state.displayHex ? bytesToHex(frame.bytes) : decodeFrame(frame);
  line.appendChild(body);
  return line;
}

function updateBufferNote() {
  const session = state.session;
  const note = $("#assistant-buffer-note");
  if (!session) {
    note.hidden = true;
    return;
  }
  const full = session.frames.length >= 5000 || session.frameBytes >= 4 * 1024 * 1024;
  note.hidden = !full;
  note.textContent = full ? "客户端缓冲已满（5000 帧或 4 MiB），已丢弃最旧数据" : "";
}

function renderAll() {
  const box = $("#assistant-log");
  box.textContent = "";
  const session = state.session;
  if (!session || !session.frames.length) {
    const empty = document.createElement("div");
    empty.className = "assistant-empty";
    empty.textContent = session ? "暂无收发" : "未打开会话";
    box.appendChild(empty);
    updateBufferNote();
    return;
  }
  const frag = document.createDocumentFragment();
  for (const frame of session.frames) frag.appendChild(lineElement(frame));
  box.appendChild(frag);
  if (state.autoscroll) box.scrollTop = box.scrollHeight;
  updateBufferNote();
}

function appendFrame(frame) {
  state.rateWindow.push({ ts: frame.ts, n: frame.bytes.length });
  if (!state.paused) {
    const box = $("#assistant-log");
    box.querySelector(".assistant-empty")?.remove();
    box.appendChild(lineElement(frame));
    while (box.childElementCount > 5000) box.firstElementChild?.remove();
    if (state.autoscroll) box.scrollTop = box.scrollHeight;
  }
  updateStats();
  updateBufferNote();
}

function readParams() {
  const path = $("#assistant-port").value.trim();
  if (!path) throw new Error("请选择或输入串口");
  const baudRate = Number($("#assistant-baud").value);
  if (!Number.isInteger(baudRate) || baudRate <= 0) throw new Error("波特率必须是正整数");
  const params = {
    path,
    baudRate,
    dataBits: Number($("#assistant-databits").value),
    parity: $("#assistant-parity").value,
    stopBits: Number($("#assistant-stopbits").value),
    flowControl: $("#assistant-flowcontrol").value,
  };
  if ($("#assistant-dtr").value !== "default") params.dtr = $("#assistant-dtr").value === "true";
  if ($("#assistant-rts").value !== "default") params.rts = $("#assistant-rts").value === "true";
  return params;
}

function renderPorts(ports) {
  const list = $("#assistant-port-list");
  list.textContent = "";
  for (const port of ports) {
    const option = document.createElement("option");
    option.value = port.path;
    list.appendChild(option);
  }
  if (!$("#assistant-port").value && ports[0]) {
    $("#assistant-port").value = ports[0].path;
  }
}

async function refreshPorts(client) {
  if (client.ports.length) {
    renderPorts(client.ports);
  }
  if (!client.connected) return;
  await client.refreshPorts();
  renderPorts(client.ports);
}

function savePrefs(params) {
  saveJSON(LS.params, {
    path: params.path,
    baudRate: params.baudRate,
    dataBits: params.dataBits,
    parity: params.parity,
    stopBits: params.stopBits,
    flowControl: params.flowControl,
    dtr: $("#assistant-dtr").value,
    rts: $("#assistant-rts").value,
  });
}

function loadPrefs() {
  const prefs = loadJSON(LS.params, null);
  if (!prefs || typeof prefs !== "object") return;
  if (prefs.path) $("#assistant-port").value = prefs.path;
  if (prefs.baudRate) $("#assistant-baud").value = prefs.baudRate;
  if (prefs.dataBits) $("#assistant-databits").value = String(prefs.dataBits);
  if (prefs.parity) $("#assistant-parity").value = prefs.parity;
  if (prefs.stopBits) $("#assistant-stopbits").value = String(prefs.stopBits);
  if (prefs.flowControl) $("#assistant-flowcontrol").value = prefs.flowControl;
  if (prefs.dtr) $("#assistant-dtr").value = prefs.dtr;
  if (prefs.rts) $("#assistant-rts").value = prefs.rts;
}

function initEncoding() {
  const saved = loadJSON(LS.encoding, "utf-8");
  state.encoding = saved === "gbk" ? "gbk" : "utf-8";
  $("#assistant-encoding").value = state.encoding;
  state.decoder = new TextDecoder(state.encoding === "gbk" ? "gbk" : "utf-8", { fatal: false });
}

function renderHistory() {
  const select = $("#assistant-history");
  select.textContent = "";
  const placeholder = document.createElement("option");
  placeholder.value = "";
  placeholder.textContent = state.history.length ? "选择历史…" : "暂无历史";
  select.appendChild(placeholder);
  state.history.forEach((item, index) => {
    const option = document.createElement("option");
    option.value = String(index);
    const preview = item.payload.length > 40 ? item.payload.slice(0, 40) + "…" : item.payload;
    option.textContent = `[${item.mode.toUpperCase()}] ${preview}`;
    select.appendChild(option);
  });
}

function rememberHistory(payload, mode, eol) {
  if (!payload) return;
  const item = { payload, mode, eol };
  state.history = [item, ...state.history.filter((x) => !(x.payload === payload && x.mode === mode && x.eol === eol))].slice(0, 20);
  saveJSON(LS.history, state.history);
  renderHistory();
}

function loadHistory() {
  const stored = loadJSON(LS.history, []);
  state.history = Array.isArray(stored)
    ? stored.filter((x) => x && typeof x.payload === "string" && (x.mode === "text" || x.mode === "hex"))
    : [];
  renderHistory();
}

function loadCommands() {
  const stored = loadJSON(LS.commands, []);
  state.commands = Array.isArray(stored)
    ? stored.filter((x) => x && typeof x.payload === "string" && (x.mode === "text" || x.mode === "hex"))
    : [];
  renderCommands();
}

function renderCommands() {
  const list = $("#assistant-cmd-list");
  list.textContent = "";
  if (!state.commands.length) {
    const empty = document.createElement("div");
    empty.className = "muted";
    empty.textContent = "暂无快捷指令";
    list.appendChild(empty);
    return;
  }
  state.commands.forEach((command, index) => {
    const row = document.createElement("div");
    row.className = "assistant-command-row";
    const name = document.createElement("span");
    name.className = "cmd-name";
    name.textContent = command.name;
    const meta = document.createElement("span");
    meta.className = "cmd-meta";
    meta.textContent = `[${command.mode.toUpperCase()}] ${command.eol === "none" ? "" : command.eol.toUpperCase()}`.trim();
    const payload = document.createElement("span");
    payload.className = "cmd-payload";
    payload.textContent = command.payload;
    payload.title = command.payload;
    const actions = document.createElement("span");
    actions.className = "cmd-actions";
    const load = document.createElement("button");
    load.type = "button";
    load.className = "small";
    load.textContent = "载入";
    load.addEventListener("click", () => loadCommand(index));
    const send = document.createElement("button");
    send.type = "button";
    send.className = "small";
    send.textContent = "发送";
    send.addEventListener("click", () => sendCommand(index));
    const remove = document.createElement("button");
    remove.type = "button";
    remove.className = "small";
    remove.textContent = "删除";
    remove.addEventListener("click", () => {
      state.commands.splice(index, 1);
      saveJSON(LS.commands, state.commands);
      renderCommands();
    });
    actions.append(load, send, remove);
    row.append(name, meta, payload, actions);
    list.appendChild(row);
  });
}

function saveCommand() {
  const payload = $("#assistant-cmd-payload").value;
  if (!payload.trim()) {
    setSendStatus("快捷指令内容为空", "error");
    return;
  }
  const name = $("#assistant-cmd-name").value.trim() || `指令 ${state.commands.length + 1}`;
  state.commands.push({
    name,
    payload,
    mode: $("#assistant-cmd-mode").value,
    eol: $("#assistant-cmd-eol").value,
  });
  saveJSON(LS.commands, state.commands);
  renderCommands();
  $("#assistant-cmd-name").value = "";
  $("#assistant-cmd-payload").value = "";
  setSendStatus(`已保存快捷指令「${name}」`, "ok");
}

function loadCommand(index) {
  const command = state.commands[index];
  if (!command) return;
  $("#assistant-send-input").value = command.payload;
  $("#assistant-send-mode").value = command.mode;
  $("#assistant-send-eol").value = command.eol;
  $("#assistant-send-eol").disabled = command.mode === "hex";
}

function sendCommand(index) {
  const command = state.commands[index];
  if (!command) return;
  if (!canSend()) {
    setSendStatus("数据 WS 未连接", "error");
    return;
  }
  try {
    const bytes = buildPayload(command.payload, command.mode, command.eol);
    const sent = sendBytes(bytes);
    rememberHistory(command.payload, command.mode, command.eol);
    setSendStatus(`已发送 ${sent} 字节`, "ok");
  } catch (err) {
    setSendStatus(err.message, "error");
  }
}

function buildPayload(payload, mode, eol) {
  if (mode === "hex") {
    return hexToBytes(payload);
  }
  const text = payload + (EOL[eol] || "");
  return new TextEncoder().encode(text);
}

function crc16Modbus(bytes) {
  let crc = 0xffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let i = 0; i < 8; i++) {
      crc = crc & 1 ? (crc >> 1) ^ 0xa001 : crc >> 1;
    }
  }
  return crc & 0xffff;
}

function withChecksum(bytes, type) {
  if (type === "none") return bytes;
  let extra = 0;
  if (type === "sum8" || type === "xor8") extra = 1;
  else if (type === "crc16") extra = 2;
  if (!extra) return bytes;
  const out = new Uint8Array(bytes.length + extra);
  out.set(bytes);
  if (type === "sum8") {
    out[out.length - 1] = bytes.reduce((sum, byte) => (sum + byte) & 0xff, 0);
  } else if (type === "xor8") {
    out[out.length - 1] = bytes.reduce((xor, byte) => xor ^ byte, 0);
  } else if (type === "crc16") {
    const crc = crc16Modbus(bytes);
    out[out.length - 2] = crc & 0xff;
    out[out.length - 1] = (crc >> 8) & 0xff;
  }
  return out;
}

function sendBytes(bytes) {
  const checksum = $("#assistant-checksum").value;
  const finalBytes = withChecksum(bytes, checksum);
  state.session.sendBytes(finalBytes);
  return finalBytes.length;
}

function sendCurrent(silent = false) {
  if (!canSend()) {
    if (!silent) setSendStatus("数据 WS 未连接", "error");
    return false;
  }
  const mode = $("#assistant-send-mode").value;
  const raw = $("#assistant-send-input").value;
  const eol = $("#assistant-send-eol").value;
  try {
    const bytes = buildPayload(raw, mode, eol);
    if (!bytes.length) return false;
    const sent = sendBytes(bytes);
    if (!silent) {
      rememberHistory(raw, mode, eol);
      setSendStatus(`已发送 ${sent} 字节`, "ok");
    }
    return true;
  } catch (err) {
    if (!silent) setSendStatus(err.message, "error");
    return false;
  }
}

function exportLog() {
  const session = state.session;
  if (!session || !session.frames.length) {
    setSendStatus("当前没有可导出的数据", "error");
    return;
  }
  const lines = session.frames.map(
    (frame) => `${fmtTime(frame.ts)}\t${frame.dir}\t${bytesToHex(frame.bytes)}\t${bytesToAscii(frame.bytes)}`,
  );
  download(`ws2s-assistant-${Date.now()}.txt`, lines.join("\n"));
}

async function openSerial(client) {
  if (state.session) return;
  let params;
  try {
    params = readParams();
  } catch (err) {
    setOpenResult(err.message);
    return;
  }
  $("#assistant-open").disabled = true;
  setOpenResult("打开中…");
  let data;
  try {
    data = await client.request("session.open", params);
  } catch (err) {
    showError(err);
    updateButtons(client);
    return;
  }
  const session = new SerialSession("A", { ...data, state: "awaiting" }, params, client, {
    onFrame: (frame) => appendFrame(frame),
    onState: () => renderSessionState(client),
    onLog: (level, message) => {
      if (level === "error" || level === "warn") {
        $("#assistant-last-error").textContent = message;
      }
    },
  });
  state.session = session;
  state.rateWindow = [];
  savePrefs(params);
  renderSessionState(client);
  try {
    await session.connectData();
    setOpenResult(`已打开 ${params.path}`);
  } catch (err) {
    showError(err);
  }
  renderSessionState(client);
}

async function closeSerial(client) {
  if (!state.session) return;
  $("#assistant-close").disabled = true;
  stopTimer();
  const session = state.session;
  try {
    await session.closeSession(false);
  } catch (err) {
    showError(err);
  }
  state.session = null;
  state.rateWindow = [];
  setOpenResult(`已关闭 ${session.params?.path || ""}`);
  setSendStatus("");
  renderAll();
  updateRate();
  renderSessionState(client);
}

async function reconnectSerial(client) {
  if (!state.session) return;
  $("#assistant-reconnect").disabled = true;
  try {
    await state.session.reconnectOld();
    setOpenResult("重新连接成功");
  } catch (err) {
    showError(err);
  }
  renderSessionState(client);
}

function bindUI(client) {
  $("#assistant-open").addEventListener("click", () => openSerial(client));
  $("#assistant-close").addEventListener("click", () => closeSerial(client));
  $("#assistant-reconnect").addEventListener("click", () => reconnectSerial(client));
  $("#assistant-send-btn").addEventListener("click", () => sendCurrent());
  $("#assistant-send-input").addEventListener("keydown", (event) => {
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      sendCurrent();
    }
  });
  $("#assistant-send-mode").addEventListener("change", () => {
    $("#assistant-send-eol").disabled = $("#assistant-send-mode").value === "hex";
  });
  $("#assistant-pause").addEventListener("click", () => {
    state.paused = !state.paused;
    $("#assistant-pause").textContent = state.paused ? "继续" : "暂停";
    $("#assistant-pause").classList.toggle("active", state.paused);
    if (!state.paused) renderAll();
  });
  $("#assistant-clear").addEventListener("click", () => {
    if (state.session) {
      state.session.frames = [];
      state.session.frameBytes = 0;
    }
    state.rateWindow = [];
    renderAll();
    updateStats();
    updateRate();
  });
  $("#assistant-export").addEventListener("click", exportLog);
  $("#assistant-autoscroll").addEventListener("change", () => {
    state.autoscroll = $("#assistant-autoscroll").checked;
  });
  $("#assistant-ts").addEventListener("change", () => {
    state.showTs = $("#assistant-ts").checked;
    renderAll();
  });
  $("#assistant-format").addEventListener("change", () => {
    state.displayHex = $("#assistant-format").value === "hex";
    renderAll();
  });
  $("#assistant-encoding").addEventListener("change", () => {
    state.encoding = $("#assistant-encoding").value === "gbk" ? "gbk" : "utf-8";
    state.decoder = new TextDecoder(state.encoding === "gbk" ? "gbk" : "utf-8", { fatal: false });
    saveJSON(LS.encoding, state.encoding);
    renderAll();
  });
  $("#assistant-history").addEventListener("change", () => {
    const select = $("#assistant-history");
    const index = Number(select.value);
    if (select.value !== "" && Number.isInteger(index) && state.history[index]) {
      const item = state.history[index];
      $("#assistant-send-input").value = item.payload;
      $("#assistant-send-mode").value = item.mode;
      $("#assistant-send-eol").value = item.eol;
      $("#assistant-send-eol").disabled = item.mode === "hex";
    }
    select.value = "";
  });
  $("#assistant-timer-start").addEventListener("click", startTimer);
  $("#assistant-timer-stop").addEventListener("click", stopTimer);
  $("#assistant-cmd-save").addEventListener("click", saveCommand);
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) stopTimer();
  });
}

async function main() {
  const config = await getConfig();
  const client = new ManagementClient(config.managementPort);
  loadPrefs();
  initEncoding();
  loadHistory();
  loadCommands();
  bindUI(client);

  const queryPort = new URLSearchParams(location.search).get("port");
  if (queryPort) $("#assistant-port").value = queryPort;

  client.onConnection((connected) => {
    if (connected) {
      setOpenResult("管理 WS 已连接");
      refreshPorts(client).catch(() => {});
    } else {
      setOpenResult("管理 WS 已断开，正在重连；已连接的数据 WS 仍可收发");
    }
    renderSessionState(client);
  });
  client.onSync(() => {
    renderPorts(client.ports);
    renderSessionState(client);
  });
  client.onEvent((name, data) => {
    if (name === "ports.changed") {
      refreshPorts(client).catch(() => {});
      return;
    }
    if (name === "session.state" && state.session && data.sessionId === state.session.sessionId) {
      state.session.updateInfo(data);
      if (data.state === "closed") {
        setSendStatus("会话已关闭", "error");
      }
    }
  });

  setInterval(updateRate, 500);
  client.connect();
  renderSessionState(client);
  renderAll();
  updateRate();
  updateButtons(client);
}

main().catch((err) => {
  const badge = $("#assistant-state");
  if (badge) {
    badge.textContent = "初始化失败";
    badge.className = "badge error";
  }
  console.error(err);
});
