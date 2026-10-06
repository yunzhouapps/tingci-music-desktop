<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import importIcon from "../assets/icons/tianjiawenjian.svg";
import addIcon from "../assets/icons/tianjia1.svg";
import deleteIcon from "../assets/icons/shanchu.svg";
import audioIcon from "../assets/icons/yinpin.svg";
import downIcon from "../assets/icons/xiangxia.svg";
import rightIcon from "../assets/icons/xiangyou.svg";
import cloudIcon from "../assets/icons/yunsou.svg";

const props = defineProps({
  state: { type: Object, required: true },
  groups: { type: Array, required: true },
  songs: { type: Array, required: true },
  currentSongId: { type: String, default: "" },
  selectedGroupId: { type: String, default: "default" },
});

const emit = defineEmits([
  "select-group",
  "play",
  "create-group",
  "rename-group",
  "delete-group",
  "clear-group",
  "toggle-group",
  "move-song",
  "copy-song",
  "remove-song",
  "reveal",
  "download-song",
  "import",
  "cloud",
]);

const playlistRoot = ref(null);
const editingGroupId = ref("");
const editingName = ref("");
const editInput = ref(null);
const creatingGroup = ref(false);
const creatingGroupName = ref("");
const createInput = ref(null);
const draggedSongId = ref("");
const addMenu = reactive({ songId: "", x: 0, y: 0 });
let longPressTimer = 0;
let suppressClickUntil = 0;
const menu = reactive({
  visible: false,
  x: 0,
  y: 0,
  type: "",
  id: "",
  songId: "",
  moveOpen: false,
});

const groupMap = computed(() => new Map(props.groups.map((group) => [group.id, group])));
const songMap = computed(() => new Map(props.songs.map((song) => [song.id, song])));
const copyTargets = computed(() => props.groups.filter((group) => !group.isDefault));

const MENU_WIDTH = 150;

function closeAddMenu() {
  addMenu.songId = "";
}

function toggleAddMenu(event, song) {
  if (addMenu.songId === song.id) {
    closeAddMenu();
    return;
  }
  const rect = event.currentTarget.getBoundingClientRect();
  const rows = Math.max(1, copyTargets.value.length);
  const height = Math.min(260, rows * 30 + 12);
  const maxLeft = Math.max(8, window.innerWidth - MENU_WIDTH - 8);
  addMenu.songId = song.id;
  addMenu.x = Math.min(Math.max(8, rect.right - MENU_WIDTH), maxLeft);
  // 下方放不下就向上弹，避免被滚动容器裁掉
  addMenu.y = rect.bottom + 6 + height > window.innerHeight
    ? Math.max(8, rect.top - 6 - height)
    : rect.bottom + 6;
}

function copyToGroup(groupId) {
  if (!addMenu.songId) return;
  emit("copy-song", { songId: addMenu.songId, groupId });
  closeAddMenu();
}

function songsForGroup(groupId) {
  return props.songs
    .filter((song) => song.groupId === groupId)
    .sort((left, right) => left.order - right.order);
}

function closeMenu() {
  menu.visible = false;
  menu.moveOpen = false;
  closeAddMenu();
}

function showMenu(event, type, id = "", songId = "") {
  menu.x = Math.min(event.clientX, window.innerWidth - 190);
  menu.y = Math.min(event.clientY, window.innerHeight - 220);
  menu.type = type;
  menu.id = id;
  menu.songId = songId;
  menu.moveOpen = false;
  menu.visible = true;
}

function onRootContextMenu(event) {
  if (event.target === playlistRoot.value || event.target.classList.contains("playlist-scroll")) {
    showMenu(event, "playlist");
  }
}

function onGroupContextMenu(event, group) {
  if (group.isDefault) {
    showMenu(event, "default-group", group.id);
    return;
  }
  showMenu(event, "group", group.id);
}

function onSongContextMenu(event, song) {
  showMenu(event, "song", song.groupId, song.id);
}

function requestCreateGroup() {
  beginCreateGroup();
}

function requestDeleteGroup() {
  if (!menu.id || menu.id === "default") {
    closeMenu();
    return;
  }
  emit("delete-group", menu.id);
  closeMenu();
}

