import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const SOURCE_ROOT = new URL("../../src/assets/js/", import.meta.url);

class FakeWebSocket {
  static OPEN = 1;
  static instances = [];

  constructor(url) {
    this.url = url;
    this.readyState = FakeWebSocket.OPEN;
    this.sent = [];
    FakeWebSocket.instances.push(this);
  }

  send(data) {
    this.sent.push(data);
  }

  close(code = 1000, reason = "") {
    this.readyState = 3;
    this.closeCode = code;
    this.closeReason = reason;
  }
}

globalThis.WebSocket = FakeWebSocket;

async function loadManagementClient() {
  const source = await readFile(new URL("api.js", SOURCE_ROOT), "utf8");
  const url = `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`;
  return (await import(url)).ManagementClient;
}

async function loadSerialSession() {
  const dir = await mkdtemp(join(tmpdir(), "ws2s-assistant-"));
  try {
    const format = await readFile(new URL("format.js", SOURCE_ROOT), "utf8");
    const serial = (await readFile(new URL("serial.js", SOURCE_ROOT), "utf8")).replace(
      '"./format.js"',
      '"./format.mjs"',
    );
    await writeFile(join(dir, "format.mjs"), format);
    await writeFile(join(dir, "serial.mjs"), serial);
    const { SerialSession } = await import(pathToFileURL(join(dir, "serial.mjs")).href);
    return { SerialSession, cleanup: () => rm(dir, { recursive: true, force: true }) };
  } catch (err) {
    await rm(dir, { recursive: true, force: true });
    throw err;
  }
}

async function testManagementTrace() {
  const ManagementClient = await loadManagementClient();
  const client = new ManagementClient(8765);
  const sent = [];
  const traces = [];
  client.connected = true;
  client.ws = { readyState: FakeWebSocket.OPEN, send: (raw) => sent.push(raw) };
  client.onTrace(() => {
    throw new Error("trace observer failure must be isolated");
  });
  client.onTrace((entry) => traces.push(entry));

  const portsRequest = client.request("ports.list", {});
  const sessionsRequest = client.request("session.list", {});
  const requests = sent.map((raw) => JSON.parse(raw));
  assert.deepEqual(requests.map((msg) => msg.action), ["ports.list", "session.list"]);
  assert.deepEqual(
    traces.map((entry) => [entry.channel, entry.dir, entry.kind, entry.text]),
    [
      ["mgmt", "out", "ports.list", sent[0]],
      ["mgmt", "out", "session.list", sent[1]],
    ],
  );

  const sessionsRaw = JSON.stringify({ id: requests[1].id, ok: true, data: { sessions: [] } });
  const portsRaw = JSON.stringify({ id: requests[0].id, ok: true, data: { ports: [] } });
  client.handleMessage(sessionsRaw);
  await sessionsRequest;
  client.handleMessage(portsRaw);
  await portsRequest;
  assert.deepEqual(
    traces.slice(2).map((entry) => [entry.dir, entry.kind, entry.text]),
    [
      ["in", "session.list ok", sessionsRaw],
      ["in", "ports.list ok", portsRaw],
    ],
    "responses must keep the action associated with their request id",
  );

  const invalidRaw = "<not-json>";
  assert.doesNotThrow(() => client.handleMessage(invalidRaw));
  assert.deepEqual(traces.at(-1), { channel: "mgmt", dir: "in", kind: "invalid", text: invalidRaw });

  const eventRaw = JSON.stringify({ event: "test.event", data: { value: 1 } });
  client.handleMessage(eventRaw);
  assert.deepEqual(traces.at(-1), {
    channel: "mgmt",
    dir: "in",
    kind: "event:test.event",
    text: eventRaw,
  });

  const lifecycle = new ManagementClient(8765);
  const lifecycleTraces = [];
  lifecycle.onTrace((entry) => lifecycleTraces.push(entry));
  lifecycle.open();
  const ws = FakeWebSocket.instances.at(-1);
  ws.onopen();
  assert.equal(lifecycle.connected, true);
  lifecycle.closedByUser = true;
  ws.onclose({ code: 1000, reason: "done" });
  assert.deepEqual(lifecycleTraces.map((entry) => entry.kind), ["connect", "disconnect"]);
  assert.match(lifecycleTraces[1].text, /code=1000/);
  assert.match(lifecycleTraces[1].text, /done/);
}

async function testSerialTrace() {
  const { SerialSession, cleanup } = await loadSerialSession();
  try {
    const traces = [];
    const session = new SerialSession(
      "A",
      { sessionId: "s1", dataUrl: "ws://127.0.0.1:9000/data/token", state: "awaiting" },
      {},
      {},
      { onTrace: (entry) => traces.push(entry) },
    );
    const connecting = session.connectData();
    const ws = FakeWebSocket.instances.at(-1);
    ws.onopen();
    await connecting;
    session.sendBytes(Uint8Array.of(1, 2, 3));
    ws.onclose({ code: 4001, reason: "device lost" });

    assert.deepEqual(traces.map((entry) => entry.kind), [
      "data.connecting",
      "data.open",
      "data.send",
      "data.close",
    ]);
    assert.ok(traces.every((entry) => entry.channel === "data"));
    assert.equal(traces[0].text, "token");
    assert.deepEqual(traces[2], { channel: "data", dir: "out", kind: "data.send", text: "3 B" });
    assert.match(traces[3].text, /code=4001/);
    assert.match(traces[3].text, /device lost/);
    assert.equal(session.state, "closed");

    const throwing = new SerialSession(
      "B",
      { sessionId: "s2", dataUrl: "ws://127.0.0.1:9001/data/other" },
      {},
      {},
      {
        onTrace: () => {
          throw new Error("observer failure");
        },
      },
    );
    let sent = 0;
    throwing.ws = { readyState: FakeWebSocket.OPEN, send: () => { sent += 1; } };
    assert.doesNotThrow(() => throwing.sendBytes(Uint8Array.of(9)));
    assert.equal(sent, 1);
    assert.equal(throwing.frames.length, 1);
  } finally {
    await cleanup();
  }
}

async function testRefreshPorts() {
  const ManagementClient = await loadManagementClient();
  const client = new ManagementClient(8765);
  let syncCount = 0;
  client.onSync(() => { syncCount += 1; });
  client.request = async (action, params) => {
    assert.equal(action, "ports.list");
    assert.deepEqual(params, {});
    return { ports: [{ path: "COM7" }] };
  };

  const refreshed = await client.refreshPorts();
  assert.equal(refreshed, true);
  assert.deepEqual(client.ports, [{ path: "COM7" }]);
  assert.equal(syncCount, 1);

  const previousPorts = client.ports;
  client.request = async () => {
    throw new Error("connection lost");
  };
  const failed = await client.refreshPorts();
  assert.equal(failed, false);
  assert.equal(client.ports, previousPorts);
  assert.deepEqual(client.ports, [{ path: "COM7" }]);
  assert.equal(syncCount, 1);
  console.log("refresh behavior: ok");
}

const selected = process.argv[2] || "all";
if (!["trace", "refresh", "panel", "all"].includes(selected)) {
  throw new Error(`unknown behavior test case: ${selected}`);
}
if (selected === "trace" || selected === "all") {
  await testManagementTrace();
  await testSerialTrace();
  console.log("trace behavior: ok");
}
if (selected === "refresh" || selected === "all") {
  await testRefreshPorts();
}
