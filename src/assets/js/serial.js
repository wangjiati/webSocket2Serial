// 数据 WebSocket 会话：管理面负责打开/关闭，数据面负责二进制收发。
// 不直接操作 DOM；页面通过 hooks 更新日志、状态与统计。

import { sleep, now, textEncoder, hexToBytes } from "./format.js";

export class SerialSession {
  /**
   * @param {string} slot 页面内的逻辑槽位，仅用于日志与展示
   * @param {{sessionId:string,dataUrl:string,state?:string,counters?:object}} info
   * @param {object} params session.open 参数
   * @param {import("./api.js").ManagementClient} client
   * @param {{onFrame?:(frame:object)=>void,onState?:(session:object)=>void,onLog?:(level:string,message:string)=>void}} hooks
   */
  constructor(slot, info, params, client, hooks = {}) {
    this.slot = slot;
    this.sessionId = info.sessionId;
    this.dataUrl = info.dataUrl;
    this.params = params;
    this.mgmt = client;
    this.hooks = hooks;
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
  }

  _log(level, message) {
    this.hooks.onLog?.(level, message);
  }

  _notifyState() {
    this.hooks.onState?.(this);
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
        this._notifyState();
        this._log("ok", `${this.slot} 数据 WS 已连接 (${url.split("/").pop()?.slice(0, 8)})`);
        resolve();
      };
      ws.onmessage = (event) => {
        if (!(event.data instanceof ArrayBuffer)) {
          this._log("error", `${this.slot} 数据 WS 收到非二进制帧`);
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
        this._notifyState();
        this._log(
          event.code === 1000 ? "ok" : "warn",
          `${this.slot} 数据 WS 关闭 code=${event.code} ${event.reason || ""}`,
        );
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
        this._log("ok", `${this.slot} 自动重连成功`);
      } catch (err) {
        this._log("error", `${this.slot} 自动重连失败: ${err.message}`);
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
    this._notifyState();
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
    this._notifyState();
  }

  updateInfo(info) {
    if (!info) return;
    this.state = info.state;
    this.counters = info.counters || this.counters;
    this.graceEndsAt = info.graceEndsAt || null;
    this.lastGap = info.lastGap || null;
    this.lastError = info.lastError || null;
    if (info.closeCode != null) this.closeCode = info.closeCode;
    this._notifyState();
  }

  _appendFrame(dir, bytes) {
    const frame = { ts: now(), dir, bytes: new Uint8Array(bytes) };
    this.frames.push(frame);
    this.frameBytes += frame.bytes.length;
    while (this.frames.length > 5000 || this.frameBytes > 4 * 1024 * 1024) {
      const removed = this.frames.shift();
      this.frameBytes -= removed.bytes.length;
    }
    this.hooks.onFrame?.(frame);
  }
}
