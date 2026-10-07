<script setup>
import { onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import homeIcon from "../assets/icons/home.svg";
import { DEFAULT_THEME_ID, THEMES, themeById } from "../lib/themes";
import {
  acceleratorFromEvent,
  DEFAULT_SHORTCUTS,
  emptyGlobalShortcuts,
  normalizeShortcuts,
  SHORTCUT_ROWS,
} from "../lib/shortcuts";

const props = defineProps({
  settings: { type: Object, required: true },
  defaults: { type: Object, required: true },
  cacheKeepPaths: { type: Array, default: () => [] },
});
const emit = defineEmits(["close", "save", "sources-changed"]);

const draft = reactive({
  downloadDir: "",
  cacheDir: "",
  activeSourceId: "",
  activeSourceName: "暂无音源",
  quality: "320k",
  theme: DEFAULT_THEME_ID,
  globalShortcutsEnabled: false,
  shortcuts: { ...DEFAULT_SHORTCUTS },
  globalShortcuts: emptyGlobalShortcuts(),
});
const message = ref("");
const sources = ref([]);
const sourcesLoading = ref(false);
const sourceUrl = ref("");
const recordingTarget = ref("");

const themes = THEMES;
const shortcutRows = SHORTCUT_ROWS;

watch(
  () => props.settings,
  (settings) => {
    draft.downloadDir = settings.downloadDir;
    draft.cacheDir = settings.cacheDir;
    draft.activeSourceId = settings.activeSourceId || "";
    draft.activeSourceName = settings.activeSourceName || "暂无音源";
    draft.quality = ["128k", "320k", "flac", "flac24bit"].includes(settings.quality) ? settings.quality : "320k";
    draft.theme = themeById(settings.theme).id;
    draft.globalShortcutsEnabled = Boolean(settings.globalShortcutsEnabled);
    draft.shortcuts = normalizeShortcuts(settings.shortcuts, DEFAULT_SHORTCUTS);
    draft.globalShortcuts = normalizeShortcuts(settings.globalShortcuts, emptyGlobalShortcuts());
  },
  { immediate: true, deep: true },
);

async function choose(key) {
  message.value = "";
  try {
    const selected = await invoke("pick_directory");
    if (selected) draft[key] = selected;
  } catch (error) {
    message.value = String(error);
  }
}

async function open(key) {
  message.value = "";
  try {
    await invoke("open_app_directory", { path: draft[key] });
  } catch (error) {
    message.value = String(error);
  }
}

async function clearCache() {
  message.value = "";
  try {
    const removed = await invoke("clear_unused_cache", {
      cacheDir: draft.cacheDir || props.defaults.cacheDir,
      keepPaths: props.cacheKeepPaths,
    });
    const count = Number(removed) || 0;
    message.value = count ? `已清除 ${count} 个未使用文件` : "没有可清理的缓存文件";
  } catch (error) {
    message.value = `清除缓存失败：${error}`;
  }
}

function restore(key) {
  draft[key] = props.defaults[key];
}

function save() {
  emit("save", {
    downloadDir: draft.downloadDir.trim(),
    cacheDir: draft.cacheDir.trim(),
    activeSourceId: draft.activeSourceId,
    activeSourceName: draft.activeSourceName,
    quality: draft.quality,
    theme: draft.theme,
    globalShortcutsEnabled: draft.globalShortcutsEnabled,
    shortcuts: { ...draft.shortcuts },
    globalShortcuts: { ...draft.globalShortcuts },
  });
}

async function adoptImportedSource(imported) {
  draft.activeSourceId = imported.id;
  draft.activeSourceName = imported.name;
  await refreshSources();
}

async function refreshSources() {
  sourcesLoading.value = true;
  message.value = "";
  try {
    const imported = await invoke("list_music_sources", {
      cacheDir: draft.cacheDir || props.defaults.cacheDir,
    });
    sources.value = Array.isArray(imported) ? imported : [];
    if (!sources.value.some((source) => source.id === draft.activeSourceId)) {
      draft.activeSourceId = sources.value[0]?.id || "";
      draft.activeSourceName = sources.value[0]?.name || "暂无音源";
    }
    emit("sources-changed", sources.value);
  } catch (error) {
    message.value = `读取音源失败：${error}`;
  } finally {
    sourcesLoading.value = false;
  }
}

async function importSource() {
  message.value = "";
  try {
    const sourcePath = await invoke("pick_music_source");
    if (!sourcePath) return;
    const imported = await invoke("import_music_source", {
      sourcePath,
      cacheDir: draft.cacheDir || props.defaults.cacheDir,
    });
    await adoptImportedSource(imported);
  } catch (error) {
    message.value = `导入音源失败：${error}`;
  }
}

async function importSourceFromUrl() {
  const url = sourceUrl.value.trim();
  if (!url) {
    message.value = "请填写音源 URL";
    return;
  }
  message.value = "";
  sourcesLoading.value = true;
  try {
    const imported = await invoke("import_music_source_url", {
      sourceUrl: url,
      cacheDir: draft.cacheDir || props.defaults.cacheDir,
    });
    sourceUrl.value = "";
    await adoptImportedSource(imported);
  } catch (error) {
    message.value = `URL 导入音源失败：${error}`;
  } finally {
    sourcesLoading.value = false;
  }
}

async function deleteSource(source) {
  if (!source) return;
  message.value = "";
  try {
    await invoke("delete_music_source", {
      sourcePath: source.path,
      cacheDir: draft.cacheDir || props.defaults.cacheDir,
    });
    await refreshSources();
  } catch (error) {
    message.value = `删除音源失败：${error}`;
  }
}

async function refreshSourceUrl(source) {
  if (!source?.path) return;
  message.value = "";
  sourcesLoading.value = true;
  try {
    const refreshed = await invoke("refresh_music_source_url", {
      sourcePath: source.path,
      cacheDir: draft.cacheDir || props.defaults.cacheDir,
    });
    await adoptImportedSource(refreshed);
    message.value = `已刷新音源：${refreshed.name}`;
  } catch (error) {
    message.value = `刷新音源失败：${error}`;
  } finally {
    sourcesLoading.value = false;
  }
}

function targetKey(action, isGlobal) {
  return `${isGlobal ? "g" : "a"}:${action}`;
}

function setShortcut(action, isGlobal, value) {
  if (isGlobal) draft.globalShortcuts[action] = value;
  else draft.shortcuts[action] = value;
}

function onShortcutKeydown(event, action, isGlobal) {
  event.preventDefault();
  event.stopPropagation();
  if (event.key === "Escape") {
    setShortcut(action, isGlobal, "");
    recordingTarget.value = "";
    return;
  }
  const result = acceleratorFromEvent(event, isGlobal);
  if (result.modifierOnly) return;
  if (result.reason === "modifier") {
    message.value = "全局快捷键需要包含 Ctrl / Alt / Shift 等修饰键";
    return;
  }
  if (result.reason === "unsupported") {
    message.value = "暂不支持该按键，请换一个";
    return;
  }
  setShortcut(action, isGlobal, result.value);
  recordingTarget.value = "";
  message.value = "";
}

function clearShortcut(action, isGlobal) {
  setShortcut(action, isGlobal, "");
  if (recordingTarget.value === targetKey(action, isGlobal)) recordingTarget.value = "";
}

let sourceUrlTimer = 0;
watch(sourceUrl, (value) => {
  window.clearTimeout(sourceUrlTimer);
  const url = value.trim();
  if (!/^https?:\/\/\S+$/i.test(url)) return;
  sourceUrlTimer = window.setTimeout(() => {
    if (sourceUrl.value.trim() === url && !sourcesLoading.value) void importSourceFromUrl();
  }, 700);
});

function onKeydown(event) {
  if (event.key === "Escape") emit("close");
}

watch(
  () => props.settings,
  () => {
    message.value = "";
  },
  { deep: true },
);

window.addEventListener("keydown", onKeydown);
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKeydown);
  window.clearTimeout(sourceUrlTimer);
});
onMounted(refreshSources);
</script>

