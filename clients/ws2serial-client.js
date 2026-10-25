// WebSocket2Serial 参考客户端（无构建步骤的 ES module）。
//
// 封装管理 WS + 数据 WS 的最小接入逻辑，供生产网页与第三方脚本参考/复用：
//   - 管理 WS 固定 `ws://127.0.0.1:<managementPort>/ws`，自动重连；`hello` 后
//     用 `session.list` 对齐状态（不依赖 clientId / localStorage 里的 session id）。
//   - 数据 WS `binaryType = "arraybuffer"`，二进制原样透传。
//   - 串口参数由「客户端侧」保存（本对象持有并复用），断线/刷新后重新 `session.open`。
//   - 数据断开先试原 `dataUrl`（宽限期内 token 仍有效，可 resume）；失败再
//     `session.open`（复用同端口非 active 会话并轮换 token）；`PORT_BUSY` 按
//     200–500ms 退避重试。
//   - 只打开管理 WS 与数据 WS 两类连接，不访问 HTTP 管理页、不自行实现串口。

const WS_BASE = "127.0.0.1";
const MGMT_PATH = "/ws";
const REQUEST_TIMEOUT_MS = 10000;

// 管理 WS 重连退避：500ms 起步，翻倍封顶 5s。
const RECONNECT_BASE_MS = 500;
const RECONNECT_MAX_MS = 5000;

// 数据面重连 / PORT_BUSY 退避：首次 200–500ms（随机），翻倍封顶 5s。
const BACKOFF_FIRST_MIN_MS = 200;
const BACKOFF_FIRST_MAX_MS = 500;
const BACKOFF_MAX_MS = 5000;

// 会话已彻底关闭、再试原 dataUrl 无意义的数据面 close code。
const SESSION_DEAD_CODES = new Set([4001, 4004, 4005, 4007]);

let idCounter = 0;

function nextId() {
  idCounter += 1;
  return `c${Date.now().toString(36)}-${idCounter}`;
}

/** 首次 200–500ms、翻倍封顶 5s 的重试退避（毫秒）。 */
function backoffMs(attempt) {
  const first =
    BACKOFF_FIRST_MIN_MS +
    Math.floor(Math.random() * (BACKOFF_FIRST_MAX_MS - BACKOFF_FIRST_MIN_MS + 1));
  if (attempt <= 0) {
    return first;
  }
  return Math.min(BACKOFF_MAX_MS, first * Math.pow(2, attempt));
}

/** 管理 WS 重连退避：500ms 起步翻倍封顶 5s。 */
function reconnectMs(attempt) {
  return Math.min(RECONNECT_MAX_MS, RECONNECT_BASE_MS * Math.pow(2, attempt));
}

/**
 * 参考客户端。
 *
 * 构造参数（均为可选）：
 *   - managementPort：管理 WS 端口（默认 8765）。
 *   - serialParams：`session.open` 参数（客户端侧保存，刷新/重连时复用）。
 *   - onData(bytes: Uint8Array)：收到串口→网页的二进制数据。
 *   - onDataClose(code, reason)：数据 WS 关闭（含 server 下发的 close code）。
 *   - onSession(sessionInfo)：`session.open` 成功或 `session.list` 对齐后的会话信息。
 *   - onConnection(connected)：管理 WS 连接状态变化。
 */
export class Ws2SerialClient {
  constructor(options = {}) {
    this.managementPort = options.managementPort ?? 8765;
    this.serialParams = options.serialParams ?? null;

    this.onData = options.onData ?? (() => {});
    this.onDataClose = options.onDataClose ?? (() => {});
    this.onSession = options.onSession ?? (() => {});
    this.onConnection = options.onConnection ?? (() => {});

    // 管理 WS
    this.mgmt = null;
    this.mgmtClosedByUser = false;
    this.mgmtTimer = null;
    this.mgmtAttempts = 0;
    this.pending = new Map();

    // 数据 WS
    this.data = null;
    this.dataUrl = null;
    this.sessionId = null;
    this.dataClosedByUser = false;
    this.dataTimer = null;
    this.dataAttempts = 0;
    this.dataOpened = false;
    // 原 dataUrl 是否已在本次断线中「重试过一次」（避免反复试已失效的 token）。
    this.dataUrlTried = false;
  }

  // ---------------------------------------------------------------------
  // 管理 WS
  // ---------------------------------------------------------------------

  /** 启动管理连接（幂等；断开后自动重连）。 */
  connect() {
    this.mgmtClosedByUser = false;
    this._openManagement();
  }

  _openManagement() {
    if (this.mgmt) {
      return;
    }
    let ws;
    try {
      ws = new WebSocket(`ws://${WS_BASE}:${this.managementPort}${MGMT_PATH}`);
    } catch (err) {
      this._scheduleMgmtReconnect();
      return;
    }
    this.mgmt = ws;
    ws.onopen = () => {
      this.mgmtAttempts = 0;
      this.onConnection(true);
    };
    ws.onmessage = (evt) => this._handleMgmtMessage(evt.data);
    ws.onclose = () => this._handleMgmtClose();
    ws.onerror = () => {
      // 关闭统一由 onclose 处理，这里占位避免未捕获错误。
    };
  }

