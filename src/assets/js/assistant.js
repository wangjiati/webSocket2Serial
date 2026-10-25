// 串口助手：会话选择、debug.subscribe（replay）、序号/时间/方向/HEX/ASCII、
// 暂停/清空/自动滚动、seq 缺口提示、客户端缓冲上限（5000 帧或 4 MiB）、导出、
// 发送（HEX/文本；默认禁用，勾选“允许写入”并二次确认）。不自行打开串口。

const MAX_FRAMES = 5000;
const MAX_BYTES = 4 * 1024 * 1024; // 4 MiB
const DISPLAY_BYTES = 128; // 单帧在界面展示的最大字节数（导出用完整数据）
const WRITE_MAX_BYTES = 64 * 1024; // 与服务端 session.write 单次上限一致

const STATE_LABELS = {
  awaiting: "等待连接",
  active: "已连接",
  grace: "宽限中",
  closed: "已关闭",
};

const textEncoder = new TextEncoder();

export function createAssistant(root, client) {
  const select = root.querySelector("#session-select");
  const logEl = root.querySelector("#log");
  const pauseBtn = root.querySelector("#pause");
  const clearBtn = root.querySelector("#clear");
  const autoscrollEl = root.querySelector("#autoscroll");
  const exportBtn = root.querySelector("#export");
  const bufferNote = root.querySelector("#buffer-note");
  const sendInput = root.querySelector("#send-input");
  const sendMode = root.querySelectorAll('input[name="send-mode"]');
  const allowWrite = root.querySelector("#allow-write");
  const sendBtn = root.querySelector("#send-btn");
  const sendStatus = root.querySelector("#send-status");

  const frames = [];
  let bufferBytes = 0;
  let dropped = false;
  let paused = false;
  let renderedCount = 0;
  const lastSeq = new Map();
  let currentSelection = null;
  let selectedId = null;
  let wasConnected = false;

  client.onSync(() => updateSessionSelect());
  client.onConnection((connected) => {
    if (connected && !wasConnected) {
      // 首次连接或重连：清空本地缓冲，由重连后的回放重新填充，避免重复帧与假缺口。
      resetLog();
    }
    wasConnected = connected;
    updateSessionSelect();
  });
  client.onEvent((name, data) => {
    if (name === "debug.frame") {
      onFrame(data);
    }
  });

  updateSessionSelect();

  select.addEventListener("change", applySelection);

  pauseBtn.addEventListener("click", () => {
    paused = !paused;
    pauseBtn.textContent = paused ? "继续" : "暂停";
    pauseBtn.classList.toggle("active", paused);
    if (!paused) {
      renderPending();
    }
  });

  clearBtn.addEventListener("click", resetLog);

  autoscrollEl.addEventListener("change", () => {
    if (autoscrollEl.checked && !paused) {
      logEl.scrollTop = logEl.scrollHeight;
    }
  });

  allowWrite.addEventListener("change", updateSendEnabled);

  sendBtn.addEventListener("click", send);

  exportBtn.addEventListener("click", exportBuffer);

  function updateSessionSelect() {
    const sessions = client.sessions || [];
    const previous = select.value;
    const frag = document.createDocumentFragment();
    const allOpt = document.createElement("option");
    allOpt.value = "all";
    allOpt.textContent = "全部会话";
    frag.appendChild(allOpt);
    for (const s of sessions) {
      const opt = document.createElement("option");
      opt.value = s.sessionId;
      opt.textContent = `${shortId(s.sessionId)} · ${s.port}（${STATE_LABELS[s.state] || s.state}）`;
      frag.appendChild(opt);
    }
    select.textContent = "";
    select.appendChild(frag);
    if (previous && Array.from(select.options).some((o) => o.value === previous)) {
      select.value = previous;
    } else {
      select.value = "all";
    }
    applySelection();
  }

  function applySelection() {
    const value = select.value;
    if (value === currentSelection) {
      return;
    }
    currentSelection = value;
    const isAll = value === "all";
    selectedId = isAll ? null : value;
    resetLog();
    const sessionIds = isAll ? "all" : [value];
    const p = client.subscribeDebug(sessionIds, true);
    if (p) {
      p.catch((err) => showError("订阅调试失败：" + err.message));
    }
    updateSendEnabled();
  }

  function resetLog() {
    frames.length = 0;
    bufferBytes = 0;
    dropped = false;
    renderedCount = 0;
    lastSeq.clear();
    logEl.textContent = "";
    bufferNote.hidden = true;
  }

  function onFrame(data) {
    let bytes;
    try {
      bytes = decodeB64(data.dataB64 || "");
    } catch (err) {
      bytes = new Uint8Array(0);
    }
    const frame = {
      sessionId: data.sessionId || "",
      direction: data.direction || "?",
      seq: typeof data.seq === "number" ? data.seq : 0,
      tsMs: typeof data.tsMs === "number" ? data.tsMs : 0,
      replay: !!data.replay,
      bytes,
    };
    const prev = lastSeq.get(frame.sessionId);
    if (typeof prev === "number" && frame.seq > prev + 1) {
      frame.gap = { from: prev + 1, to: frame.seq - 1 };
    }
    lastSeq.set(frame.sessionId, frame.seq);
    appendFrame(frame);
  }

  function appendFrame(frame) {
    frames.push(frame);
    bufferBytes += frame.bytes.length;
    while (frames.length > MAX_FRAMES || bufferBytes > MAX_BYTES) {
      const removed = frames.shift();
      bufferBytes -= removed.bytes.length;
      dropped = true;
      if (renderedCount > 0) {
        logEl.removeChild(logEl.firstChild);
        renderedCount -= 1;
      }
    }
    if (dropped) {
      bufferNote.hidden = false;
      bufferNote.textContent = "客户端缓冲已满（5000 帧或 4 MiB），已丢弃最旧数据";
    }
    if (!paused) {
      renderPending();
    }
  }

  function renderPending() {
    if (paused) {
      return;
    }
    const frag = document.createDocumentFragment();
    while (renderedCount < frames.length) {
      frag.appendChild(renderFrame(frames[renderedCount]));
      renderedCount += 1;
    }
    logEl.appendChild(frag);
    if (autoscrollEl.checked) {
      logEl.scrollTop = logEl.scrollHeight;
    }
  }

  function renderFrame(frame) {
    const row = document.createElement("div");
    row.className = "frame" + (frame.replay ? " replay" : "");
    if (frame.gap) {
      const gap = document.createElement("div");
      gap.className = "gap-note";
      gap.textContent = `⚠ seq 缺口 ${frame.gap.from}–${frame.gap.to}（${shortId(frame.sessionId)} · 写队列满已丢弃）`;
      row.appendChild(gap);
    }
    const line = document.createElement("div");
    line.className = "frame-line " + (frame.direction === "rx" ? "rx" : "tx");
    line.appendChild(span("seq", String(frame.seq)));
    line.appendChild(span("ts", formatTs(frame.tsMs)));
    line.appendChild(span("dir", frame.direction.toUpperCase()));
    if (currentSelection === "all") {
      line.appendChild(span("sid", shortId(frame.sessionId)));
    }
    line.appendChild(span("hex", toHex(frame.bytes, true)));
    line.appendChild(span("ascii", toAscii(frame.bytes, true)));
    row.appendChild(line);
    return row;
  }

  function send() {
    if (selectedId == null) {
      showError("请选择具体会话（“全部会话”不可写入）");
      return;
    }
    if (!allowWrite.checked) {
      showError("请先勾选“允许写入”");
      return;
    }
    const raw = sendInput.value;
    let bytes;
    try {
      bytes = currentMode() === "hex" ? hexToBytes(raw) : textToBytes(raw);
    } catch (err) {
      showError(err.message);
      return;
    }
    if (bytes.length === 0) {
      showError("发送内容为空");
      return;
    }
    if (bytes.length > WRITE_MAX_BYTES) {
      showError("调试写单次上限 64 KiB");
      return;
    }
    const ok = window.confirm(
      `确认向串口 ${shortId(selectedId)} 写入 ${bytes.length} 字节？` +
        "此写入与生产网页共用同一串口，会实际发送到设备。",
    );
    if (!ok) {
      return;
    }
    sendBtn.disabled = true;
    client
      .request("session.write", { sessionId: selectedId, dataB64: encodeB64(bytes) })
      .then((data) => showInfo(`已写入 ${data.written} 字节`))
      .catch((err) => showError("写入失败：" + err.message))
      .finally(() => updateSendEnabled());
  }

  function exportBuffer() {
    if (frames.length === 0) {
      window.alert("当前缓冲为空，无可导出内容");
      return;
    }
    const lines = frames.map((f) => {
      let line = `[${f.seq}] ${formatTs(f.tsMs)} ${f.direction.toUpperCase()}`;
      if (currentSelection === "all") {
        line += ` ${shortId(f.sessionId)}`;
      }
      line += `  ${toHex(f.bytes, false)}  ${toAscii(f.bytes, false)}`;
      return line;
    });
    const text = lines.join("\n") + "\n";
    const blob = new Blob([text], { type: "text/plain;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `ws2s-debug-${timestamp()}.txt`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  function updateSendEnabled() {
    sendInput.disabled = !allowWrite.checked;
    sendBtn.disabled = !(allowWrite.checked && selectedId != null);
  }

  function currentMode() {
    for (const el of sendMode) {
      if (el.checked) {
        return el.value;
      }
    }
    return "text";
  }

  function showError(msg) {
    sendStatus.textContent = msg;
    sendStatus.className = "status error";
  }

  function showInfo(msg) {
    sendStatus.textContent = msg;
    sendStatus.className = "status ok";
  }
}

function span(cls, text) {
  const el = document.createElement("span");
  el.className = cls;
  el.textContent = text;
  return el;
}

function shortId(id) {
  return id && id.length > 8 ? id.slice(0, 8) : id;
}

function formatTs(ms) {
  const d = new Date(ms);
  const pad = (n, w) => String(n).padStart(w, "0");
  return `${pad(d.getHours(), 2)}:${pad(d.getMinutes(), 2)}:${pad(d.getSeconds(), 2)}.${pad(d.getMilliseconds(), 3)}`;
}

function decodeB64(b64) {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i += 1) {
    bytes[i] = bin.charCodeAt(i);
  }
  return bytes;
}

function encodeB64(bytes) {
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    bin += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  }
  return btoa(bin);
}