<template>
  <div class="settings-page" @click.self="emit('close')">
    <section class="settings-modal" role="dialog" aria-modal="true" aria-labelledby="settings-title">
      <header class="modal-header">
        <button class="back-button" type="button" @click="emit('close')">
          <img :src="homeIcon" alt="" />
          <span>返回主页</span>
        </button>
        <div class="modal-heading">
          <h2 id="settings-title">设置</h2>
        </div>
      </header>

      <div class="settings-body">

        <div class="setting-block">
          <div class="setting-heading">
            <div>
              <strong>主题</strong>
              <span>图标与强调色的配色方案</span>
            </div>
          </div>
          <div class="theme-grid">
            <button
              v-for="theme in themes"
              :key="theme.id"
              type="button"
              class="theme-swatch"
              :class="{ active: draft.theme === theme.id }"
              @click="draft.theme = theme.id"
            >
              <span
                class="theme-dot"
                :style="{ background: `linear-gradient(135deg, ${theme.accent}, ${theme.accent2})` }"
              ></span>
              <span class="theme-name">{{ theme.name }}</span>
            </button>
          </div>
        </div>
        
        <div class="setting-block">
          <div class="setting-heading">
            <div>
              <strong>下载目录</strong>
              <span>导入歌曲时复制到这里</span>
            </div>
            <div class="setting-actions">
              <button type="button" @click="choose('downloadDir')">选择</button>
              <button type="button" @click="open('downloadDir')">打开</button>
              <button type="button" @click="restore('downloadDir')">恢复默认</button>
            </div>
          </div>
          <input v-model="draft.downloadDir" class="path-input" spellcheck="false" aria-label="下载目录" />
        </div>

        <div class="setting-block">
          <div class="setting-heading">
            <div>
              <strong>缓存目录</strong>
              <span>封面、频谱缓存与临时文件</span>
            </div>
            <div class="setting-actions">
              <button type="button" @click="choose('cacheDir')">选择</button>
              <button type="button" @click="open('cacheDir')">打开</button>
              <button type="button" @click="restore('cacheDir')">恢复默认</button>
              <button type="button" @click="clearCache">清除缓存</button>
            </div>
          </div>
          <input v-model="draft.cacheDir" class="path-input" spellcheck="false" aria-label="缓存目录" />
        </div>

        <div class="setting-block">
          <div class="setting-heading">
            <div>
              <strong>优先播放音质</strong>
              <span>云库试听与下载时优先使用</span>
            </div>
            <select v-model="draft.quality" class="quality-select" aria-label="优先播放音质">
              <option value="128k">128k</option>
              <option value="320k">320k</option>
              <option value="flac">FLAC</option>
              <option value="flac24bit">FLAC 24bit</option>
            </select>
          </div>
        </div>

        <div class="setting-block">
          <div class="setting-heading">
            <div>
              <strong>快捷键</strong>
              <span>应用内快捷键窗口聚焦时生效；全局快捷键窗口隐藏也能响应（需带修饰键）</span>
            </div>
            <label class="switch">
              <input type="checkbox" v-model="draft.globalShortcutsEnabled" />
              <span>启用全局快捷键</span>
            </label>
          </div>
          <div class="shortcut-header-row">
            <span></span>
            <span>应用内</span>
            <span>全局</span>
            <span></span>
          </div>
          <div class="shortcut-list">
            <div v-for="row in shortcutRows" :key="row.action" class="shortcut-row">
              <span class="shortcut-label">{{ row.label }}</span>
              <input
                class="shortcut-input"
                :class="{ recording: recordingTarget === targetKey(row.action, false) }"
                :value="recordingTarget === targetKey(row.action, false) ? '按下按键…' : draft.shortcuts[row.action] || '未设置'"
                readonly
                @focus="recordingTarget = targetKey(row.action, false)"
                @blur="recordingTarget = ''"
                @keydown="onShortcutKeydown($event, row.action, false)"
              />
              <input
                class="shortcut-input"
                :class="{ recording: recordingTarget === targetKey(row.action, true) }"
                :value="recordingTarget === targetKey(row.action, true) ? '按下组合键…' : draft.globalShortcuts[row.action] || '未设置'"
                readonly
                :disabled="!draft.globalShortcutsEnabled"
                @focus="recordingTarget = targetKey(row.action, true)"
                @blur="recordingTarget = ''"
                @keydown="onShortcutKeydown($event, row.action, true)"
              />
              <button
                type="button"
                class="shortcut-clear"
                :disabled="!draft.shortcuts[row.action] && !draft.globalShortcuts[row.action]"
                @click="clearShortcut(row.action, false); clearShortcut(row.action, true)"
              >清除</button>
            </div>
          </div>
        </div>

        <div class="setting-block">
          <div class="setting-heading">
            <div>
              <strong>音源导入</strong>
              <span>导入后可在云库顶部切换音源</span>
            </div>
            <div class="setting-actions">
              <button type="button" :disabled="sourcesLoading" @click="importSource">导入音源</button>
              <button type="button" @click="refreshSources">刷新列表</button>
            </div>
          </div>
          <div class="source-url-row">
            <input
              v-model="sourceUrl"
              class="path-input source-url-input"
              type="url"
              placeholder="https://example.com/source.js"
              spellcheck="false"
              aria-label="音源 URL"
              @keyup.enter="importSourceFromUrl"
            />
            <button type="button" :disabled="sourcesLoading" @click="importSourceFromUrl">URL 导入</button>
          </div>
          <div class="source-list">
            <div
              v-for="source in sources"
              :key="source.id"
              class="source-item"
            >
              <div class="source-copy">
                <span>{{ source.name }}</span>
                <small>{{ source.version || "未知版本" }}{{ source.source_url ? " · URL" : "" }}</small>
              </div>
              <button
                v-if="source.source_url"
                class="source-refresh"
                type="button"
                title="按 URL 重新下载并覆盖该音源文件"
                :disabled="sourcesLoading"
                @click.stop="refreshSourceUrl(source)"
              >刷新</button>
              <button
                class="source-delete"
                type="button"
                title="删除音源"
                @click.stop="deleteSource(source)"
              >删除</button>
            </div>
            <div v-if="!sources.length && !sourcesLoading" class="source-empty">暂无可用音源</div>
          </div>
        </div>
        <div class="settings-note">
          设置会保存到本地，重启后继续生效；目录不存在时会自动创建。
        </div>
        <div class="settings-note">
            https://github.com/yunzhouapps/tingci-music-desktop
        </div>
        <div class="settings-note">R.M.W.S.</div>
        <div v-if="message" class="settings-message">{{ message }}</div>
      </div>

      <footer class="modal-footer">
        <span class="settings-version">v1.0.1</span>
        <button class="secondary-button" type="button" @click="emit('close')">取消</button>
        <button class="primary-button" type="button" @click="save">保存设置</button>
      </footer>
    </section>
  </div>
