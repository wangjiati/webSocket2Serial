// 管理页「功能演示」模块；与 demo/websocket2serial-demo.html 保持功能同步。
export function createDemo(root, client) {
  const $ = (selector) => root.querySelector(selector);
  const $$ = (selector) => Array.from(root.querySelectorAll(selector));

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
  }

  tabs.forEach((btn, index) => {
    btn.addEventListener("click", () => activate(btn.dataset.demoTab));
    btn.addEventListener("keydown", (event) => {
      if (event.key !== "ArrowRight" && event.key !== "ArrowLeft") {
        return;
      }
      event.preventDefault();
      const delta = event.key === "ArrowRight" ? 1 : -1;
      const next = (index + delta + tabs.length) % tabs.length;
      tabs[next].focus();
      activate(tabs[next].dataset.demoTab);
    });
  });

  activate(current);

  return {
    onShow() {
      activate(current);
    },
  };
}
