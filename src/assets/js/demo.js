// 管理页「功能演示」模块；与 demo/websocket2serial-demo.html 保持功能同步。
export function createDemo(root, client) {
  const $ = (selector) => root.querySelector(selector);
  const $$ = (selector) => Array.from(root.querySelectorAll(selector));

  // ----------------------------- utilities -----------------------------
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  const now = () => Date.now();
  const shortId = (id) => (id ? String(id).slice(0, 8) : "-");
  const fmtTime = (ms) => {
    const d = new Date(ms);
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}:${String(d.getSeconds()).padStart(2, "0")}.${String(ms % 1000).padStart(3, "0")}`;
  };
  const bytesToHex = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join(" ");
  const bytesToAscii = (bytes) => Array.from(bytes, (b) => (b >= 32 && b < 127 ? String.fromCharCode(b) : ".")).join("");
  const bytesEqual = (a, b) => a.length === b.length && a.every((v, i) => v === b[i]);
  const textEncoder = new TextEncoder();
  const b64encodeBytes = (bytes) => {
    let s = "";
    for (let i = 0; i < bytes.length; i += 0x8000) {
      s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    }
    return btoa(s);
  };
  const b64decodeBytes = (b64) => {
    const s = atob(b64);
    const out = new Uint8Array(s.length);
    for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
    return out;
  };
  const hexToBytes = (text) => {
    const clean = String(text).replace(/[^0-9a-fA-F]/g, "");
    if (!clean.length) throw new Error("HEX 内容为空");
    if (clean.length % 2) throw new Error("HEX 长度必须为偶数");
    const out = new Uint8Array(clean.length / 2);
    for (let i = 0; i < out.length; i++) out[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
    return out;
  };
  const pattern = (n) => Uint8Array.from({ length: n }, (_, i) => (i * 37 + 11) & 0xff);
  // Chromium/Firefox 会拒绝这些端口的 WS 连接（ERR_UNSAFE_PORT）。数据面是动态端口，
  // 命中时重新打开会话换一个端口，避免演示/客户端无故握手失败。
  const BROWSER_BLOCKED_PORTS = new Set([
    1, 7, 9, 11, 13, 15, 17, 19, 20, 21, 22, 23, 25, 37, 42, 43, 53, 69, 77, 79, 87, 95,
    101, 102, 103, 104, 109, 110, 111, 113, 115, 117, 119, 123, 135, 137, 138, 139, 143,
    161, 179, 389, 427, 465, 512, 513, 514, 515, 526, 530, 531, 532, 540, 548, 554, 556,
    563, 587, 601, 636, 989, 990, 993, 995, 1719, 1720, 1723, 2049, 3659, 4045, 5060,
    5061, 6000, 6566, 6665, 6666, 6667, 6668, 6669, 6697, 10080,
  ]);
  const dataUrlPort = (url) => {
    try {
      return Number(new URL(url).port) || 0;
    } catch {
      return 0;
    }
  };
  const blockedDataUrl = (url) => BROWSER_BLOCKED_PORTS.has(dataUrlPort(url));
  const el = (tag, cls, text) => {
    const node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text != null) node.textContent = String(text);
    return node;
  };
  const download = (name, text, mime = "text/plain;charset=utf-8") => {
    const blob = new Blob([text], { type: mime });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = name;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };
  const stateClass = (s) => (["awaiting", "active", "grace", "closed", "error"].includes(s) ? s : "");

  // ----------------------------- state -----------------------------
  const state = {
    hello: null,
    ports: [],
    sessions: new Map(),
    slots: { A: null, B: null },
    adminEnabled: null,
    autostart: null,
    events: [],
    debug: {
      subscribed: false,
      all: true,
      ids: new Set(),
      frames: [],
      lastSeq: new Map(),
      gaps: 0,
    },
    closedCodes: new Map(),
    terminalPrefs: {
      A: { hex: true, ts: true, autoscroll: true, paused: false },
      B: { hex: true, ts: true, autoscroll: true, paused: false },
    },
    autoReconnect: false,
    pollBusy: false,
    runner: { running: false, steps: [] },
  };

  // ----------------------------- logging -----------------------------
  function logEvent(kind, data) {
    state.events.push({ ts: now(), kind, data });
    if (state.events.length > 1500) state.events.splice(0, state.events.length - 1500);
    renderEvents();
    if (kind === "error") {
      toast(typeof data === "string" ? data : data?.message || "操作失败", "err");
    }
  }

  function renderEvents() {
    const box = $("#demo-event-log");
    if (!box) return;
    const filter = ($("#demo-event-filter")?.value || "").trim().toLowerCase();
    box.textContent = "";
    const frag = document.createDocumentFragment();
    for (const ev of state.events) {
      let text;
      try {
        text = typeof ev.data === "string" ? ev.data : JSON.stringify(ev.data);
      } catch {
        text = String(ev.data);
      }
      if (filter && !(ev.kind + " " + text).toLowerCase().includes(filter)) continue;
      const line = el("div", "event-line");
      line.appendChild(el("span", "ev-ts", fmtTime(ev.ts)));
      line.appendChild(el("span", "ev-kind" + (ev.kind === "error" ? " err" : ev.kind === "ok" ? " ok" : ""), ev.kind));
      line.appendChild(el("span", "", text));
      frag.appendChild(line);
    }
    box.appendChild(frag);
    $("#demo-event-count").textContent = String(state.events.length);
  }

  function edgeLog(text) {
    const box = $("#demo-edge-log");
    if (!box) return;
    const line = el("div", "event-line");
    line.appendChild(el("span", "ev-ts", fmtTime(now())));
    line.appendChild(el("span", "ev-kind", "edge"));
    line.appendChild(el("span", "", text));
    box.appendChild(line);
    box.scrollTop = box.scrollHeight;
  }

  function toast(message, kind = "") {
    const box = $("#demo-banner");
    if (!box) return;
    box.textContent = message;
    box.className = "demo-banner" + (kind ? " " + kind : "");
    box.classList.remove("hidden");
    clearTimeout(toast.timer);
    toast.timer = setTimeout(() => box.classList.add("hidden"), 6000);
  }

  // ----------------------------- client wiring -----------------------------
  client.onConnection((connected) => {
    const pill = $("#demo-pill-mgmt");
    if (pill) {
      pill.textContent = connected ? "管理 WS：已连接" : "管理 WS：未连接";
      pill.className = "demo-pill " + (connected ? "ok" : "err");
    }
    if (connected) {
      logEvent("info", `管理 WS 已连接 127.0.0.1:${client.managementPort}`);
      refreshSessions().catch((err) => logEvent("error", "对齐会话失败: " + err.message));
    } else {
      logEvent("warn", "管理 WS 已断开，自动重连中");
    }
  });

  client.onEvent((name, data) => {
    if (name === "hello") {
      onHello(data);
      return;
    }
    if (name === "session.state") {
      const prev = state.sessions.get(data.sessionId) || {};
      const next = { ...prev, ...data };
      state.sessions.set(data.sessionId, next);
      const controller = slotForSession(data.sessionId);
      if (controller) controller.updateInfo(next);
      if (data.state === "closed") {
        state.closedCodes.set(data.sessionId, data.closeCode);
        logEvent("warn", `会话关闭 ${shortId(data.sessionId)} ${data.reason || ""} code=${data.closeCode ?? "-"}`);
        state.sessions.delete(data.sessionId);
        renderSessions();
      } else {
        logEvent("event", { event: name, data });
        renderSessions();
      }
      renderTerminalMeta();
      renderDebugOptions();
      renderDebugWriteOptions();
      return;
    }
    if (name === "admin.state") {
      state.adminEnabled = !!data.enabled;
      renderAdmin();
      logEvent("event", { event: name, data });
      return;
    }
    if (name === "ports.changed") {
      logEvent("event", { event: name, data });
      refreshPorts().catch(() => {});
      return;
    }
    if (name === "debug.frame") {
      onDebugFrame(data);
      return;
    }
    logEvent("event", { event: name, data });
  });

  client.onSync(() => syncFromClient());

  function syncFromClient() {
    state.ports = Array.isArray(client.ports) ? client.ports : [];
    state.sessions = new Map((client.sessions || []).map((s) => [s.sessionId, s]));
    for (const slot of ["A", "B"]) {
      const controller = state.slots[slot];
      if (!controller) continue;
      const info = state.sessions.get(controller.sessionId);
      if (info) {
        controller.updateInfo(info);
      } else if (controller.state !== "closed") {
        controller.state = "closed";
        controller._updateTerminalHead();
      }
    }
    renderPorts();
    renderSessions();
    renderTerminalMeta();
    renderHelloSessions();
  }

  function onHello(data) {
    state.hello = data;
    state.adminEnabled = !!data.admin?.enabled;
    state.autostart = !!data.autostart;
    $("#demo-hello-protocol").textContent = data.protocol || "-";
    $("#demo-hello-version").textContent = data.version || "-";
    $("#demo-hello-os").textContent = `${data.os || "-"} / ${data.arch || "-"}`;
    $("#demo-hello-admin").textContent = data.admin
      ? `${data.admin.enabled ? "开启" : "关闭"} ${data.admin.url || ""}`
      : "-";
    $("#demo-hello-autostart").textContent = `${data.autostart ? "开启" : "关闭"}（只读，托盘菜单修改）`;
    $("#demo-pill-version").textContent = `版本：${data.version || "-"}`;
    renderAdmin();
    renderHelloSessions();
    logEvent("event", { event: "hello", data });
  }

  function renderAdmin() {
    const pill = $("#demo-pill-admin");
    if (pill) {
      if (state.adminEnabled == null) {
        pill.textContent = "管理页：-";
        pill.className = "demo-pill";
      } else {
        pill.textContent = `管理页：${state.adminEnabled ? "开启" : "关闭"}`;
        pill.className = "demo-pill " + (state.adminEnabled ? "ok" : "warn");
      }
    }
    const admin = $("#demo-hello-admin");
    if (admin && state.hello?.admin) {
      admin.textContent = `${state.adminEnabled ? "开启" : "关闭"} ${state.hello.admin.url || ""}`;
    }
    const autostart = $("#demo-pill-autostart");
    if (autostart) {
      if (state.autostart == null) {
        autostart.textContent = "自启：-";
        autostart.className = "demo-pill";
      } else {
        autostart.textContent = `自启：${state.autostart ? "开启" : "关闭"}`;
        autostart.className = "demo-pill " + (state.autostart ? "ok" : "muted");
      }
    }
  }

  function renderHelloSessions() {
    const count = state.sessions.size;
    const hello = $("#demo-hello-sessions");
    if (hello) hello.textContent = `${count} 个`;
    const badge = $("#demo-sessions-count");
    if (badge) badge.textContent = String(count);
  }

  // ----------------------------- ports -----------------------------
  async function refreshPorts() {
    if (!client.connected) throw new Error("管理 WS 未连接");
    await client.refreshPorts();
    state.ports = Array.isArray(client.ports) ? client.ports : [];
    renderPorts();
    logEvent("info", `端口列表 ${state.ports.length} 个: ${state.ports.map((p) => p.path).join(", ") || "无"}`);
    return state.ports;
  }

  function renderPorts() {
    const body = $("#demo-ports-body");
    const empty = $("#demo-ports-empty");
    const datalist = $("#demo-port-list");
    if (!body || !datalist) return;
    body.textContent = "";
    datalist.textContent = "";
    if (!state.ports.length) {
      empty?.classList.remove("hidden");
    } else {
      empty?.classList.add("hidden");
    }
    for (const p of state.ports) {
      const tr = document.createElement("tr");
      tr.appendChild(el("td", "mono", p.path));
      tr.appendChild(el("td", "", p.portType || "-"));
      tr.appendChild(el("td", "", p.manufacturer || "-"));
      tr.appendChild(el("td", "", p.product || "-"));
      tr.appendChild(el("td", "mono", p.serialNumber || "-"));
      tr.appendChild(el("td", "mono", p.vid != null || p.pid != null ? `${p.vid ?? "?"}/${p.pid ?? "?"}` : "-"));
      const actions = document.createElement("td");
      const btnA = el("button", "btn small ghost", "设为 A");
      btnA.addEventListener("click", () => {
        $("#demo-path-A").value = p.path;
      });
      const btnB = el("button", "btn small ghost", "设为 B");
      btnB.addEventListener("click", () => {
        $("#demo-path-B").value = p.path;
      });
      actions.appendChild(btnA);
      actions.appendChild(document.createTextNode(" "));
      actions.appendChild(btnB);
      tr.appendChild(actions);
      body.appendChild(tr);
      const option = el("option", "", p.path);
      option.value = p.path;
      datalist.appendChild(option);
    }
    if (!$("#demo-path-A").value && state.ports[0]) $("#demo-path-A").value = state.ports[0].path;
    if (!$("#demo-path-B").value && state.ports[1]) $("#demo-path-B").value = state.ports[1].path;
    const count = $("#demo-ports-count");
    if (count) count.textContent = String(state.ports.length);
  }

  // ----------------------------- sessions -----------------------------
  function readOpenParams(pathOverride) {
    const path = String(pathOverride ?? $("#demo-path-A").value ?? "").trim();
    if (!path) throw new Error("请选择或输入串口");
    const baudRate = Number($("#demo-baud").value);
    if (!Number.isInteger(baudRate) || baudRate <= 0) throw new Error("波特率必须是正整数");
    const params = {
      path,
      baudRate,
      dataBits: Number($("#demo-databits").value),
      parity: $("#demo-parity").value,
      stopBits: Number($("#demo-stopbits").value),
      flowControl: $("#demo-flowcontrol").value,
    };
    if ($("#demo-dtr").value !== "default") params.dtr = $("#demo-dtr").value === "true";
    if ($("#demo-rts").value !== "default") params.rts = $("#demo-rts").value === "true";
    return params;
  }

  function slotForSession(sessionId) {
    for (const slot of ["A", "B"]) {
      if (state.slots[slot]?.sessionId === sessionId) return state.slots[slot];
    }
    return null;
  }

  async function openSlot(slot) {
    const path = $(`#demo-path-${slot}`).value.trim();
    if (!path) throw new Error(`请先选择 ${slot} 串口`);
    if (state.slots[slot] && state.slots[slot].state !== "closed") {
      throw new Error(`${slot} 已有会话，请先关闭`);
    }
    state.slots[slot] = null;
    const params = readOpenParams(path);
    let data = null;
    let lastErr = null;
    for (let attempt = 0; attempt < 10; attempt++) {
      try {
        data = await client.request("session.open", params);
      } catch (err) {
        lastErr = err;
        // 端口可能正在异步释放（例如刚关闭）：短暂重试即可，不掩盖真正的占用。
        if (err.code !== "PORT_BUSY") throw err;
        await sleep(120);
        continue;
      }
      if (blockedDataUrl(data.dataUrl)) {
        lastErr = new Error(`动态数据端口 ${dataUrlPort(data.dataUrl)} 被浏览器禁止，已换端口重试`);
        logEvent("warn", lastErr.message);
        await client.request("session.close", { sessionId: data.sessionId }).catch(() => {});
        await sleep(100);
        data = null;
        continue;
      }
      break;
    }
    if (!data) throw lastErr || new Error("打开会话失败");
    state.sessions.set(data.sessionId, {
      sessionId: data.sessionId,
      state: data.state || "awaiting",
      port: path,
      params,
      counters: data.counters || { rxBytes: 0, txBytes: 0, rxDiscardedBytes: 0 },
      dataPort: (() => {
        try {
          return Number(new URL(data.dataUrl).port);
        } catch {
          return null;
        }
      })(),
    });
    const controller = new SessionController(slot, { ...data, state: "awaiting" }, params, client);
    controller.autoReconnect = state.autoReconnect;
    state.slots[slot] = controller;
    renderSessions();
    try {
      await controller.connectData();
    } catch (err) {
      logEvent("error", `${slot} 打开后连接数据 WS 失败: ${err.message}`);
    }
    logEvent("ok", `${slot} 已打开 ${path} (${data.reused ? "复用" : "新建"})`);
    renderTerminal();
    return controller;
  }

  async function closeSlot(slot, asClient = false) {
    const controller = state.slots[slot];
    if (!controller) return;
    await controller.closeSession(asClient);
    state.slots[slot] = null;
    state.sessions.delete(controller.sessionId);
    renderSessions();
    renderTerminal();
    logEvent("info", `${slot} 已关闭`);
  }

  async function refreshSessions() {
    if (!client.connected) return;
    await client.refreshSessions();
    syncFromClient();
  }

  function renderSessions() {
    const body = $("#demo-sessions-body");
    const empty = $("#demo-sessions-empty");
    if (!body) return;
    body.textContent = "";
    const rows = Array.from(state.sessions.values());
    if (!rows.length) empty?.classList.remove("hidden");
    else empty?.classList.add("hidden");
    for (const session of rows) {
      const tr = document.createElement("tr");
      const slot = ["A", "B"].find((name) => state.slots[name]?.sessionId === session.sessionId) || "-";
      tr.appendChild(el("td", "mono", slot));
      tr.appendChild(el("td", "mono", session.port || "-"));
      const stateCell = document.createElement("td");
      stateCell.appendChild(el("span", "badge " + stateClass(session.state), session.state || "-"));
      tr.appendChild(stateCell);
      const p = session.params || {};
      tr.appendChild(
        el("td", "mono", `${p.baudRate ?? "?"} ${p.dataBits ?? "?"}${p.parity ?? "?"}${p.stopBits ?? "?"} ${p.flowControl ?? ""}`),
      );
      const c = session.counters || {};
      tr.appendChild(el("td", "mono", `${c.rxBytes ?? 0} / ${c.txBytes ?? 0} / ${c.rxDiscardedBytes ?? 0}`));
      tr.appendChild(
        el("td", "mono", session.lastGap ? `${session.lastGap.discardedBytes} B / ${session.lastGap.durationMs ?? "?"} ms` : "-"),
      );
      tr.appendChild(el("td", "mono", session.lastError ? `${session.lastError.code}: ${session.lastError.message}` : "-"));
      const action = document.createElement("td");
      const close = el("button", "btn small ghost", "关闭");
      close.addEventListener("click", async () => {
        try {
          if (slot !== "-") {
            await closeSlot(slot);
          } else {
            await client.request("session.close", { sessionId: session.sessionId });
            await refreshSessions();
          }
        } catch (err) {
          logEvent("error", err.message);
        }
      });
      action.appendChild(close);
      tr.appendChild(action);
      body.appendChild(tr);
    }
    const count = $("#demo-sessions-count");
    if (count) count.textContent = String(rows.length);
    renderHelloSessions();
    renderDebugOptions();
    renderDebugWriteOptions();
  }

  // ----------------------------- data session -----------------------------
  class SessionController {
    constructor(slot, info, params, management) {
      this.slot = slot;
      this.sessionId = info.sessionId;
      this.dataUrl = info.dataUrl;
      this.params = params;
      this.mgmt = management;
      this.ws = null;
      this.state = info.state || "awaiting";
      this.counters = info.counters || { rxBytes: 0, txBytes: 0, rxDiscardedBytes: 0 };
      this.graceEndsAt = null;
      this.lastGap = null;
      this.lastError = null;
      this.frames = [];
      this.frameBytes = 0;
      this.buffer = new Uint8Array(0);
      this.waiters = [];
      this.closeCode = null;
      this.closeReason = "";
      this._closeWaiters = [];
      this.manual = false;
      this.connecting = false;
      this.autoReconnect = false;
      this.reconnectTimer = null;
      this._updateTerminalHead();
    }

    async connectData(timeout = 5000) {
      if (this.ws && this.ws.readyState === WebSocket.OPEN) return;
      this.manual = false;
      this.connecting = true;
      const url = this.dataUrl;
      return new Promise((resolve, reject) => {
        const ws = new WebSocket(url);
        ws.binaryType = "arraybuffer";
        this.ws = ws;
        let settled = false;
        const timer = setTimeout(() => {
          if (settled) return;
          settled = true;
          this.connecting = false;
          try {
            ws.close();
          } catch {
            // ignore
          }
          reject(new Error("数据 WS 连接超时"));
        }, timeout);
        ws.onopen = () => {
          if (settled) return;
          settled = true;
          this.connecting = false;
          clearTimeout(timer);
          this.state = "active";
          this.closeCode = null;
          this._updateTerminalHead();
          renderSessions();
          logEvent("ok", `${this.slot} 数据 WS 已连接 (${url.split("/").pop()?.slice(0, 8)})`);
          resolve();
        };
        ws.onmessage = (event) => {
          if (!(event.data instanceof ArrayBuffer)) {
            logEvent("error", `${this.slot} 数据 WS 收到非二进制帧`);
            return;
          }
          const bytes = new Uint8Array(event.data);
          this._appendBuffer(bytes);
          this._appendFrame("RX", bytes);
        };
        ws.onerror = () => {
          if (!settled) {
            settled = true;
            this.connecting = false;
            clearTimeout(timer);
            reject(new Error("数据 WS 连接失败"));
          }
        };
        ws.onclose = (event) => {
          this.connecting = false;
          this.closeCode = event.code;
          this.closeReason = event.reason || "";
          this.state = [1000, 4001, 4004, 4005, 4007].includes(event.code) ? "closed" : "grace";
          this._updateTerminalHead();
          renderSessions();
          logEvent(event.code === 1000 ? "ok" : "warn", `${this.slot} 数据 WS 关闭 code=${event.code} ${event.reason || ""}`);
          for (const done of this._closeWaiters.splice(0)) done();
          if (!settled) {
            settled = true;
            clearTimeout(timer);
            reject(new Error(`数据 WS 连接失败 (${event.code})`));
          }
          if (!this.manual && event.code !== 1000 && this.autoReconnect) this._scheduleReconnect();
        };
      });
    }

    _scheduleReconnect() {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = setTimeout(async () => {
        if (this.manual) return;
        try {
          await this.reconnectOld();
          logEvent("ok", `${this.slot} 自动重连成功`);
        } catch (err) {
          logEvent("error", `${this.slot} 自动重连失败: ${err.message}`);
        }
      }, 350);
    }

    async reconnectOld() {
      this.manual = false;
      try {
        await this.connectData(3000);
        return;
      } catch {
        // fall through to session.open reuse
      }
      let lastErr = null;
      for (let i = 0; i < 6; i++) {
        try {
          const data = await this.mgmt.request("session.open", this.params);
          this.sessionId = data.sessionId;
          this.dataUrl = data.dataUrl;
          this.state = "awaiting";
          await this.connectData(3000);
          return;
        } catch (err) {
          lastErr = err;
          await sleep(250);
        }
      }
      throw lastErr || new Error("重连失败");
    }

    _appendBuffer(bytes) {
      if (!bytes.length) return;
      const next = new Uint8Array(this.buffer.length + bytes.length);
      next.set(this.buffer, 0);
      next.set(bytes, this.buffer.length);
      this.buffer = next;
      if (this.buffer.length > 4 * 1024 * 1024) {
        this.buffer = this.buffer.slice(this.buffer.length - 4 * 1024 * 1024);
      }
      this._flushWaiters();
    }

    _flushWaiters() {
      while (this.waiters.length) {
        const waiter = this.waiters[0];
        if (this.buffer.length < waiter.n) break;
        this.waiters.shift();
        clearTimeout(waiter.timer);
        const out = this.buffer.slice(0, waiter.n);
        this.buffer = this.buffer.slice(waiter.n);
        waiter.resolve(out);
      }
    }

    takeBytes(n, timeout = 5000) {
      if (this.buffer.length >= n) {
        const out = this.buffer.slice(0, n);
        this.buffer = this.buffer.slice(n);
        return Promise.resolve(out);
      }
      return new Promise((resolve, reject) => {
        const waiter = { n, resolve, reject };
        waiter.timer = setTimeout(() => {
          const index = this.waiters.indexOf(waiter);
          if (index >= 0) this.waiters.splice(index, 1);
          reject(new Error(`等待 ${n} 字节超时（当前缓冲 ${this.buffer.length}）`));
        }, timeout);
        this.waiters.push(waiter);
      });
    }

    clearBuffer() {
      this.buffer = new Uint8Array(0);
    }

    sendBytes(bytes) {
      if (!this.ws || this.ws.readyState !== WebSocket.OPEN) {
        throw new Error(`${this.slot} 数据 WS 未连接`);
      }
      this.ws.send(bytes);
      this._appendFrame("TX", bytes);
    }

    sendText(text) {
      this.sendBytes(textEncoder.encode(text));
    }

    sendHex(text) {
      this.sendBytes(hexToBytes(text));
    }

    closeData(code = 1000, reason = "") {
      this.manual = true;
      clearTimeout(this.reconnectTimer);
      if (this.ws && this.ws.readyState <= WebSocket.OPEN) {
        try {
          this.ws.close(code, reason);
        } catch {
          // ignore
        }
      }
      this.state = code === 1000 ? "closed" : this.state;
      this._updateTerminalHead();
      renderSessions();
    }

    disconnectAbnormal(code = 3001) {
      // WebSocket API 只允许 1000 或 3000-4999；1001 会抛 InvalidAccessError。
      // 非 1000 的应用关闭码会被服务端视为异常断开并进入 grace。
      this.closeData(code, "demo-abnormal");
    }

    waitClose(timeout = 2500) {
      if (!this.ws || this.ws.readyState === WebSocket.CLOSED) return Promise.resolve();
      return new Promise((resolve) => {
        let done = false;
        const finish = () => {
          if (done) return;
          done = true;
          clearTimeout(timer);
          resolve();
        };
        const timer = setTimeout(finish, timeout);
        this._closeWaiters.push(finish);
      });
    }

    async closeSession(asClient = false) {
      this.manual = true;
      clearTimeout(this.reconnectTimer);
      if (asClient && this.ws && this.ws.readyState <= WebSocket.OPEN) {
        const closed = this.waitClose(2500);
        try {
          this.ws.close(1000, "demo-client-close");
        } catch {
          // ignore
        }
        await closed;
        await sleep(120);
        try {
          const data = await this.mgmt.request("session.list");
          if ((data.sessions || []).some((s) => s.sessionId === this.sessionId)) {
            await this.mgmt.request("session.close", { sessionId: this.sessionId }).catch(() => {});
          }
        } catch {
          // 会话已销毁
        }
      } else {
        try {
          await this.mgmt.request("session.close", { sessionId: this.sessionId });
        } catch {
          // 已关闭或已过期
        }
        if (this.ws && this.ws.readyState <= WebSocket.OPEN) {
          try {
            this.ws.close(1000, "admin-close");
          } catch {
            // ignore
          }
        }
      }
      this.state = "closed";
      this._updateTerminalHead();
      renderSessions();
    }

    updateInfo(info) {
      if (!info) return;
      this.state = info.state;
      this.counters = info.counters || this.counters;
      this.graceEndsAt = info.graceEndsAt || null;
      this.lastGap = info.lastGap || null;
      this.lastError = info.lastError || null;
      if (info.closeCode != null) this.closeCode = info.closeCode;
      this._updateTerminalHead();
      this._renderMeta();
    }

    _appendFrame(dir, bytes) {
      const frame = { ts: now(), dir, bytes: new Uint8Array(bytes) };
      this.frames.push(frame);
      this.frameBytes += frame.bytes.length;
      while (this.frames.length > 5000 || this.frameBytes > 4 * 1024 * 1024) {
        const removed = this.frames.shift();
        this.frameBytes -= removed.bytes.length;
      }
      appendTerminalLine(this.slot, frame);
      this._renderMeta();
    }

    _updateTerminalHead() {
      const port = $(`#demo-term-port-${this.slot}`);
      const st = $(`#demo-term-state-${this.slot}`);
      const sid = $(`#demo-term-sid-${this.slot}`);
      if (port) port.textContent = this.params?.path || "-";
      if (st) {
        st.textContent = this.state;
        st.className = "badge " + stateClass(this.state);
      }
      if (sid) sid.textContent = this.sessionId ? shortId(this.sessionId) : "";
      this._renderMeta();
    }

    _renderMeta() {
      const c = this.counters || {};
      const counters = `RX ${c.rxBytes ?? 0} / TX ${c.txBytes ?? 0} / DROP ${c.rxDiscardedBytes ?? 0}`;
      const countersEl = $(`#demo-term-counters-${this.slot}`);
      if (countersEl) countersEl.textContent = counters;
      const close = $(`#demo-term-close-${this.slot}`);
      if (close) {
        close.textContent = this.closeCode == null ? "" : `close ${this.closeCode}${this.closeReason ? " " + this.closeReason : ""}`;
      }
      const grace = $(`#demo-term-grace-${this.slot}`);
      if (grace) {
        if (this.state === "grace" && this.graceEndsAt) {
          const left = Math.max(0, Math.ceil((this.graceEndsAt - now()) / 1000));
          grace.textContent = `grace ${left}s`;
          grace.className = "red";
        } else {
          grace.textContent = "";
          grace.className = "";
        }
      }
    }
  }

  // ----------------------------- terminal -----------------------------
  function terminalPrefs(slot) {
    return state.terminalPrefs[slot];
  }

  function renderTerminal() {
    for (const slot of ["A", "B"]) renderTerminalLog(slot);
    renderTerminalMeta();
  }

  function renderTerminalMeta() {
    for (const slot of ["A", "B"]) {
      const controller = state.slots[slot];
      if (controller) {
        controller._renderMeta();
      } else {
        const st = $(`#demo-term-state-${slot}`);
        const port = $(`#demo-term-port-${slot}`);
        const sid = $(`#demo-term-sid-${slot}`);
        if (st) {
          st.textContent = "-";
          st.className = "badge";
        }
        if (port) port.textContent = "未打开";
        if (sid) sid.textContent = "";
        const counters = $(`#demo-term-counters-${slot}`);
        if (counters) counters.textContent = "RX 0 / TX 0 / DROP 0";
        const close = $(`#demo-term-close-${slot}`);
        if (close) close.textContent = "";
        const grace = $(`#demo-term-grace-${slot}`);
        if (grace) grace.textContent = "";
      }
    }
  }

  function renderTerminalLog(slot) {
    const box = $(`#demo-log-${slot}`);
    if (!box) return;
    const controller = state.slots[slot];
    box.textContent = "";
    if (!controller || !controller.frames.length) {
      box.appendChild(el("div", "empty", controller ? "暂无收发" : "未打开会话"));
      return;
    }
    const frag = document.createDocumentFragment();
    for (const frame of controller.frames) frag.appendChild(makeTerminalLine(slot, frame));
    box.appendChild(frag);
    box.scrollTop = box.scrollHeight;
  }

  function makeTerminalLine(slot, frame) {
    const prefs = terminalPrefs(slot);
    const line = el("div", "log-line");
    line.appendChild(el("span", "ts", prefs.ts ? fmtTime(frame.ts) : ""));
    line.appendChild(el("span", "dir " + (frame.dir === "RX" ? "rx" : "tx"), frame.dir));
    let body;
    if (prefs.hex) {
      const max = 512;
      body = bytesToHex(frame.bytes.subarray(0, max));
      if (frame.bytes.length > max) body += ` … (+${frame.bytes.length - max} B)`;
    } else {
      body = bytesToAscii(frame.bytes);
      if (body.length > 512) body = body.slice(0, 512) + `… (+${frame.bytes.length - 512} B)`;
    }
    line.appendChild(el("span", "body", body));
    return line;
  }

  function appendTerminalLine(slot, frame) {
    const prefs = terminalPrefs(slot);
    if (prefs.paused) return;
    const box = $(`#demo-log-${slot}`);
    if (!box) return;
    if (box.querySelector(".empty")) box.textContent = "";
    box.appendChild(makeTerminalLine(slot, frame));
    while (box.childElementCount > 5000) box.firstElementChild?.remove();
    if (prefs.autoscroll) box.scrollTop = box.scrollHeight;
  }

  function bindTerminalPrefs(slot) {
    const hex = $(`#demo-hex-${slot}`);
    const ts = $(`#demo-ts-${slot}`);
    const auto = $(`#demo-autoscroll-${slot}`);
    const paused = $(`#demo-paused-${slot}`);
    const sync = () => {
      const prefs = terminalPrefs(slot);
      prefs.hex = hex.checked;
      prefs.ts = ts.checked;
      prefs.autoscroll = auto.checked;
      prefs.paused = paused.checked;
      renderTerminalLog(slot);
    };
    [hex, ts, auto, paused].forEach((node) => node?.addEventListener("change", sync));
  }

  function sendFromSlot(slot) {
    const controller = state.slots[slot];
    if (!controller) {
      toast(`${slot} 未打开`, "err");
      return;
    }
    const mode = $(`#demo-sendmode-${slot}`).value;
    const text = $(`#demo-send-${slot}`).value;
    if (!text.length) return;
    try {
      if (mode === "hex") controller.sendHex(text);
      else controller.sendText(text);
      $(`#demo-send-${slot}`).value = "";
    } catch (err) {
      logEvent("error", `${slot} 发送失败: ${err.message}`);
    }
  }

  async function loopback(fromSlot, toSlot) {
    const from = state.slots[fromSlot];
    const to = state.slots[toSlot];
    if (!from || !to) throw new Error("请先打开 A 和 B");
    if (from.state !== "active" || to.state !== "active") throw new Error("A/B 数据 WS 都必须是 active");
    from.clearBuffer();
    to.clearBuffer();
    const payload = pattern(4096);
    from.sendBytes(payload);
    const got = await to.takeBytes(payload.length, 6000);
    if (!bytesEqual(got, payload)) {
      let at = 0;
      while (at < got.length && got[at] === payload[at]) at++;
      throw new Error(`回环内容不一致（收到 ${got.length}/${payload.length}，首个差异 @${at}）`);
    }
    $("#demo-loop-result").textContent = `${fromSlot} → ${toSlot} 4096 字节一致`;
    logEvent("ok", `${fromSlot} → ${toSlot} 回环 4096 字节通过`);
    return true;
  }

  function exportTerminal(slot) {
    const controller = state.slots[slot];
    if (!controller) return;
    const lines = controller.frames.map(
      (frame) => `${fmtTime(frame.ts)}\t${frame.dir}\t${bytesToHex(frame.bytes)}\t${bytesToAscii(frame.bytes)}`,
    );
    download(`ws2s-${slot}-${Date.now()}.txt`, lines.join("\n"));
  }

  // ----------------------------- debug -----------------------------
  function renderDebugOptions() {
    const box = $("#demo-debug-sessions");
    if (!box) return;
    const selected = new Set(state.debug.ids);
    const all = $("#demo-debug-all")?.checked !== false;
    box.textContent = "";
    for (const session of state.sessions.values()) {
      const label = el("label", "check");
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.dataset.session = session.sessionId;
      checkbox.checked = all || selected.has(session.sessionId);
      checkbox.disabled = all;
      label.appendChild(checkbox);
      label.appendChild(
        document.createTextNode(`${slotForSession(session.sessionId)?.slot || "?"} ${session.port} (${shortId(session.sessionId)})`),
      );
      box.appendChild(label);
    }
    if (!state.sessions.size) box.appendChild(el("span", "muted small", "暂无会话"));
  }

  function renderDebugState() {
    const stateEl = $("#demo-debug-sub-state");
    if (stateEl) {
      stateEl.textContent = state.debug.subscribed ? "已订阅" : "未订阅";
      stateEl.className = "badge " + (state.debug.subscribed ? "active" : "");
    }
    const countEl = $("#demo-debug-count");
    if (countEl) countEl.textContent = `${state.debug.frames.length} 帧`;
    const gapsEl = $("#demo-debug-gaps");
    if (gapsEl) gapsEl.textContent = `seq 缺口 ${state.debug.gaps}`;
  }

  function debugFrameRow(frame) {
    const tr = document.createElement("tr");
    tr.appendChild(el("td", "mono", fmtTime(frame.ts)));
    const slot = slotForSession(frame.sessionId);
    tr.appendChild(el("td", "mono", `${slot?.slot || "?"} ${shortId(frame.sessionId)}`));
    tr.appendChild(el("td", "", frame.direction === "rx" ? "RX" : "TX"));
    tr.appendChild(el("td", "mono", frame.seq ?? "-"));
    tr.appendChild(el("td", "mono", String(frame.size ?? frame.bytes.length)));
    const cell = el("td", "hexcell");
    const max = 160;
    cell.textContent = `${bytesToHex(frame.bytes.subarray(0, max))}${frame.bytes.length > max ? " …" : ""}`;
    cell.title = bytesToAscii(frame.bytes);
    tr.appendChild(cell);
    return tr;
  }

  function renderDebugFrames() {
    const body = $("#demo-debug-body");
    const empty = $("#demo-debug-empty");
    if (!body) return;
    body.textContent = "";
    if (!state.debug.frames.length) empty?.classList.remove("hidden");
    else empty?.classList.add("hidden");
    const frag = document.createDocumentFragment();
    for (const frame of state.debug.frames.slice(-5000)) frag.appendChild(debugFrameRow(frame));
    body.appendChild(frag);
    renderDebugState();
  }

  function onDebugFrame(data) {
    const bytes = b64decodeBytes(data.dataB64 || "");
    const frame = { ts: now(), ...data, bytes };
    const last = state.debug.lastSeq.get(data.sessionId) || 0;
    if (data.seq > last + 1) {
      state.debug.gaps += data.seq - last - 1;
      logEvent("warn", `调试 seq 缺口: ${shortId(data.sessionId)} ${last} → ${data.seq}`);
    }
    if (data.seq) state.debug.lastSeq.set(data.sessionId, data.seq);
    state.debug.frames.push(frame);
    $("#demo-debug-empty")?.classList.add("hidden");
    if (state.debug.frames.length > 5000) state.debug.frames.splice(0, state.debug.frames.length - 5000);
    if (state.debug.frames.length % 20 === 0) {
      renderDebugFrames();
    } else {
      const body = $("#demo-debug-body");
      if (body) {
        body.appendChild(debugFrameRow(frame));
        while (body.childElementCount > 5000) body.firstElementChild?.remove();
      }
      renderDebugState();
    }
    const controller = slotForSession(data.sessionId);
    if (controller) controller._renderMeta();
  }

  async function subscribeDebug() {
    if (!client.connected) throw new Error("管理 WS 未连接");
    const all = $("#demo-debug-all").checked;
    const ids = all ? "all" : $$("#demo-debug-sessions input[type=checkbox]:checked").map((x) => x.dataset.session);
    if (!all && !ids.length) throw new Error("请选择至少一个会话，或勾选「全部会话」");
    await client.subscribeDebug(ids, $("#demo-debug-replay").checked);
    state.debug.subscribed = true;
    state.debug.all = all;
    state.debug.ids = new Set(all ? [] : ids);
    renderDebugOptions();
    renderDebugState();
    logEvent("ok", `已订阅调试帧: ${all ? "全部" : ids.length + " 个会话"}`);
  }

  async function unsubscribeDebug() {
    if (!client.connected) throw new Error("管理 WS 未连接");
    await client.unsubscribeDebug();
    state.debug.subscribed = false;
    state.debug.ids.clear();
    renderDebugOptions();
    renderDebugState();
    logEvent("info", "已取消调试订阅");
  }

  function exportDebug() {
    const lines = state.debug.frames.map(
      (frame) =>
        `${fmtTime(frame.ts)}\t${frame.sessionId}\t${frame.direction}\t${frame.seq}\t${bytesToHex(frame.bytes)}\t${bytesToAscii(frame.bytes)}`,
    );
    download(`ws2s-debug-${Date.now()}.txt`, lines.join("\n"));
  }

  async function debugWrite() {
    const sessionId = $("#demo-debug-write-session").value;
    if (!sessionId) throw new Error("没有可写的会话");
    const mode = $("#demo-debug-write-mode").value;
    const text = $("#demo-debug-write-input").value;
    if (!text.length) throw new Error("请输入内容");
    const bytes = mode === "hex" ? hexToBytes(text) : textEncoder.encode(text);
    const data = await client.request("session.write", { sessionId, dataB64: b64encodeBytes(bytes) });
    $("#demo-debug-write-result").textContent = `written=${data.written}`;
    logEvent("ok", `调试写 ${sessionId.slice(0, 8)} ${data.written} 字节`);
  }

  function renderDebugWriteOptions() {
    const select = $("#demo-debug-write-session");
    if (!select) return;
    const previous = select.value;
    select.textContent = "";
    for (const session of state.sessions.values()) {
      const option = el("option", "", `${slotForSession(session.sessionId)?.slot || "?"} ${session.port} (${shortId(session.sessionId)})`);
      option.value = session.sessionId;
      select.appendChild(option);
    }
    if (previous && [...select.options].some((option) => option.value === previous)) {
      select.value = previous;
    }
  }

  // ----------------------------- edge tests -----------------------------
  function requireA() {
    const a = state.slots.A;
    if (!a) throw new Error("请先在「会话」子 tab 打开 A");
    return a;
  }

  function waitClose(ws, timeout = 4000) {
    return new Promise((resolve) => {
      let done = false;
      const finish = (code) => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        resolve(code);
      };
      const timer = setTimeout(() => {
        try {
          ws.close();
        } catch {
          // ignore
        }
        finish(-1);
      }, timeout);
      ws.onclose = (event) => finish(event.code);
      ws.onerror = () => {};
    });
  }

  async function edgeSecondClient() {
    const a = requireA();
    if (a.state !== "active") throw new Error("A 必须是 active 才能验证第二客户端 4002");
    if (!a.dataUrl) throw new Error("A 没有数据 URL");
    const ws = new WebSocket(a.dataUrl);
    ws.binaryType = "arraybuffer";
    const code = await waitClose(ws, 4000);
    edgeLog(`第二数据客户端 close code=${code}（期望 4002）`);
    logEvent(code === 4002 ? "ok" : "warn", `第二数据客户端 close=${code}`);
  }

  async function edgeActiveOpen() {
    const a = requireA();
    if (a.state !== "active") throw new Error("A 必须是 active 才能验证 PORT_BUSY");
    try {
      await client.request("session.open", a.params);
      edgeLog("重复 open 意外成功");
    } catch (err) {
      edgeLog(`重复 open 被拒绝: ${err.code || "ERROR"}: ${err.message}（期望 PORT_BUSY）`);
      logEvent("ok", "active 重复 open 被拒绝");
    }
  }

  async function edgeBadToken() {
    const a = requireA();
    if (a.state !== "active") throw new Error("A 必须是 active 才能验证 token");
    const base = a.dataUrl.split("/").slice(0, -1).join("/");
    const ws = new WebSocket(`${base}/bad-token-${Date.now()}`);
    ws.binaryType = "arraybuffer";
    const code = await waitClose(ws, 4000);
    edgeLog(`错误 token 连接结束 code=${code}（浏览器不暴露 HTTP 404，只看到握手失败/1006）`);
  }

  async function edgeMissingPort() {
    const base = state.ports[0]?.path || "serial";
    const path = `${base}__missing__`;
    try {
      await client.request("session.open", { ...readOpenParams(path), path });
      edgeLog("不存在串口意外打开成功");
    } catch (err) {
      edgeLog(`不存在串口被拒绝: ${err.code || "ERROR"}: ${err.message}（期望 PORT_NOT_FOUND）`);
    }
  }

  async function edgeWriteInactive() {
    const used = new Set(Array.from(state.sessions.values(), (s) => s.port));
    const free = state.ports.find((p) => !used.has(p.path));
    let sessionId = null;
    let restoreA = false;
    if (free) {
      const data = await client.request("session.open", readOpenParams(free.path));
      sessionId = data.sessionId;
    } else {
      const a = requireA();
      const path = a.params.path;
      await closeSlot("A");
      restoreA = true;
      edgeLog(`没有空闲串口，临时借用 A (${path}) 测试，完成后自动恢复`);
      const data = await client.request("session.open", readOpenParams(path));
      sessionId = data.sessionId;
    }
    try {
      await client.request("session.write", { sessionId, dataB64: btoa("x") });
      edgeLog("非 active 写意外成功");
    } catch (err) {
      edgeLog(`非 active 写被拒绝: ${err.code || "ERROR"}: ${err.message}（期望 SESSION_STATE）`);
    } finally {
      await client.request("session.close", { sessionId }).catch(() => {});
      if (restoreA) {
        try {
          await openSlot("A");
          edgeLog("A 已恢复");
        } catch (err) {
          edgeLog(`A 恢复失败: ${err.message}`);
        }
      }
    }
  }

  async function edgeUnknownAction() {
    try {
      await client.request("no.such.action", {});
      edgeLog("未知 action 意外成功");
    } catch (err) {
      edgeLog(`未知 action: ${err.code || "ERROR"}: ${err.message}（期望 UNKNOWN_ACTION）`);
    }
  }

  async function edgeTextFrame() {
    const a = requireA();
    if (!a.ws || a.ws.readyState !== WebSocket.OPEN) throw new Error("A 数据 WS 未连接");
    if (!confirm("会向 A 发送文本帧，服务端将以 1003 关闭数据 WS，会话进入 grace。继续？")) return;
    a.ws.send("this-is-a-text-frame");
    edgeLog("已发送文本帧，等待 1003...");
    await sleep(400);
    edgeLog(`A 状态=${a.state} close=${a.closeCode ?? "-"}`);
  }

  async function edgeOversize() {
    const a = requireA();
    if (!a.ws || a.ws.readyState !== WebSocket.OPEN) throw new Error("A 数据 WS 未连接");
    if (!confirm("会向 A 发送 1MiB+1 字节，服务端将以 1009 关闭数据 WS，会话进入 grace。继续？")) return;
    a.ws.send(new Uint8Array(1024 * 1024 + 1));
    edgeLog("已发送 1MiB+1，等待 1009...");
    await sleep(500);
    edgeLog(`A 状态=${a.state} close=${a.closeCode ?? "-"}`);
  }

  async function edgeAbnormal() {
    const a = requireA();
    if (a.state !== "active") throw new Error("A 必须是 active 才能模拟异常断开");
    a.disconnectAbnormal();
    edgeLog("已发送 close 3001（浏览器不允许发 1001），等待 session.state=grace...");
    await sleep(400);
    edgeLog(`A 状态=${a.state}`);
  }

  async function edgeNormalClose() {
    if (!state.slots.A) throw new Error("A 未打开");
    await closeSlot("A", true);
    edgeLog("A 已正常关闭 1000，端口应已释放");
  }


  // ----------------------------- rule runner -----------------------------
  function runnerSteps() {
    return [
      ["connect", "连接管理 WS"],
      ["ports", "扫描端口"],
      ["select", "选择 A/B 两个不同串口"],
      ["open", "打开 A/B 串口"],
      ["data", "连接 A/B 数据 WS"],
      ["ab", "A → B 4KB 回环"],
      ["ba", "B → A 4KB 回环"],
      ["counters", "读取会话计数"],
      ["debug", "订阅调试帧并验证"],
      ["debug-write", "调试写并验证 B 收到"],
      ["second", "第二数据客户端 4002"],
      ["busy", "active 重复 open PORT_BUSY"],
      ["normal", "正常关闭 A / 重新打开"],
      ["grace", "异常断开 B / 宽限重连"],
      ["admin", "管理页关闭 / 恢复（集成版跳过）"],
      ["autostart", "开机自启状态（只读）"],
      ["cleanup", "清理会话"],
    ];
  }

  function renderRunner() {
    const box = $("#demo-runner-steps");
    if (!box) return;
    box.textContent = "";
    for (const step of state.runner.steps) {
      const li = document.createElement("li");
      li.dataset.id = step.id;
      const icon =
        step.status === "pass" ? "✓" : step.status === "fail" ? "✗" : step.status === "running" ? "…" : step.status === "skip" ? "−" : "·";
      li.appendChild(el("span", "st " + step.status, icon));
      li.appendChild(el("span", "", step.label));
      li.appendChild(el("span", "detail", step.detail || ""));
      box.appendChild(li);
    }
    const pass = state.runner.steps.filter((s) => s.status === "pass").length;
    const fail = state.runner.steps.filter((s) => s.status === "fail").length;
    const skip = state.runner.steps.filter((s) => s.status === "skip").length;
    $("#demo-runner-count").textContent =
      `${pass} / ${state.runner.steps.length}` + (skip ? ` · ${skip} skip` : "") + (fail ? ` · ${fail} FAIL` : "");
    $("#demo-runner-status").textContent = state.runner.running
      ? "运行中"
      : fail
        ? "有失败"
        : pass + skip === state.runner.steps.length
          ? skip
            ? "完成（已跳过）"
            : "全部通过"
          : "未运行";
    $("#demo-runner-status").className =
      "badge " + (fail ? "error" : pass + skip === state.runner.steps.length ? "active" : "");
  }

  function runnerSet(id, status, detail = "") {
    const step = state.runner.steps.find((s) => s.id === id);
    if (!step) return;
    step.status = status;
    step.detail = detail;
    renderRunner();
  }

  async function waitSessionState(sessionId, want, timeout = 6000) {
    const end = now() + timeout;
    while (now() < end) {
      if (want === "gone") {
        if (!state.sessions.has(sessionId)) return;
      } else {
        const session = state.sessions.get(sessionId);
        if (session?.state === want) return;
      }
      await sleep(100);
      try {
        await refreshSessions();
      } catch {
        // 轮询错误继续等待
      }
    }
    throw new Error(`等待会话 ${shortId(sessionId)} 状态 ${want} 超时`);
  }

  async function runDemo() {
    if (state.runner.running) return;
    state.runner.running = true;
    state.runner.steps = runnerSteps().map(([id, label]) => ({ id, label, status: "pending", detail: "" }));
    renderRunner();
    const mark = (id, status, detail) => runnerSet(id, status, detail);
    let a = null;
    let b = null;

    try {
      mark("connect", "running");
      await client.whenReady();
      mark("connect", "pass", `127.0.0.1:${client.managementPort}`);

      mark("ports", "running");
      const ports = await refreshPorts();
      mark("ports", "pass", `${ports.length} 个串口`);

      mark("select", "running");
      let aPath = $("#demo-path-A").value.trim();
      let bPath = $("#demo-path-B").value.trim();
      if (!aPath || !ports.some((p) => p.path === aPath)) aPath = ports[0]?.path;
      if (!bPath || !ports.some((p) => p.path === bPath) || bPath === aPath) {
        bPath = ports.find((p) => p.path !== aPath)?.path;
      }
      if (!aPath || !bPath) throw new Error("需要至少两个不同的串口");
      $("#demo-path-A").value = aPath;
      $("#demo-path-B").value = bPath;
      mark("select", "pass", `${aPath} ↔ ${bPath}`);

      await closeSlot("A");
      await closeSlot("B");

      mark("open", "running");
      a = await openSlot("A");
      b = await openSlot("B");
      mark("open", "pass", "A/B 已打开");

      mark("data", "running");
      if (a.state !== "active" || b.state !== "active") throw new Error("A/B 数据 WS 未 active");
      mark("data", "pass");

      mark("ab", "running");
      a.clearBuffer();
      b.clearBuffer();
      const p1 = pattern(4096);
      a.sendBytes(p1);
      const gotB = await b.takeBytes(p1.length, 6000);
      if (!bytesEqual(gotB, p1)) throw new Error(`A→B 内容不一致（${gotB.length}/${p1.length}）`);
      mark("ab", "pass", "4096 B 一致");

      mark("ba", "running");
      const p2 = pattern(4096);
      b.clearBuffer();
      a.clearBuffer();
      b.sendBytes(p2);
      const gotA = await a.takeBytes(p2.length, 6000);
      if (!bytesEqual(gotA, p2)) throw new Error(`B→A 内容不一致（${gotA.length}/${p2.length}）`);
      mark("ba", "pass", "4096 B 一致");

      mark("counters", "running");
      await refreshSessions();
      const sessionA = state.sessions.get(a.sessionId);
      const sessionB = state.sessions.get(b.sessionId);
      if (!sessionA || !sessionB) throw new Error("刷新后找不到 A/B 会话");
      mark(
        "counters",
        "pass",
        `A RX ${sessionA.counters.rxBytes}/TX ${sessionA.counters.txBytes}; B RX ${sessionB.counters.rxBytes}/TX ${sessionB.counters.txBytes}`,
      );

      mark("debug", "running");
      await client.subscribeDebug("all", true);
      state.debug.subscribed = true;
      state.debug.all = true;
      state.debug.ids.clear();
      state.debug.frames.length = 0;
      state.debug.gaps = 0;
      state.debug.lastSeq.clear();
      renderDebugOptions();
      renderDebugState();
      renderDebugFrames();
      const debugBefore = state.debug.frames.length;
      a.clearBuffer();
      b.clearBuffer();
      a.sendBytes(textEncoder.encode("DBG"));
      await b.takeBytes(3, 4000);
      await sleep(250);
      if (state.debug.frames.length <= debugBefore) throw new Error("未收到 debug.frame");
      mark("debug", "pass", `${state.debug.frames.length} 帧`);

      mark("debug-write", "running");
      a.clearBuffer();
      b.clearBuffer();
      const written = await client.request("session.write", {
        sessionId: a.sessionId,
        dataB64: b64encodeBytes(textEncoder.encode("MGT")),
      });
      const gotMgt = await b.takeBytes(3, 4000);
      if (!bytesEqual(gotMgt, textEncoder.encode("MGT"))) throw new Error("B 未收到调试写内容");
      mark("debug-write", "pass", `written=${written.written}`);

      mark("second", "running");
      const second = new WebSocket(a.dataUrl);
      second.binaryType = "arraybuffer";
      const secondCode = await waitClose(second, 4000);
      if (secondCode !== 4002) throw new Error(`第二客户端 close=${secondCode}，期望 4002`);
      mark("second", "pass", "4002");

      mark("busy", "running");
      try {
        await client.request("session.open", a.params);
        throw new Error("重复 open 未被拒绝");
      } catch (err) {
        if (err.code !== "PORT_BUSY") throw err;
      }
      mark("busy", "pass", "PORT_BUSY");

      mark("normal", "running");
      const oldAId = a.sessionId;
      await closeSlot("A", true);
      await waitSessionState(oldAId, "gone", 5000);
      a = await openSlot("A");
      mark("normal", "pass", "关闭 1000 + 重新打开");

      mark("grace", "running");
      b.disconnectAbnormal(3001);
      await waitSessionState(b.sessionId, "grace", 5000);
      await b.reconnectOld();
      await waitSessionState(b.sessionId, "active", 5000);
      mark("grace", "pass", "3001 → grace → active");

      mark("admin", "skip", "集成在管理页内，跳过关闭自身；可在概览页手动验证");
      mark("autostart", "pass", `当前 ${state.autostart ? "开启" : "关闭"}（只读）`);
      mark("cleanup", "running");
    } catch (err) {
      const running = state.runner.steps.find((s) => s.status === "running");
      if (running) mark(running.id, "fail", err.message);
      for (const step of state.runner.steps) {
        if (step.status === "pending") step.status = "skip";
      }
      logEvent("error", "一键演示: " + err.message);
    } finally {
      try {
        await closeSlot("A");
        await closeSlot("B");
        if (state.debug.subscribed) {
          await client.unsubscribeDebug().catch(() => {});
          state.debug.subscribed = false;
          state.debug.ids.clear();
          renderDebugOptions();
          renderDebugState();
        }
        await refreshSessions();
        mark("cleanup", "pass", "会话已清理");
      } catch (err) {
        mark("cleanup", "fail", err.message);
      }
      state.runner.running = false;
      renderRunner();
      logEvent("info", "一键演示结束");
    }
  }


  // ----------------------------- tabs / init -----------------------------
  const tabs = $$("[data-demo-tab]");
  const panels = $$(".demo-panel");
  let current = "overview";

  function activate(name) {
    current = name;
    for (const btn of tabs) {
      const active = btn.dataset.demoTab === name;
      btn.classList.toggle("active", active);
      btn.setAttribute("aria-selected", String(active));
      btn.tabIndex = active ? 0 : -1;
    }
    for (const panel of panels) {
      panel.classList.toggle("active", panel.id === `demo-tab-${name}`);
    }
    if (name === "terminal") renderTerminal();
  }

  tabs.forEach((btn, index) => {
    btn.addEventListener("click", () => activate(btn.dataset.demoTab));
    btn.addEventListener("keydown", (event) => {
      if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") return;
      event.preventDefault();
      const delta = event.key === "ArrowRight" ? 1 : -1;
      const next = (index + delta + tabs.length) % tabs.length;
      tabs[next].focus();
      activate(tabs[next].dataset.demoTab);
    });
  });

  function bindCoreUI() {
    $("#demo-btn-refresh-all").addEventListener("click", () => {
      refreshPorts().catch((err) => logEvent("error", err.message));
      refreshSessions().catch((err) => logEvent("error", err.message));
    });
    $("#demo-btn-refresh-ports").addEventListener("click", () => {
      refreshPorts().catch((err) => logEvent("error", err.message));
    });
    $("#demo-btn-open-a").addEventListener("click", () => {
      openSlot("A").catch((err) => {
        $("#demo-open-result").textContent = err.message;
        logEvent("error", err.message);
      });
    });
    $("#demo-btn-open-b").addEventListener("click", () => {
      openSlot("B").catch((err) => {
        $("#demo-open-result").textContent = err.message;
        logEvent("error", err.message);
      });
    });
    $("#demo-btn-close-a").addEventListener("click", () => closeSlot("A").catch((err) => logEvent("error", err.message)));
    $("#demo-btn-close-b").addEventListener("click", () => closeSlot("B").catch((err) => logEvent("error", err.message)));
    $("#demo-auto-reconnect").addEventListener("change", () => {
      state.autoReconnect = $("#demo-auto-reconnect").checked;
      for (const slot of ["A", "B"]) {
        if (state.slots[slot]) state.slots[slot].autoReconnect = state.autoReconnect;
      }
    });
    for (const slot of ["A", "B"]) {
      bindTerminalPrefs(slot);
      $(`#demo-sendbtn-${slot}`).addEventListener("click", () => sendFromSlot(slot));
      $(`#demo-send-${slot}`).addEventListener("keydown", (event) => {
        if (event.key === "Enter") sendFromSlot(slot);
      });
    }
    $$("[data-term]").forEach((btn) => {
      btn.addEventListener("click", async () => {
        const slot = btn.dataset.slot;
        const controller = state.slots[slot];
        try {
          if (!controller) throw new Error(`${slot} 未打开`);
          if (btn.dataset.term === "clear") {
            controller.frames = [];
            controller.frameBytes = 0;
            renderTerminalLog(slot);
          } else if (btn.dataset.term === "export") {
            exportTerminal(slot);
          } else if (btn.dataset.term === "reconnect") {
            await controller.reconnectOld();
            toast(`${slot} 重连成功`, "ok");
          } else if (btn.dataset.term === "abnormal") {
            controller.disconnectAbnormal();
            toast(`${slot} 已模拟异常断开，等待 grace`, "warn");
          } else if (btn.dataset.term === "close") {
            await closeSlot(slot, true);
          }
        } catch (err) {
          logEvent("error", `${slot}: ${err.message}`);
        }
      });
    });
    $("#demo-btn-loop-ab").addEventListener("click", () => {
      loopback("A", "B").catch((err) => {
        $("#demo-loop-result").textContent = err.message;
        logEvent("error", err.message);
      });
    });
    $("#demo-btn-loop-ba").addEventListener("click", () => {
      loopback("B", "A").catch((err) => {
        $("#demo-loop-result").textContent = err.message;
        logEvent("error", err.message);
      });
    });
    $("#demo-event-filter").addEventListener("input", renderEvents);
    $("#demo-btn-event-clear").addEventListener("click", () => {
      state.events.length = 0;
      renderEvents();
    });
    $("#demo-btn-event-export").addEventListener("click", () => {
      download(`ws2s-events-${Date.now()}.json`, JSON.stringify(state.events, null, 2), "application/json");
    });
    $("#demo-btn-run-demo").addEventListener("click", () => {
      runDemo();
    });
    $("#demo-debug-all").addEventListener("change", () => {
      state.debug.all = $("#demo-debug-all").checked;
      renderDebugOptions();
    });
    $("#demo-btn-debug-subscribe").addEventListener("click", () => {
      subscribeDebug().catch((err) => logEvent("error", err.message));
    });
    $("#demo-btn-debug-unsubscribe").addEventListener("click", () => {
      unsubscribeDebug().catch((err) => logEvent("error", err.message));
    });
    $("#demo-btn-debug-export").addEventListener("click", exportDebug);
    $("#demo-btn-debug-clear").addEventListener("click", () => {
      state.debug.frames.length = 0;
      state.debug.gaps = 0;
      state.debug.lastSeq.clear();
      renderDebugFrames();
    });
    $("#demo-btn-debug-write").addEventListener("click", () => {
      debugWrite().catch((err) => {
        $("#demo-debug-write-result").textContent = err.message;
        logEvent("error", err.message);
      });
    });
    $$("[data-edge]").forEach((btn) => {
      btn.addEventListener("click", () => {
        const handlers = {
          "second-client": edgeSecondClient,
          "active-open": edgeActiveOpen,
          "bad-token": edgeBadToken,
          "missing-port": edgeMissingPort,
          "write-inactive": edgeWriteInactive,
          "unknown-action": edgeUnknownAction,
          "text-frame": edgeTextFrame,
          oversize: edgeOversize,
          abnormal: edgeAbnormal,
          "normal-close": edgeNormalClose,
        };
        const fn = handlers[btn.dataset.edge];
        if (fn) Promise.resolve().then(fn).catch((err) => edgeLog(`失败: ${err.message}`));
      });
    });
  }

  function renderOrigin() {
    const origin = location.origin;
    const originEl = $("#demo-page-origin");
    if (originEl) originEl.textContent = origin;
    const mgmtEl = $("#demo-mgmt-url");
    if (mgmtEl) mgmtEl.textContent = `ws://127.0.0.1:${client.managementPort}/ws`;
    const loopback = ["127.0.0.1", "localhost", "[::1]"].includes(location.hostname);
    if (!loopback) {
      toast(`当前页面不是从 127.0.0.1/localhost 打开的：${origin}`, "err");
    }
  }

  function initPrefs() {
    for (const slot of ["A", "B"]) {
      state.terminalPrefs[slot] = {
        hex: $(`#demo-hex-${slot}`).checked,
        ts: $(`#demo-ts-${slot}`).checked,
        autoscroll: $(`#demo-autoscroll-${slot}`).checked,
        paused: $(`#demo-paused-${slot}`).checked,
      };
    }
  }

  function poll() {
    if (!client.connected || state.pollBusy) return;
    state.pollBusy = true;
    refreshSessions()
      .catch(() => {})
      .finally(() => {
        state.pollBusy = false;
      });
  }

  bindCoreUI();
  renderOrigin();
  initPrefs();
  renderAdmin();
  renderEvents();
  renderTerminal();
  renderDebugOptions();
  renderDebugFrames();
  state.runner.steps = runnerSteps().map(([id, label]) => ({ id, label, status: "pending", detail: "" }));
  renderRunner();
  activate(current);
  setInterval(() => {
    for (const slot of ["A", "B"]) state.slots[slot]?._renderMeta();
  }, 500);
  setInterval(poll, 1500);

  return {
    onShow() {
      activate(current);
      renderTerminal();
      if (current === "debug") {
        renderDebugOptions();
        renderDebugFrames();
      }
    },
  };
}