</template>

<style scoped>
.settings-page {
  position: fixed;
  inset: 0;
  z-index: 20;
  display: flex;
  background: var(--canvas);
}

.settings-modal {
  width: 100%;
  max-width: none;
  height: 100%;
  max-height: none;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 0;
  border-radius: 0;
  background: var(--surface);
  box-shadow: none;
}

.modal-header,
.modal-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 15px 17px;
}

.modal-header {
  border-bottom: 1px solid var(--line);
}

.modal-heading {
  flex: 1;
  text-align: right;
}

.back-button {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 6px 8px;
  border: 1px solid var(--line);
  border-radius: 6px;
  background: var(--surface-2);
  color: var(--muted);
  cursor: pointer;
  font-size: 13px;
}

.back-button:hover {
  border-color: var(--accent);
  color: var(--accent);
}

.back-button img {
  width: 15px;
  height: 15px;
}

.modal-kicker {
  color: var(--accent);
  font-size: 12px;
  font-weight: 650;
  letter-spacing: 0.1em;
  text-transform: uppercase;
}

h2 {
  margin: 2px 0 0;
  color: var(--text);
  font-size: 20px;
}

.settings-body {
  min-height: 0;
  overflow: auto;
  padding: 15px 17px;
}

.setting-block + .setting-block {
  margin-top: 16px;
  padding-top: 16px;
  border-top: 1px solid var(--line);
}