function requestClearGroup() {
  if (menu.id !== "default") {
    closeMenu();
    return;
  }
  emit("clear-group", menu.id);
  closeMenu();
}

function requestPlay() {
  if (menu.songId) emit("play", menu.songId);
  closeMenu();
}

function requestRemove() {
  if (menu.songId) emit("remove-song", menu.songId);
  closeMenu();
}

function requestReveal() {
  if (menu.songId) emit("reveal", menu.songId);
  closeMenu();
}

function requestDownload() {
  if (menu.songId) emit("download-song", menu.songId);
  closeMenu();
}

function moveTo(groupId) {
  if (menu.songId) emit("move-song", { songId: menu.songId, groupId });
  closeMenu();
}

function beginEdit(group) {
  if (group.isDefault) return;
  cancelCreateGroup();
  editingGroupId.value = group.id;
  editingName.value = group.name;
  menu.visible = false;
  void nextTick(() => {
    editInput.value?.focus();
    editInput.value?.select();
  });
}

function cancelEdit() {
  editingGroupId.value = "";
  editingName.value = "";
}

function saveEdit() {
  if (!editingGroupId.value) return;
  const name = editingName.value.trim();
  if (name) emit("rename-group", { groupId: editingGroupId.value, name });
  cancelEdit();
}

function beginCreateGroup() {
  closeMenu();
  cancelEdit();
  creatingGroupName.value = "";
  creatingGroup.value = true;
  void nextTick(() => {
    createInput.value?.focus();
    createInput.value?.select();
  });
}

function cancelCreateGroup() {
  creatingGroup.value = false;
  creatingGroupName.value = "";
}

function saveCreateGroup() {
  const name = creatingGroupName.value.trim();
  if (!name) {
    cancelCreateGroup();
    return;
  }
  emit("create-group", name);
  cancelCreateGroup();
}

function clearLongPress() {
  window.clearTimeout(longPressTimer);
  longPressTimer = 0;
}

function beginHeaderPointer(event, group) {
  if (event.button !== 0 || group.isDefault || editingGroupId.value || creatingGroup.value) return;
  clearLongPress();
  longPressTimer = window.setTimeout(() => {
    suppressClickUntil = Date.now() + 500;
    beginEdit(group);
  }, 550);
}

function endHeaderPointer() {
  clearLongPress();
}

function handleHeaderClick(group) {
  if (Date.now() < suppressClickUntil || editingGroupId.value || creatingGroup.value) return;
  selectGroup(group);
  toggleGroup(group);
}

function handleHeaderDoubleClick(group) {
  clearLongPress();
  if (group.isDefault) return;
  beginEdit(group);
}

function beginDrag(event, song) {
  draggedSongId.value = song.id;
  event.dataTransfer.effectAllowed = "move";
  const payload = JSON.stringify({ songId: song.id });
  event.dataTransfer.setData("application/x-tingci-song", payload);
  event.dataTransfer.setData("text/plain", payload);
}

function endDrag() {
  draggedSongId.value = "";
}

function dropSong(event, groupId, targetSongId = "") {
  event.preventDefault();
  event.stopPropagation();
  let payload;
  try {
    const rawPayload =
      event.dataTransfer.getData("application/x-tingci-song") ||
      event.dataTransfer.getData("text/plain");
    payload = JSON.parse(rawPayload);
  } catch {
    payload = { songId: draggedSongId.value };
  }
  if (!payload?.songId) return;
  let afterTarget = false;
  if (targetSongId && event.currentTarget instanceof HTMLElement) {
    const bounds = event.currentTarget.getBoundingClientRect();
    afterTarget = event.clientY > bounds.top + bounds.height / 2;
  }
  const sourceSong = props.songs.find((song) => song.id === payload.songId);
  const eventName = sourceSong?.groupId !== groupId ? "copy-song" : "move-song";
  emit(eventName, { songId: payload.songId, groupId, targetSongId, afterTarget });
}

function dragOver(event) {
  event.preventDefault();
  event.dataTransfer.dropEffect = "move";
}

function selectGroup(group) {
  emit("select-group", group.id);
}

function toggleGroup(group) {
  emit("toggle-group", group.id);
}

