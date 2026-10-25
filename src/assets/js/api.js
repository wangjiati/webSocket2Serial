// 管理面 WS 客户端：固定回环地址、自动重连、hello 后 session.list/ports.list 对齐、
// 保留调试订阅并在重连后恢复。管理页自身不打开数据 WS。

const WS_BASE = "127.0.0.1";
const REQUEST_TIMEOUT_MS = 10000;
const RECONNECT_BASE_MS = 500;
const RECONNECT_MAX_MS = 5000;

/** 读取 `/api/config`，返回 `{managementPort, version, admin, autostart}`。 */
export async function getConfig() {
  const resp = await fetch("/api/config", { cache: "no-store" });
  if (!resp.ok) {
    throw new Error(`获取配置失败：HTTP ${resp.status}`);
  }
  return resp.json();
}

let idCounter = 0;

function nextId() {
  idCounter += 1;
  return `c${Date.now().toString(36)}-${idCounter}`;
}

export class ManagementClient {
  constructor(managementPort) {
    this.managementPort = managementPort;
    // 固定回环地址，不用 location.hostname。
    this.url = `ws://${WS_BASE}:${managementPort}/ws`;
    this.ws = null;
    this.closedByUser = false;
    this.connected = false;
    this.reconnectAttempts = 0;
    this.reconnectTimer = null;
    this.pending = new Map();
    this.eventHandlers = new Set();
    this.syncHandlers = new Set();
    this.connectionHandlers = new Set();
    this.subscription = null;
    this.sessions = [];
    this.ports = [];
    this.hello = null;
    this.admin = null;
  }

  connect() {
    this.closedByUser = false;
    this.open();
  }

  open() {
    if (this.ws) {
      return;
    }
    let ws;
    try {
      ws = new WebSocket(this.url);
    } catch (err) {
      this.scheduleReconnect();
      return;
    }
    this.ws = ws;
    ws.onopen = () => {
      this.connected = true;
      this.reconnectAttempts = 0;
      this.emitConnection(true);
    };
    ws.onmessage = (evt) => {
      this.handleMessage(evt.data);
    };
    ws.onclose = () => {
      this.handleClose();
    };
    ws.onerror = () => {
      // 关闭由 onclose 统一处理，这里仅占位避免未捕获错误。
    };
  }

  handleMessage(raw) {
    let msg;
    try {
      msg = JSON.parse(raw);
    } catch (err) {
      return;
    }
    if (msg && typeof msg.event === "string") {
      this.dispatchEvent(msg.event, msg.data);
    } else if (msg && Object.prototype.hasOwnProperty.call(msg, "id")) {
      this.resolveResponse(msg);
    }
  }

  dispatchEvent(name, data) {
    if (name === "hello") {
      this.hello = data || null;
      this.admin = data && data.admin ? data.admin : null;
      this.align();
      this.restoreSubscription();
    } else if (name === "session.state") {
      this.refreshSessions();
    } else if (name === "ports.changed") {
      this.refreshPorts();
    } else if (name === "admin.state") {
      this.admin = data || null;
      this.notifySync();
    }
    for (const cb of this.eventHandlers) {
      try {
        cb(name, data);
      } catch (err) {
        // 视图回调异常不应中断事件分发。
      }
    }
  }

  async align() {
    await Promise.allSettled([this.refreshSessions(), this.refreshPorts()]);
    this.notifySync();
  }

  async refreshSessions() {
    try {
      const data = await this.request("session.list", {});
      this.sessions = (data && data.sessions) || [];
      this.notifySync();
    } catch (err) {
      // 连接中断等：保持旧数据。
    }
  }

  async refreshPorts() {
    try {
      const data = await this.request("ports.list", {});
      this.ports = (data && data.ports) || [];
      this.notifySync();
    } catch (err) {
      // 连接中断等：保持旧数据。
    }
  }

  restoreSubscription() {
    if (this.subscription) {
      this.request("debug.subscribe", this.subscription).catch(() => {});
    }
  }

  subscribeDebug(sessionIds, replay) {
    this.subscription = { sessionIds, replay: !!replay };
    if (!this.connected || !this.ws || this.ws.readyState !== WebSocket.OPEN) {
      return undefined;
    }
    return this.request("debug.subscribe", this.subscription);
  }

  request(action, params) {
    const payload = { id: nextId(), action, params: params || {} };
    return new Promise((resolve, reject) => {
      if (!this.connected || !this.ws || this.ws.readyState !== WebSocket.OPEN) {
        reject(new Error(`未连接管理服务（${action}）`));
        return;
      }
      const timer = setTimeout(() => {
        this.pending.delete(payload.id);
        reject(new Error(`请求超时（${action}）`));
      }, REQUEST_TIMEOUT_MS);
      this.pending.set(payload.id, { resolve, reject, timer });
      try {
        this.ws.send(JSON.stringify(payload));
      } catch (err) {
        clearTimeout(timer);
        this.pending.delete(payload.id);
        reject(err);
      }
    });
  }

  resolveResponse(msg) {
    const entry = this.pending.get(msg.id);
    if (!entry) {
      return;
    }
    clearTimeout(entry.timer);
    this.pending.delete(msg.id);
    if (msg.ok) {
      entry.resolve(msg.data);
    } else {
      const err = msg.error || { code: "UNKNOWN", message: "未知错误" };
      const error = new Error(err.message || err.code);
      error.code = err.code;
      error.body = err;
      entry.reject(error);
    }
  }

  onEvent(cb) {
    this.eventHandlers.add(cb);
  }

  onSync(cb) {
    this.syncHandlers.add(cb);
  }

  onConnection(cb) {
    this.connectionHandlers.add(cb);
  }

  notifySync() {
    for (const cb of this.syncHandlers) {
      try {
        cb(this);
      } catch (err) {
        // 忽略视图错误。
      }
    }
  }

  emitConnection(connected) {
    for (const cb of this.connectionHandlers) {
      try {
        cb(connected);
      } catch (err) {
        // 忽略视图错误。
      }
    }
  }

  handleClose() {
    this.ws = null;
    this.connected = false;
    this.emitConnection(false);
    for (const [, entry] of this.pending) {
      clearTimeout(entry.timer);
      entry.reject(new Error("连接中断"));
    }
    this.pending.clear();
    if (!this.closedByUser) {
      this.scheduleReconnect();
    }
  }

  scheduleReconnect() {
    if (this.closedByUser || this.reconnectTimer) {
      return;
    }
    const delay = Math.min(
      RECONNECT_MAX_MS,
      RECONNECT_BASE_MS * Math.pow(2, this.reconnectAttempts),
    );
    this.reconnectAttempts += 1;
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.open();
    }, delay);
  }

  close() {
    this.closedByUser = true;
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    for (const [, entry] of this.pending) {
      clearTimeout(entry.timer);
      entry.reject(new Error("连接已关闭"));
    }
    this.pending.clear();
    if (this.ws) {
      try {
        this.ws.close();
      } catch (err) {
        // 忽略关闭错误。
      }
      this.ws = null;
    }
    this.connected = false;
  }
}