.setting-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
}

.setting-heading strong,
.setting-heading span {
  display: block;
}

.setting-heading strong {
  color: var(--text);
  font-size: 15px;
}

.setting-heading span {
  margin-top: 3px;
  color: var(--muted);
  font-size: 12px;
}

.setting-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 5px;
}

.setting-actions button,
.secondary-button,
.primary-button {
  border: 1px solid var(--line);
  border-radius: 6px;
  padding: 6px 8px;
  background: var(--surface-2);
  color: var(--muted);
  cursor: pointer;
  font-size: 12px;
}

.setting-actions button:hover,
.secondary-button:hover {
  border-color: var(--accent);
  color: var(--accent);
}

.path-input {
  width: 100%;
  margin-top: 10px;
  padding: 9px 10px;
  border: 1px solid var(--line);
  border-radius: 6px;
  outline: none;
  background: var(--canvas);
  color: var(--text);
  font-size: 13px;
}

.path-input:focus {
  border-color: var(--accent);
}

.quality-select {
  min-width: 92px;
  padding: 7px 8px;
  border: 1px solid var(--line);
  border-radius: 6px;
  outline: none;
  background: var(--surface-2);
  color: var(--text);
  font-size: 13px;
}

.quality-select:focus {
  border-color: var(--accent);
}

.theme-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 7px;
  margin-top: 10px;
}

.theme-swatch {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  border: 1px solid var(--line);
  border-radius: 99px;
  background: var(--surface-2);
  color: var(--muted);
  cursor: pointer;
  font-size: 12px;
}