  _handleMgmtMessage(raw) {
    let msg;
    try {
      msg = JSON.parse(raw);
    } catch (err) {
      return;
    }
    if (msg && typeof msg.event === "string") {
      this._dispatchEvent(msg.event, msg.data);
    } else if (msg && Object.prototype.hasOwnProperty.call(msg, "id")) {
      this._resolveResponse(msg);
    }
  }

  _dispatchEvent(name, data) {
    if (name === "hello") {
      // hello 后立即 session.list 对齐，避免重连期间错过状态事件。
      this._align();
    }
  }

  async _align() {
    try {
      await this.request("session.list", {});
    } catch (err) {
      // 连接中断等：下次重连再对齐。
    }
  }

  _handleMgmtClose() {
    this.mgmt = null;
    this.onConnection(false);
    for (const [, entry] of this.pending) {
      clearTimeout(entry.timer);
      entry.reject(new Error("管理连接中断"));
    }
    this.pending.clear();
    if (!this.mgmtClosedByUser) {
      this._scheduleMgmtReconnect();
    }
  }

  _scheduleMgmtReconnect() {
    if (this.mgmtClosedByUser || this.mgmtTimer) {
      return;
    }
    const delay = reconnectMs(this.mgmtAttempts);
    this.mgmtAttempts += 1;
    this.mgmtTimer = setTimeout(() => {
      this.mgmtTimer = null;
      this._openManagement();
    }, delay);
  }

  /** 管理请求：`{id, action, params}` → Promise<data>。 */
  request(action, params) {
    const payload = { id: nextId(), action, params: params || {} };
    return new Promise((resolve, reject) => {
      if (!this.mgmt || this.mgmt.readyState !== WebSocket.OPEN) {
        reject(new Error(`管理服务未连接（${action}）`));
        return;
      }
      const timer = setTimeout(() => {
        this.pending.delete(payload.id);
        reject(new Error(`请求超时（${action}）`));
      }, REQUEST_TIMEOUT_MS);
      this.pending.set(payload.id, { resolve, reject, timer });
      try {
        this.mgmt.send(JSON.stringify(payload));
      } catch (err) {
        clearTimeout(timer);
        this.pending.delete(payload.id);
        reject(err);
      }
    });
  }

  _resolveResponse(msg) {
    const entry = this.pending.get(msg.id);
    if (!entry) {
      return;
    }
    clearTimeout(entry.timer);
    this.pending.delete(msg.id);
    if (msg.ok) {
      entry.resolve(msg.data);
    } else {
      const err = msg.error || { code: "INTERNAL", message: "未知错误" };
      const error = new Error(err.message || err.code);
      error.code = err.code;
      error.body = err;
      entry.reject(error);
    }
  }

  // ---------------------------------------------------------------------
  // 数据 WS 与 session.open
  // ---------------------------------------------------------------------

  /**
   * 打开串口并建立数据 WS。`params` 缺省时复用构造时保存的 `serialParams`。
   * 刷新/重启后直接调用本方法即可接回（同端口非 active 会话复用并轮换 token）。
   */
  async open(params) {
    if (params) {
      this.serialParams = params;
    }
    if (!this.serialParams) {
      throw new Error("缺少串口参数：先传入 serialParams 或 open(params)");
    }
    this.dataClosedByUser = false;
    this.dataAttempts = 0;
    this.dataUrlTried = false;
    return this._openViaSession();
  }

  /** 通过 `session.open` 取得（或复用）会话并连接其数据 URL。 */
  async _openViaSession() {
    let result;
    try {
      result = await this.request("session.open", this.serialParams);
    } catch (err) {
      // PORT_BUSY：旧连接尚未释放（同端口已有 active 会话），退避重试。
      if (err.code === "PORT_BUSY") {
        this._scheduleDataReconnect();
        return null;
      }
      throw err;
    }
    this.sessionId = result.sessionId;
    this.dataUrl = result.dataUrl;
    if (this.onSession) {
      this.onSession(result);
    }
    this._openData(this.dataUrl);
    return result;
  }

  /** 建立数据 WS（binaryType=arraybuffer，二进制透传）。 */
  _openData(url) {
    this._closeDataSocket();
    this.dataOpened = false;
    let ws;
    try {
      ws = new WebSocket(url);
    } catch (err) {
      this._scheduleDataReconnect();
      return;
    }
    this.data = ws;
    ws.binaryType = "arraybuffer";
    ws.onopen = () => {
      this.dataOpened = true;
      this.dataAttempts = 0;
      this.dataUrlTried = false;
    };
    ws.onmessage = (evt) => {
      if (evt.data instanceof ArrayBuffer) {
        this.onData(new Uint8Array(evt.data));
      }
    };
    ws.onclose = (evt) => this._handleDataClose(evt.code);
    ws.onerror = () => {
      // 关闭统一由 onclose 处理。
    };
  }

