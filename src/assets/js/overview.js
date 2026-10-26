// 概览视图：版本 / 管理地址（复制）、管理服务开关、串口列表、会话表。

const STATE_LABELS = {
  awaiting: "等待连接",
  active: "已连接",
  grace: "宽限中",
  closed: "已关闭",
};

export function bindOverview(root, client, state) {
  const versionEl = root.querySelector("#version");
  const adminUrlEl = root.querySelector("#admin-url");
  const copyBtn = root.querySelector("#copy-url");
  const toggle = root.querySelector("#admin-toggle");
  const adminNote = root.querySelector("#admin-note");
  const autostartEl = root.querySelector("#autostart");
  const portsBody = root.querySelector("#ports-body");
  const sessionsBody = root.querySelector("#sessions-body");
  const connStatus = document.getElementById("conn-status");

  if (state && state.config && state.config.version) {
    versionEl.textContent = state.config.version;
  }

  client.onConnection((connected) => {
    if (connStatus) {
      connStatus.textContent = connected ? "已连接" : "连接中…";
      connStatus.className = "badge " + (connected ? "ok" : "waiting");
    }
  });

  client.onSync(() => {
    renderInfo();
    renderPorts();
    renderSessions();
  });

  // 管理页只做监控：每 2 秒刷新一次会话计数，页面隐藏时暂停。
  setInterval(() => {
    if (!document.hidden) {
      client.refreshSessions();
    }
  }, 2000);

  copyBtn.addEventListener("click", async () => {
    const url = client.admin && client.admin.url ? client.admin.url : adminUrlEl.textContent;
    if (!url || url === "—") {
      return;
    }
    try {
      await navigator.clipboard.writeText(url);
      flash(copyBtn, "已复制");
    } catch (err) {
      // 剪贴板不可用（非安全上下文）：回退为选中文本。
      const sel = window.getSelection();
      const range = document.createRange();
      range.selectNodeContents(adminUrlEl);
      sel.removeAllRanges();
      sel.addRange(range);
      flash(copyBtn, "请 Ctrl+C");
    }
  });

  toggle.addEventListener("change", async () => {
    const target = toggle.checked;
    if (!target) {
      const ok = window.confirm("关闭管理服务后，刷新页面将无法访问本管理页，确定关闭？");
      if (!ok) {
        toggle.checked = !target;
        return;
      }
    }
    toggle.disabled = true;
    try {
      await client.request("admin.set", { enabled: target });
      // admin.state 事件会触发 sync 更新 UI。
    } catch (err) {
      toggle.checked = !target;
      window.alert("切换管理服务失败：" + err.message);
    } finally {
      toggle.disabled = false;
    }
  });

  function renderInfo() {
    if (client.hello) {
      versionEl.textContent = `${client.hello.version}（${client.hello.os}/${client.hello.arch}）`;
    }
    const admin = client.admin;
    if (admin) {
      adminUrlEl.textContent = admin.url || "—";
      toggle.checked = !!admin.enabled;
      adminNote.textContent = admin.enabled ? "" : "已关闭：刷新后无法访问";
      adminNote.className = "muted" + (admin.enabled ? "" : " warn");
    }
    const autostart = client.hello?.autostart ?? state?.config?.autostart;
    if (autostartEl) {
      autostartEl.textContent = autostart == null ? "—" : `${autostart ? "开启" : "关闭"}（只读）`;
    }
  }

  function renderPorts() {
    const ports = client.ports || [];
    portsBody.textContent = "";
    if (ports.length === 0) {
      portsBody.appendChild(emptyRow(7, "未发现串口"));
      return;
    }
    for (const p of ports) {
      const tr = document.createElement("tr");
      tr.appendChild(td(p.path));
      tr.appendChild(td(vidPid(p)));
      tr.appendChild(td(p.manufacturer || "—"));
      tr.appendChild(td(p.product || "—"));
      tr.appendChild(td(p.serialNumber || "—"));
      tr.appendChild(td(p.portType || "—"));
      const action = document.createElement("td");
      const link = document.createElement("a");
      link.href = `/assistant?port=${encodeURIComponent(p.path)}`;
      link.target = "_blank";
      link.rel = "noopener";
      link.textContent = "用助手打开";
      action.appendChild(link);
      tr.appendChild(action);
      portsBody.appendChild(tr);
    }
  }

  function renderSessions() {
    const sessions = client.sessions || [];
    sessionsBody.textContent = "";
    if (sessions.length === 0) {
      sessionsBody.appendChild(emptyRow(8, "无活动会话"));
      return;
    }
    for (const s of sessions) {
      const tr = document.createElement("tr");
      tr.appendChild(td(shortId(s.sessionId)));
      tr.appendChild(td(STATE_LABELS[s.state] || s.state));
      tr.appendChild(td(`${s.port} · ${formatParams(s.params)}`));
      tr.appendChild(td(`${s.counters.rxBytes} / ${s.counters.txBytes}`));
      tr.appendChild(td(String(s.counters.rxDiscardedBytes ?? 0)));
      tr.appendChild(td(formatGap(s.lastGap)));
      tr.appendChild(td(formatError(s.lastError)));
      const closeBtn = document.createElement("button");
      closeBtn.type = "button";
      closeBtn.className = "small";
      closeBtn.textContent = "关闭";
      closeBtn.addEventListener("click", () => closeSession(s.sessionId, closeBtn));
      const tdBtn = document.createElement("td");
      tdBtn.appendChild(closeBtn);
      tr.appendChild(tdBtn);
      sessionsBody.appendChild(tr);
    }
  }

  async function closeSession(sessionId, btn) {
    if (!window.confirm(`确定关闭会话 ${shortId(sessionId)}？`)) {
      return;
    }
    btn.disabled = true;
    try {
      await client.request("session.close", { sessionId });
    } catch (err) {
      window.alert("关闭会话失败：" + err.message);
    } finally {
      btn.disabled = false;
    }
  }
}

function td(text) {
  const el = document.createElement("td");
  el.textContent = text;
  return el;
}

function emptyRow(colspan, text) {
  const tr = document.createElement("tr");
  const cell = document.createElement("td");
  cell.colSpan = colspan;
  cell.textContent = text;
  cell.className = "muted";
  tr.appendChild(cell);
  return tr;
}

function shortId(id) {
  return id && id.length > 8 ? id.slice(0, 8) : id;
}

function vidPid(p) {
  const parts = [];
  if (p.vid != null) {
    parts.push("VID " + hex16(p.vid));
  }
  if (p.pid != null) {
    parts.push("PID " + hex16(p.pid));
  }
  return parts.length ? parts.join(" ") : "—";
}

function hex16(n) {
  return "0x" + n.toString(16).toUpperCase().padStart(4, "0");
}

function formatParams(params) {
  if (!params) {
    return "—";
  }
  const parity = params.parity ? params.parity.charAt(0).toUpperCase() : "?";
  return `${params.baudRate},${params.dataBits}${parity}${params.stopBits}`;
}

function formatGap(gap) {
  if (!gap) {
    return "—";
  }
  return `${gap.durationMs}ms · 丢 ${gap.discardedBytes}B${gap.resumed ? "" : "（未恢复）"}`;
}

function formatError(err) {
  if (!err) {
    return "—";
  }
  return `${err.code}: ${err.message}`;
}

function flash(btn, text) {
  const old = btn.textContent;
  btn.textContent = text;
  setTimeout(() => {
    btn.textContent = old;
  }, 1200);
}