watch(
  () => editingGroupId.value,
  (value) => {
    if (value) void nextTick(() => editInput.value?.focus());
  },
);

onMounted(() => {
  window.addEventListener("click", closeMenu);
  // 滚动时收起"复制到其他分组"菜单（它是 fixed 定位的，不跟随滚动）
  window.addEventListener("scroll", closeAddMenu, true);
});
onBeforeUnmount(() => {
  window.removeEventListener("click", closeMenu);
  window.removeEventListener("scroll", closeAddMenu, true);
  clearLongPress();
});
</script>

<template>
  <section ref="playlistRoot" class="playlist-manager" @contextmenu="onRootContextMenu">
    <div class="playlist-toolbar">
      <div class="toolbar-left">
        <button class="toolbar-button" type="button" @click="beginCreateGroup">
          <img class="toolbar-icon" :src="addIcon" alt="" />
          新建分组
        </button>
        <button class="toolbar-button" type="button" @click="emit('import')">
          <img class="toolbar-icon" :src="importIcon" alt="" />
          导入歌曲
        </button>
      </div>
      <button class="toolbar-button cloud-button" type="button" title="打开云库" @click="emit('cloud')">
        <img class="toolbar-icon" :src="cloudIcon" alt="" />
        云库
      </button>
    </div>

    <div class="playlist-scroll">
      <article
        v-for="group in groups"
        :key="group.id"
        class="group"
        :class="{ 'group-selected': selectedGroupId === group.id }"
        @dragover="dragOver"
        @drop="dropSong($event, group.id)"
      >
        <header
          class="group-header"
          :class="{ 'group-default': group.isDefault }"
          @pointerdown="beginHeaderPointer($event, group)"
          @pointerup="endHeaderPointer"
          @pointercancel="endHeaderPointer"
          @click.stop="handleHeaderClick(group)"
          @dblclick.stop="handleHeaderDoubleClick(group)"
          @contextmenu.stop="onGroupContextMenu($event, group)"
          @dragover="dragOver"
          @drop="dropSong($event, group.id)"
        >
          <img
            class="group-chevron"
            :src="group.collapsed ? rightIcon : downIcon"
            :alt="group.collapsed ? '展开分组' : '折叠分组'"
            :title="group.collapsed ? '展开分组' : '折叠分组'"
            draggable="false"
          />
          <input
            v-if="editingGroupId === group.id"
            ref="editInput"
            v-model="editingName"
            class="group-rename"
            maxlength="40"
            @pointerdown.stop
            @click.stop
            @dblclick.stop
            @contextmenu.stop
            @keydown.enter.prevent="saveEdit"
            @keydown.esc.stop.prevent="cancelEdit"
            @blur="saveEdit"
          />
          <template v-else>
            <span class="group-name">{{ group.name }}</span>
            <span class="group-count">{{ songsForGroup(group.id).length }}</span>
          </template>
        </header>

        <div
          v-if="!group.collapsed"
          class="group-songs"
          @dragover="dragOver"
          @drop="dropSong($event, group.id)"
          @contextmenu.stop
        >
          <div
            v-for="song in songsForGroup(group.id)"
            :key="song.id"
            class="song-item"
            :class="{
              'song-current': currentSongId === song.id,
              'song-default': group.isDefault,
            }"
            draggable="true"
            @dragstart="beginDrag($event, song)"
            @dragend="endDrag"
            @dragover="dragOver"
            @drop="dropSong($event, group.id, song.id)"
            @dblclick.stop="emit('play', song.id)"
            @contextmenu.stop="onSongContextMenu($event, song)"
          >
            <img class="song-playing" :class="{ active: currentSongId === song.id }" :src="audioIcon" alt="" draggable="false" />
            <span class="song-default-line">{{ song.title }} - {{ song.artist }}</span>
            <div
              v-if="group.isDefault"
              class="song-actions"
              @click.stop
              @dblclick.stop
              @contextmenu.stop
              @pointerdown.stop
            >
              <button class="song-action" type="button" title="复制添加到其他分组" @click.stop="toggleAddMenu($event, song)">
                <img :src="addIcon" alt="" draggable="false" />
              </button>
              <button class="song-action" type="button" title="从列表移除" @click.stop="emit('remove-song', song.id)">
                <img :src="deleteIcon" alt="" draggable="false" />
              </button>
            </div>
          </div>
        </div>
      </article>

      <article v-if="creatingGroup" class="group group-new">
        <header class="group-header">
          <input
            ref="createInput"
            v-model="creatingGroupName"
            class="group-rename creating-rename"
            maxlength="40"
            placeholder="输入分组名"
            @pointerdown.stop
            @click.stop
            @dblclick.stop
            @contextmenu.stop
            @keydown.enter.prevent="saveCreateGroup"
            @keydown.esc.stop.prevent="cancelCreateGroup"
            @blur="saveCreateGroup"
          />
        </header>
      </article>

      <!-- <div class="playlist-hint">空白处右键可新建分组；双击或长按分组名可重命名。</div> -->
    </div>

    <div
      v-if="menu.visible"
      class="context-menu"
      :style="{ left: `${menu.x}px`, top: `${menu.y}px` }"
      @click.stop
      @contextmenu.prevent
    >
      <template v-if="menu.type === 'playlist'">
        <button type="button" @click="requestCreateGroup">新建分组</button>
        <button type="button" @click="emit('import'); closeMenu()">导入歌曲</button>
      </template>
      <template v-else-if="menu.type === 'group'">
        <button v-if="menu.id !== 'default'" type="button" @click="requestDeleteGroup">删除分组</button>
      </template>
      <template v-else-if="menu.type === 'default-group'">
        <button type="button" @click="requestClearGroup">清空列表</button>
      </template>
      <template v-else-if="menu.type === 'song'">
        <button type="button" @click="requestPlay">播放</button>
        <button type="button" @click="menu.moveOpen = !menu.moveOpen">移动到分组 ›</button>
        <div v-if="menu.moveOpen" class="move-menu">
          <button
            v-for="group in groups"
            :key="group.id"
            type="button"
            @click="moveTo(group.id)"
          >
            {{ group.name }}
          </button>
        </div>
        <button v-if="menu.id === 'default'" type="button" @click="requestDownload">下载到下载目录</button>
        <button type="button" @click="requestRemove">从列表移除</button>
        <button type="button" @click="requestReveal">在文件夹中显示</button>
      </template>
    </div>
  </section>

  <!-- 复制到其他分组：用 fixed + teleport，避免被滚动容器裁掉、保证显示在最上层 -->
  <Teleport to="body">
    <div
      v-if="addMenu.songId"
      class="song-add-menu"
      :style="{ left: `${addMenu.x}px`, top: `${addMenu.y}px`, width: `${MENU_WIDTH}px` }"
      @click.stop
      @dblclick.stop
      @contextmenu.stop.prevent
    >
      <button
        v-for="target in copyTargets"
        :key="target.id"
        type="button"
        @click.stop="copyToGroup(target.id)"
      >{{ target.name }}</button>
      <div v-if="!copyTargets.length" class="song-add-empty">暂无其他分组</div>
    </div>
  </Teleport>