  _handleDataClose(code) {
    this.data = null;
    this.dataOpened = false;
    this.onDataClose(code, "");
    if (this.dataClosedByUser) {
      return;
    }
    // 会话已彻底关闭的 close code 直接走 session.open，不再试原 URL。
    if (SESSION_DEAD_CODES.has(code)) {
      this.dataUrl = null;
    }
    this._scheduleDataReconnect();
  }

  /** 数据断开：先试原 dataUrl（宽限期内可 resume），失败再 session.open。 */
  _scheduleDataReconnect() {
    if (this.dataClosedByUser || this.dataTimer) {
      return;
    }
    const delay = backoffMs(this.dataAttempts);
    this.dataAttempts += 1;
    this.dataTimer = setTimeout(() => {
      this.dataTimer = null;
      this._dataReconnectStep();
    }, delay);
  }

  /** 单次重连：优先试原 dataUrl 一次，失败后走 session.open（复用并轮换 token）。 */
  _dataReconnectStep() {
    if (this.dataClosedByUser) {
      return;
    }
    if (this.dataUrl && !this.dataUrlTried) {
      // 宽限期内 token 仍有效：重连原 dataUrl 可 resume，且不轮换 token。
      this.dataUrlTried = true;
      this._openData(this.dataUrl);
      return;
    }
    // 原 URL 已试过（或会话已死）：重新 session.open，复用同端口非 active 会话。
    this.dataUrlTried = false;
    this._openViaSession().catch(() => {});
  }

  /** 向串口写入二进制数据。 */
  send(bytes) {
    if (!this.data || this.data.readyState !== WebSocket.OPEN) {
      throw new Error("数据连接未建立");
    }
    const payload = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    this.data.send(payload.buffer.slice(payload.byteOffset, payload.byteOffset + payload.byteLength));
  }

  // ---------------------------------------------------------------------
  // 便捷动作（全部走管理 WS）
  // ---------------------------------------------------------------------

  /** 列出当前串口快照。 */
  listPorts() {
    return this.request("ports.list", {}).then((data) => data.ports || []);
  }

  /** 列出当前会话快照。 */
  listSessions() {
    return this.request("session.list", {}).then((data) => data.sessions || []);
  }

  /** 关闭指定会话（管理端）。 */
  closeSession(sessionId) {
    return this.request("session.close", { sessionId });
  }

  /** 调试写：向 active 会话写原始字节（base64 编码后走 `session.write`）。 */
  writeDebug(sessionId, bytes) {
    const data = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    let binary = "";
    for (let i = 0; i < data.length; i += 1) {
      binary += String.fromCharCode(data[i]);
    }
    const dataB64 = btoa(binary);
    return this.request("session.write", { sessionId, dataB64 });
  }

  /** 订阅调试帧；`sessionIds` 为 `"all"` 或 id 数组，`replay` 可选。 */
  subscribeDebug(sessionIds, replay) {
    return this.request("debug.subscribe", { sessionIds, replay: !!replay });
  }

  /** 运行时开关管理页（不回写配置）。 */
  setAdmin(enabled) {
    return this.request("admin.set", { enabled });
  }

  // ---------------------------------------------------------------------
  // 关闭
  // ---------------------------------------------------------------------

  /** 关闭数据连接与（可选）会话，并关闭管理连接。 */
  async close() {
    this.dataClosedByUser = true;
    if (this.dataTimer) {
      clearTimeout(this.dataTimer);
      this.dataTimer = null;
    }
    this._closeDataSocket();
    if (this.sessionId && this.mgmt && this.mgmt.readyState === WebSocket.OPEN) {
      try {
        await this.request("session.close", { sessionId: this.sessionId });
      } catch (err) {
        // 会话可能已关闭；忽略。
      }
    }
    this.sessionId = null;
    this.dataUrl = null;

    this.mgmtClosedByUser = true;
    if (this.mgmtTimer) {
      clearTimeout(this.mgmtTimer);
      this.mgmtTimer = null;
    }
    for (const [, entry] of this.pending) {
      clearTimeout(entry.timer);
      entry.reject(new Error("客户端已关闭"));
    }
    this.pending.clear();
    if (this.mgmt) {
      try {
        this.mgmt.close();
      } catch (err) {
        // 忽略关闭错误。
      }
      this.mgmt = null;
    }
  }

  _closeDataSocket() {
    if (this.data) {
      this.data.onopen = null;
      this.data.onmessage = null;
      this.data.onclose = null;
      this.data.onerror = null;
      try {
        this.data.close();
      } catch (err) {
        // 忽略关闭错误。
      }
      this.data = null;
    }
  }
}
