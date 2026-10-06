import { createApp } from "vue";
import { invoke } from "@tauri-apps/api/core";
import App from "./App.vue";

// 禁用 webview 的原生右键菜单（另存为 / 刷新 / 检查等）。
// 用捕获阶段：组件里的 @contextmenu.stop 会阻止冒泡，普通监听收不到事件，
// 捕获阶段先执行，能保证所有元素/节点上的右键都被拦掉，同时不影响应用内自定义右键菜单。
window.addEventListener(
  "contextmenu",
  (event) => {
    event.preventDefault();
  },
  true,
);

// Ctrl+F12 打开调试窗口
window.addEventListener(
  "keydown",
  (event) => {
    if (event.ctrlKey && event.key === "F12") {
      event.preventDefault();
      void invoke("open_devtools").catch(() => {});
    }
  },
  true,
);

createApp(App).mount("#app");
