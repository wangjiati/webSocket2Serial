// 功能测试页入口：复用 createDemo。

import { getConfig, ManagementClient } from "./api.js";
import { createDemo } from "./demo.js";

async function main() {
  const config = await getConfig();
  const client = new ManagementClient(config.managementPort);
  createDemo(document.getElementById("demo"), client);
  client.connect();
}

main().catch((err) => {
  const status = document.getElementById("demo-pill-mgmt");
  if (status) {
    status.textContent = "初始化失败";
    status.className = "demo-pill err";
  }
  console.error(err);
});
