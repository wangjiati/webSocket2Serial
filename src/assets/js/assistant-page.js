// 串口助手页：自己打开/关闭会话，通过数据 WS 收发，不依赖 debug.frame。

import { getConfig, ManagementClient } from "./api.js";
import { SerialSession } from "./serial.js";
import { fmtTime, bytesToHex, bytesToAscii, hexToBytes, download } from "./format.js";

const $ = (selector) => document.querySelector(selector);

const EOL = { none: "", crlf: "\r\n", lf: "\n", cr: "\r" };
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
};

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

function updateButtons(client) {
  const session = state.session;
  $("#assistant-open").disabled = !client.connected || !!session;
  $("#assistant-close").disabled = !session;
  const canReconnect = !!session && client.connected && ["grace", "closed"].includes(session.state);
  $("#assistant-reconnect").disabled = !canReconnect;
  const canSend = !!session && session.ws && session.ws.readyState === WebSocket.OPEN;
  $("#assistant-send-btn").disabled = !canSend;
}

function updateCounters() {
  const session = state.session;
  if (!session) {
    $("#assistant-counters").textContent = "RX 0 / TX 0 / DROP 0";
    return;
  }
  let rx = 0;
  let tx = 0;
  for (const frame of session.frames) {
    if (frame.dir === "RX") rx += frame.bytes.length;
    else tx += frame.bytes.length;
  }
  const drop = session.counters?.rxDiscardedBytes ?? 0;
  $("#assistant-counters").textContent = `RX ${rx} / TX ${tx} / DROP ${drop}`;
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
      $("#assistant-last-error").textContent = `close ${session.closeCode}${session.closeReason ? " " + session.closeReason : ""}`;
    }
  } else {
    $("#assistant-last-error").textContent = "";
  }
  updateCounters();
  updateButtons(client);
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
  body.textContent = state.displayHex ? bytesToHex(frame.bytes) : bytesToAscii(frame.bytes);
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
  if (!state.paused) {
    const box = $("#assistant-log");
    box.querySelector(".assistant-empty")?.remove();
    box.appendChild(lineElement(frame));
    while (box.childElementCount > 5000) box.firstElementChild?.remove();
    if (state.autoscroll) box.scrollTop = box.scrollHeight;
  }
  updateCounters();
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
  const session = state.session;
  try {
    await session.closeSession(false);
  } catch (err) {
    showError(err);
  }
  state.session = null;
  setOpenResult(`已关闭 ${session.params?.path || ""}`);
  setSendStatus("");
  renderAll();
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

function sendCurrent() {
  const session = state.session;
  if (!session || !session.ws || session.ws.readyState !== WebSocket.OPEN) {
    setSendStatus("数据 WS 未连接", "error");
    return;
  }
  const mode = $("#assistant-send-mode").value;
  const raw = $("#assistant-send-input").value;
  let bytes;
  try {
    if (mode === "hex") {
      bytes = hexToBytes(raw);
    } else {
      const eol = EOL[$("#assistant-send-eol").value] || "";
      if (!raw.length && !eol.length) return;
      bytes = new TextEncoder().encode(raw + eol);
    }
    session.sendBytes(bytes);
    setSendStatus(`已发送 ${bytes.length} 字节`, "ok");
  } catch (err) {
    setSendStatus(err.message, "error");
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

function bindUI(client) {
  $("#assistant-open").addEventListener("click", () => openSerial(client));
  $("#assistant-close").addEventListener("click", () => closeSerial(client));
  $("#assistant-reconnect").addEventListener("click", () => reconnectSerial(client));
  $("#assistant-send-btn").addEventListener("click", sendCurrent);
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
    renderAll();
    updateCounters();
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
}

async function main() {
  const config = await getConfig();
  const client = new ManagementClient(config.managementPort);
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

  client.connect();
  renderSessionState(client);
  renderAll();
}

main().catch((err) => {
  const badge = $("#assistant-state");
  if (badge) {
    badge.textContent = "初始化失败";
    badge.className = "badge error";
  }
  console.error(err);
});
