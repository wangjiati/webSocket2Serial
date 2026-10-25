// 管理页入口：读取 /api/config，串联 ManagementClient 与概览、串口助手两个视图。

import { getConfig, ManagementClient } from "./api.js";
import { bindOverview } from "./overview.js";
import { createAssistant } from "./assistant.js";

async function main() {
  const config = await getConfig();
  const client = new ManagementClient(config.managementPort);
  const state = { config };
  bindOverview(document.getElementById("overview"), client, state);
  createAssistant(document.getElementById("assistant"), client, state);
  client.connect();
}

main().catch((err) => {
  const status = document.getElementById("conn-status");
  if (status) {
    status.textContent = "初始化失败";
    status.className = "badge error";
  }
  // 控制台保留错误细节便于排障。
  console.error(err);
});
