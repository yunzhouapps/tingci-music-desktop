<script setup>
import { getCurrentWindow } from "@tauri-apps/api/window";

const emit = defineEmits(["collapse", "settings", "close"]);
const window = getCurrentWindow();

async function minimize() {
  await window.minimize();
}

async function toggleMaximize() {
  await window.toggleMaximize();
}

function close() {
  // 交给 App 处理：关闭右侧功能窗口后再隐藏主窗口
  emit("close");
}
</script>

<template>
  <header class="titlebar" data-tauri-drag-region>
    <div class="titlebar-brand" data-tauri-drag-region>
      <img class="brand-logo" src="../assets/icon.svg" alt="听词 Logo" />
      <span class="brand-name">听词</span>
    </div>
    <div class="titlebar-actions">
      <button
        class="titlebar-button"
        type="button"
        title="折叠播放列表"
        aria-label="折叠播放列表"
        @pointerdown.stop
        @click.stop="emit('collapse')"
      >
        <img class="titlebar-icon" src="../assets/icons/qiehuan.svg" alt="" />
      </button>
      <button
        class="titlebar-button"
        type="button"
        title="设置"
        aria-label="设置"
        @pointerdown.stop
        @click.stop="emit('settings')"
      >
        <img class="titlebar-icon" src="../assets/icons/shezhi.svg" alt="" />
      </button>
      <button
        class="titlebar-button"
        type="button"
        title="最小化"
        aria-label="最小化"
        @pointerdown.stop
        @click.stop="minimize"
      >
        <img class="titlebar-icon" src="../assets/icons/quxiao1.svg" alt="" />
      </button>
      <button
        class="titlebar-button"
        type="button"
        title="最大化 / 还原"
        aria-label="最大化或还原"
        @pointerdown.stop
        @click.stop="toggleMaximize"
      >
        <img class="titlebar-icon" src="../assets/icons/tianjia1.svg" alt="" />
      </button>
      <button
        class="titlebar-button"
        type="button"
        title="关闭窗口"
        aria-label="关闭窗口"
        @pointerdown.stop
        @click.stop="close"
      >
        <img class="titlebar-icon" src="../assets/icons/cuowu.svg" alt="" />
      </button>
    </div>
  </header>
</template>

<style scoped>
.titlebar {
  height: 46px;
  min-height: 46px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 8px 0 12px;
  border-bottom: 1px solid var(--line);
  background: var(--surface);
  user-select: none;
  position: relative;
  z-index: 5;
}

.titlebar-brand {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.brand-logo {
  width: 21px;
  height: 21px;
  flex: 0 0 auto;
}

.brand-name {
  font-size: 16px;
  font-weight: 650;
  letter-spacing: 0.04em;
}

.titlebar-actions {
  display: flex;
  align-items: center;
  gap: 2px;
}

.titlebar-button {
  width: 31px;
  height: 29px;
  padding: 0;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--muted);
  font-size: 19px;
  line-height: 1;
  cursor: pointer;
  transition: background 120ms ease, color 120ms ease;
}

.titlebar-button:hover {
  color: var(--text);
  background: var(--hover);
}

.titlebar-close:hover {
  color: #fff;
  background: #d94b4b;
}

.titlebar-icon {
  width: 16px;
  height: 16px;
  object-fit: contain;
  opacity: 0.82;
}

.titlebar-close .titlebar-icon {
  filter: grayscale(1) brightness(0.75);
}
</style>
