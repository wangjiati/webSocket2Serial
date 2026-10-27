// 管理页/串口助手/功能测试共用的纯工具函数。

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export const now = () => Date.now();

export const shortId = (id) => (id ? String(id).slice(0, 8) : "-");

export const fmtTime = (ms) => {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}:${String(d.getSeconds()).padStart(2, "0")}.${String(ms % 1000).padStart(3, "0")}`;
};

export const bytesToHex = (bytes) => Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join(" ");

export const bytesToAscii = (bytes) => Array.from(bytes, (b) => (b >= 32 && b < 127 ? String.fromCharCode(b) : ".")).join("");

export const bytesEqual = (a, b) => a.length === b.length && a.every((v, i) => v === b[i]);

export const pushCapped = (items, item, limit) => {
  items.push(item);
  while (items.length > limit) items.shift();
};

export const textEncoder = new TextEncoder();

export const b64encodeBytes = (bytes) => {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  }
  return btoa(s);
};

export const b64decodeBytes = (b64) => {
  const s = atob(b64);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
};

export const hexToBytes = (text) => {
  const clean = String(text).replace(/[^0-9a-fA-F]/g, "");
  if (!clean.length) throw new Error("HEX 内容为空");
  if (clean.length % 2) throw new Error("HEX 长度必须为偶数");
  const out = new Uint8Array(clean.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
  return out;
};

export const pattern = (n) => Uint8Array.from({ length: n }, (_, i) => (i * 37 + 11) & 0xff);

export const download = (name, text, mime = "text/plain;charset=utf-8") => {
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

export const stateClass = (s) => (["awaiting", "active", "grace", "closed", "error"].includes(s) ? s : "");