function textToBytes(text) {
  return textEncoder.encode(text);
}

function hexToBytes(hex) {
  const s = hex.replace(/[\s,;:.|-]/g, "");
  if (s.length === 0) {
    return new Uint8Array(0);
  }
  if (s.length % 2 !== 0) {
    throw new Error("HEX 长度必须为偶数");
  }
  if (!/^[0-9a-fA-F]+$/.test(s)) {
    throw new Error("HEX 含非法字符");
  }
  const bytes = new Uint8Array(s.length / 2);
  for (let i = 0; i < bytes.length; i += 1) {
    bytes[i] = parseInt(s.slice(i * 2, i * 2 + 2), 16);
  }
  return bytes;
}

function toHex(bytes, truncate) {
  const view = truncate && bytes.length > DISPLAY_BYTES ? bytes.subarray(0, DISPLAY_BYTES) : bytes;
  let out = "";
  for (let i = 0; i < view.length; i += 1) {
    out += (i ? " " : "") + view[i].toString(16).toUpperCase().padStart(2, "0");
  }
  if (truncate && bytes.length > DISPLAY_BYTES) {
    out += ` …（共 ${bytes.length} 字节）`;
  }
  return out || "（空）";
}

function toAscii(bytes, truncate) {
  const view = truncate && bytes.length > DISPLAY_BYTES ? bytes.subarray(0, DISPLAY_BYTES) : bytes;
  let out = "";
  for (let i = 0; i < view.length; i += 1) {
    const b = view[i];
    out += b >= 0x20 && b <= 0x7e ? String.fromCharCode(b) : ".";
  }
  if (truncate && bytes.length > DISPLAY_BYTES) {
    out += "…";
  }
  return out;
}

function timestamp() {
  const d = new Date();
  const pad = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}`;
}
