// 管理页入口：读取 /api/config，创建管理页与功能演示两条管理 WS，
// 并串联主 tab、概览、串口助手与演示四个视图。

import { getConfig, ManagementClient } from "./api.js";
import { bindOverview } from "./overview.js";
import { createAssistant } from "./assistant.js";
import { createDemo } from "./demo.js";

/**
 * 主 tab：概览 / 串口助手 / 功能演示。
 * 只切换显示状态，不销毁视图；进入演示时通知它重绘当前子 tab。
 */
function setupMainTabs(demoView) {
  const nav = document.getElementById("main-tabs");
  const buttons = Array.from(nav.querySelectorAll("[data-main-tab]"));
  const panels = buttons.map((btn) => document.getElementById(btn.dataset.mainTab));
  let current = "overview";

  const activate = (name) => {
    current = name;
    for (const btn of buttons) {
      const active = btn.dataset.mainTab === name;
      btn.classList.toggle("active", active);
      btn.setAttribute("aria-selected", String(active));
      btn.tabIndex = active ? 0 : -1;
    }
    for (const panel of panels) {
      if (panel) {
        panel.classList.toggle("active", panel.id === name);
      }
    }
    if (name === "demo") {
      demoView.onShow();
    }
  };

  buttons.forEach((btn, index) => {
    btn.addEventListener("click", () => activate(btn.dataset.mainTab));
    btn.addEventListener("keydown", (event) => {
      if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") {
        return;
      }
      event.preventDefault();
      const delta = event.key === "ArrowRight" ? 1 : -1;
      const next = (index + delta + buttons.length) % buttons.length;
      buttons[next].focus();
      activate(buttons[next].dataset.mainTab);
    });
  });

  activate(current);
}

async function main() {
  const config = await getConfig();
  const adminClient = new ManagementClient(config.managementPort);
  const demoClient = new ManagementClient(config.managementPort);
  const state = { config };

  bindOverview(document.getElementById("overview"), adminClient, state);
  createAssistant(document.getElementById("assistant"), adminClient);
  const demoView = createDemo(document.getElementById("demo"), demoClient);
  setupMainTabs(demoView);

  adminClient.connect();
  demoClient.connect();
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