</template>

<style scoped>
.playlist-manager {
  position: relative;
  min-height: 0;
  display: flex;
  flex-direction: column;
  background: var(--canvas);
}

.playlist-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 11px 14px 8px;
}

.toolbar-button,
.new-group-inline {
  border: 1px solid var(--line);
  border-radius: 5px;
  background: var(--surface);
  color: var(--muted);
  cursor: pointer;
  font-size: 13px;
}

.toolbar-left {
  display: flex;
  align-items: center;
  gap: 6px;
}

.cloud-button {
  flex: 0 0 auto;
}

.toolbar-button {
  padding: 6px 8px;
}

.toolbar-button,
.new-group-inline {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
}

.toolbar-icon {
  width: 14px;
  height: 14px;
  object-fit: contain;
}

.toolbar-button:hover,
.new-group-inline:hover {
  border-color: var(--accent);
  color: var(--accent);
}

.playlist-scroll {
  min-height: 0;
  flex: 1;
  overflow: auto;
  padding: 0 9px 12px;
  scrollbar-width: thin;
  scrollbar-color: var(--track) transparent;
}

.group {
  overflow: hidden;
  background: var(--surface);
  box-shadow: var(--shadow-sm);
}



.group-selected {
  border-color: color-mix(in srgb, var(--accent) 58%, var(--line));
}