.theme-swatch:hover {
  border-color: var(--accent);
}

.theme-swatch.active {
  border-color: var(--accent);
  color: var(--accent);
  background: color-mix(in srgb, var(--accent) 10%, var(--surface-2));
}

.theme-dot {
  width: 13px;
  height: 13px;
  flex: 0 0 13px;
  border-radius: 50%;
  box-shadow: inset 0 0 0 1px rgba(0, 0, 0, 0.08);
}

.theme-name {
  white-space: nowrap;
}

.switch {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  color: var(--muted);
  cursor: pointer;
  font-size: 12px;
}

.switch input {
  accent-color: var(--accent);
}

.shortcut-list {
  display: grid;
  gap: 6px;
  margin-top: 10px;
}

.shortcut-list.disabled {
  opacity: 0.55;
}

.shortcut-row {
  display: grid;
  grid-template-columns: 1fr 108px 108px auto;
  align-items: center;
  gap: 7px;
}

.shortcut-header-row {
  display: grid;
  grid-template-columns: 1fr 108px 108px auto;
  gap: 7px;
  margin-top: 10px;
  color: var(--faint);
  font-size: 12px;
  text-align: center;
}

.shortcut-header-row > span:first-child {
  text-align: left;
}

.shortcut-label {
  color: var(--text);
  font-size: 13px;
}

.shortcut-input {
  width: 100%;
  padding: 6px 8px;
  border: 1px solid var(--line);
  border-radius: 6px;
  outline: none;
  background: var(--canvas);
  color: var(--text);
  cursor: pointer;
  font-size: 12px;
  text-align: center;
}

.shortcut-input.recording {
  border-color: var(--accent);
  color: var(--accent);
}

.shortcut-clear {
  padding: 5px 7px;
  border: 1px solid var(--line);
  border-radius: 5px;
  background: var(--surface-2);
  color: var(--muted);
  cursor: pointer;
  font-size: 12px;
}

.shortcut-clear:disabled {
  cursor: default;
  opacity: 0.5;
}

.source-list {
  display: grid;
  gap: 6px;
  margin-top: 10px;
}

.source-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  padding: 8px 9px;
  border: 1px solid var(--line);
  border-radius: 6px;
  background: var(--canvas);
  color: var(--text);
  text-align: left;
}

.source-item:hover {
  border-color: var(--accent);
}

.source-copy {
  min-width: 0;
  flex: 1;
}

.source-copy span {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
  font-size: 13px;
}

.source-copy small,
.source-empty {
  color: var(--muted);
  font-size: 12px;
}

.source-delete {
  flex: 0 0 auto;
  padding: 4px 7px;
  border: 1px solid var(--line);
  border-radius: 5px;
  background: var(--surface-2);
  color: var(--muted);
  cursor: pointer;
  font-size: 12px;
}

.source-refresh {
  flex: 0 0 auto;
  padding: 4px 7px;
  border: 1px solid var(--line);
  border-radius: 5px;
  background: var(--surface-2);
  color: var(--muted);
  cursor: pointer;
  font-size: 12px;
}

.source-refresh:hover {
  border-color: var(--accent);
  color: var(--accent);
}

.source-url-row {
  display: flex;
  align-items: stretch;
  gap: 6px;
  margin-top: 10px;
}

.source-url-input {
  min-width: 0;
  flex: 1;
  margin-top: 0;
}

.source-url-row > button {
  flex: 0 0 auto;
  padding: 6px 9px;
  border: 1px solid var(--line);
  border-radius: 6px;
  background: var(--surface-2);
  color: var(--muted);
  cursor: pointer;
  font-size: 12px;
}

.source-url-row > button:hover {
  border-color: var(--accent);
  color: var(--accent);
}

.source-delete:hover {
  border-color: #d15a5a;
  color: #d15a5a;
}

.source-empty {
  padding: 8px 0;
}

.settings-note,
.settings-message {
  margin-top: 16px;
  color: var(--faint);
  font-size: 13px;
  line-height: 1.6;
}

.settings-message {
  color: #d15a5a;
}

.modal-footer {
  justify-content: flex-end;
  border-top: 1px solid var(--line);
}

.settings-version {
  margin-right: auto;
  color: var(--faint);
  font-size: 12px;
}

.primary-button {
  border-color: var(--accent);
  background: var(--accent);
  color: #fff;
}

.primary-button:hover {
  background: var(--accent-2);
}
</style>