.group-header {
  min-height: 34px;
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 0 9px;
  border-bottom: 1px solid var(--line);
  background: var(--surface-2);
  cursor: pointer;
  user-select: none;
}

.group-header:hover {
  background: var(--hover);
}

.group-chevron {
  width: 14px;
  height: 14px;
  flex: 0 0 14px;
  object-fit: contain;
  opacity: 0.72;
  user-select: none;
}

.group-default {
  background: color-mix(in srgb, var(--accent) 8%, var(--surface-2));
}

.group-name {
  min-width: 0;
  flex: 1;
  overflow: hidden;
  color: var(--text);
  font-size: 14px;
  font-weight: 600;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.group-count {
  min-width: 20px;
  padding: 2px 5px;
  border-radius: 99px;
  background: var(--track);
  color: var(--muted);
  font-size: 12px;
  text-align: center;
}

.group-rename {
  min-width: 0;
  flex: 1;
  height: 24px;
  border: 1px solid var(--accent);
  border-radius: 4px;
  outline: none;
  padding: 0 5px;
  background: var(--surface);
  color: var(--text);
  font-size: 14px;
}

.creating-rename {
  flex: 1;
}

.group-songs {
  min-height: 35px;
  padding: 4px;
}

.song-item {
  position: relative;
  min-height: 34px;
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 5px 6px;
  border: 1px solid transparent;
  border-radius: 6px;
  cursor: pointer;
  transition: background 100ms ease, border-color 100ms ease;
}

.song-item:hover {
  background: var(--hover);
}

.song-actions {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 2px;
  opacity: 0;
  pointer-events: none;
  transition: opacity 100ms ease;
}

.song-item:hover .song-actions,
.song-item:focus-within .song-actions {
  opacity: 1;
  pointer-events: auto;
}

.song-action {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  padding: 0;
  border: 0;
  border-radius: 5px;
  background: transparent;
  cursor: pointer;
}

.song-action:hover {
  background: var(--surface-2);
}

.song-action img {
  width: 14px;
  height: 14px;
  object-fit: contain;
}

.song-add-menu {
  position: fixed;
  z-index: 60;
  max-height: 260px;
  overflow: auto;
  padding: 4px;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--surface);
  box-shadow: var(--shadow);
  color: var(--text);
}

.song-add-menu button {
  display: block;
  width: 100%;
  padding: 6px 8px;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font-size: 13px;
  text-align: left;
}

.song-add-menu button:hover {
  background: var(--hover);
}

.song-add-empty {
  padding: 6px 8px;
  color: var(--faint);
  font-size: 12px;
}

.song-current {
  border-color: color-mix(in srgb, var(--accent) 55%, transparent);
  background: color-mix(in srgb, var(--accent) 12%, transparent);
}

.song-playing {
  width: 15px;
  height: 15px;
  flex: 0 0 15px;
  object-fit: contain;
  opacity: 0.38;
  text-align: center;
}

.song-playing.active {
  opacity: 1;
}

.song-default-line {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.song-default-line {
  min-width: 0;
  flex: 1;
  color: var(--text);
  font-size: 14px;
}

.new-group-inline {
  width: 100%;
  padding: 8px;
}

.playlist-hint {
  padding: 8px 3px 0;
  color: var(--faint);
  font-size: 12px;
  line-height: 1.5;
  text-align: center;
}

.context-menu,
.move-menu {
  position: fixed;
  z-index: 30;
  min-width: 155px;
  padding: 4px;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--surface);
  box-shadow: var(--shadow);
}

.context-menu button,
.move-menu button {
  display: block;
  width: 100%;
  padding: 7px 8px;
  border: 0;
  border-radius: 4px;
  background: transparent;
  color: var(--text);
  cursor: pointer;
  font-size: 13px;
  text-align: left;
}

.context-menu button:hover,
.move-menu button:hover {
  background: var(--hover);
}

.move-menu {
  position: static;
  margin: 2px 0;
  padding: 2px;
  border-width: 1px 0;
  border-radius: 0;
  box-shadow: none;
}

</style>
