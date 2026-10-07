<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { register as registerShortcut, unregisterAll as unregisterAllShortcuts } from "@tauri-apps/plugin-global-shortcut";
import TitleBar from "./components/TitleBar.vue";
import PlayerPanel from "./components/PlayerPanel.vue";
import PlaylistManager from "./components/PlaylistManager.vue";
import SettingsModal from "./components/SettingsModal.vue";
import CloudPanel from "./components/CloudPanel.vue";
import { callMusicSource, getMusicSourcePlatforms } from "./lib/musicSourceRuntime";
import { DEFAULT_THEME_ID, themeById, themeStyle } from "./lib/themes";
import {
  acceleratorMatches,
  DEFAULT_SHORTCUTS,
  emptyGlobalShortcuts,
  normalizeShortcuts,
} from "./lib/shortcuts";
const STORAGE_KEY = "tingci-music-state-v1";
const ALLOWED_PLAY_MODES = ["sequence", "random", "repeat"];
const PAGE_WIDTH = 520; // 云库打开时的初始宽度
const MIN_WINDOW_WIDTH = 320; // 窗口最小宽度
const MIN_PLAYER_WIDTH = 320; // 仅限制内联拖拽时的播放页宽度
const DEFAULT_WINDOW_WIDTH = 380;
const FALLBACK_CLOUD_PLATFORMS = [
  { id: "netease", name: "网易" },
  { id: "tencent", name: "企鹅" },
  { id: "kugou", name: "酷狗" },
  { id: "kuwo", name: "酷我" },
];
const state = reactive({
  groups: [],
  songs: [],
  settings: {
    downloadDir: "",
    cacheDir: "",
    volume: 0.82,
    muted: false,
    playMode: "sequence",
    lastPlayedSongId: null,
    windowBounds: null,
    activeSourceId: "",
    activeSourceName: "暂无音源",
    quality: "320k",
    theme: DEFAULT_THEME_ID,
    globalShortcutsEnabled: false,
    shortcuts: { ...DEFAULT_SHORTCUTS },
    globalShortcuts: emptyGlobalShortcuts(),
  },
  playback: {
    currentSongId: null,
    isPlaying: false,
    currentTime: 0,
    selectedGroupId: "default",
    playMode: "sequence",
  },
});

const defaults = reactive({ downloadDir: "", cacheDir: "" });
const musicSources = ref([]);
const collapsed = ref(false);
const cloudOpen = ref(false);
const windowWidth = ref(Math.max(0, Math.round(window.innerWidth) || DEFAULT_WINDOW_WIDTH));
const playerWidth = ref(0);
const cloudWidth = computed(() =>
  cloudOpen.value ? Math.max(0, windowWidth.value - playerWidth.value) : 0,
);
const cloudPlatforms = ref([...FALLBACK_CLOUD_PLATFORMS]);
const settingsOpen = ref(false);
const deleteGroupTarget = ref(null);
const clearDefaultTarget = ref(false);
const importing = ref(false);
const toast = ref("");
const hydrated = ref(false);
const mediaReady = ref(false);
const expandedWindowHeight = ref(0);
let persistTimer = 0;
let toastTimer = 0;
let idSequence = 0;
let lastWindowHeight = 0;
let suppressResizeUncollapseUntil = 0;
let changedHashes = false;
const resizingPlayerWidth = ref(false);
let savedPlayerWidth = 0;
let restoreWindowWidth = 0;
let windowWasMaximized = false;
let resizeStartX = 0;
let resizeStartWidth = 0;
const appShellStyle = computed(() =>
  cloudOpen.value && playerWidth.value ? { flex: `0 0 ${playerWidth.value}px` } : null,
);
const cloudPanelStyle = computed(() =>
  cloudOpen.value ? { flex: `0 0 ${cloudWidth.value}px` } : null,
);
let autoSkipDepth = 0;
const playNextQueue = ref({}); // "下一首播放"队列：按分组 id 分开维护
const playHistory = ref([]); // 播放过的歌曲 id，最多 20 首
let playHistoryIndex = -1; // 当前歌曲在 playHistory 中的位置
let playHistoryGroup = ""; // 这份历史所属的分组
let playIntentToken = 0; // 播放请求序号，用于丢弃被取代的下载/校验结果
let mediaVerificationResolve = null;
let mediaVerificationTimer = 0;

function createId(prefix) {
  idSequence += 1;
  return `${prefix}-${Date.now().toString(36)}-${idSequence.toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 7)}`;
}

function makeDefaultGroup() {
  return {
    id: "default",
    name: "试听列表",
    isDefault: true,
    order: 0,
    createdAt: Date.now(),
    collapsed: false,
  };
}

function defaultSettings() {
  return {
    downloadDir: "",
    cacheDir: "",
    volume: 0.82,
    muted: false,
    playMode: "sequence",
    lastPlayedSongId: null,
    windowBounds: null,
    activeSourceId: "",
    activeSourceName: "暂无音源",
    quality: "320k",
    theme: DEFAULT_THEME_ID,
    globalShortcutsEnabled: false,
    shortcuts: { ...DEFAULT_SHORTCUTS },
    globalShortcuts: emptyGlobalShortcuts(),
  };
}

function numberOr(value, fallback) {
  const number = Number(value);
  return Number.isFinite(number) ? number : fallback;
}

function hydrateSavedState() {
  let saved = null;
  try {
    saved = JSON.parse(localStorage.getItem(STORAGE_KEY) || "null");
  } catch {
    saved = null;
  }
  const savedGroups = Array.isArray(saved?.groups) ? saved.groups : [];
  const groups = savedGroups
    .filter((group) => group && typeof group.id === "string" && group.id)
    .map((group, index) => ({
      id: group.id,
      name: String(group.name || "未命名分组").slice(0, 40),
      isDefault: group.id === "default",
      order: numberOr(group.order, index + 1),
      createdAt: numberOr(group.createdAt, Date.now()),
      collapsed: Boolean(group.collapsed),
    }));
  const existingDefault = groups.find((group) => group.id === "default");
  if (!existingDefault) groups.unshift(makeDefaultGroup());
  else {
    const existingName = existingDefault.name;
    const existingCollapsed = existingDefault.collapsed;
    Object.assign(existingDefault, makeDefaultGroup(), { createdAt: existingDefault.createdAt });
    existingDefault.name = existingName || "试听列表";
    existingDefault.collapsed = Boolean(existingCollapsed);
  }
  groups.sort((left, right) => {
    if (left.isDefault !== right.isDefault) return left.isDefault ? -1 : 1;
    return left.order - right.order;
  });
  state.groups = groups;

  const groupIds = new Set(groups.map((group) => group.id));
  const savedSongs = Array.isArray(saved?.songs) ? saved.songs : [];
  state.songs = savedSongs
    .filter(
      (song) =>
        song &&
        typeof song.id === "string" &&
        song.id &&
        typeof song.filePath === "string" &&
        !song.loading,
    )
    .map((song, index) => ({
      id: song.id,
      title: String(song.title || "未知歌曲"),
      artist: String(song.artist || "未知歌手"),
      album: String(song.album || "未知专辑"),
      duration: numberOr(song.duration, 0),
      filePath: song.filePath,
      originalPath: String(song.originalPath || song.filePath),
      coverPath: song.coverPath || null,
      lyrics: String(song.lyrics || ""),
      lyricsPath: song.lyricsPath || null,
      cloudId: String(song.cloudId || ""),
      cloudSourceId: String(song.cloudSourceId || ""),
      cloudPlatform: String(song.cloudPlatform || ""),
      cloudHash: String(song.cloudHash || ""),
      cloudAlbumId: String(song.cloudAlbumId || ""),
      cloudStrMediaMid: String(song.cloudStrMediaMid || ""),
      cloudUrl: String(song.cloudUrl || ""),
      cloudCoverUrl: String(song.cloudCoverUrl || ""),
      cloudLyrics: String(song.cloudLyrics || ""),
      cloudQuality: String(song.cloudQuality || ""),
      cloudQualities: Array.isArray(song.cloudQualities) ? song.cloudQualities.map(String) : [],
      quality: String(song.quality || ""),
      sourceHash: String(song.sourceHash || ""),
      groupId: groupIds.has(song.groupId) ? song.groupId : "default",
      order: numberOr(song.order, index),
      addedAt: numberOr(song.addedAt, Date.now()),
    }));

  state.settings = Object.assign(defaultSettings(), saved?.settings || {});
  if (!ALLOWED_PLAY_MODES.includes(state.settings.playMode)) state.settings.playMode = "sequence";
  state.settings.volume = Math.min(1, Math.max(0, numberOr(state.settings.volume, 0.82)));
  state.settings.muted = Boolean(state.settings.muted);
  if (!state.settings.activeSourceId) state.settings.activeSourceId = "";
  if (!state.settings.activeSourceName) state.settings.activeSourceName = "暂无音源";
  if (!["128k", "320k", "flac", "flac24bit"].includes(state.settings.quality)) state.settings.quality = "320k";
  state.settings.theme = themeById(state.settings.theme).id;
  state.settings.globalShortcutsEnabled = Boolean(state.settings.globalShortcutsEnabled);
  state.settings.shortcuts = normalizeShortcuts(state.settings.shortcuts, DEFAULT_SHORTCUTS);
  state.settings.globalShortcuts = normalizeShortcuts(state.settings.globalShortcuts, emptyGlobalShortcuts());
  // 不记忆窗口大小与位置：每次启动都用默认尺寸和居中位置（全新窗口）。
  state.settings.windowBounds = null;
  if (state.settings.lastPlayedSongId && !state.songs.some((song) => song.id === state.settings.lastPlayedSongId)) {
    state.settings.lastPlayedSongId = null;
  }
  const playback = saved?.playback || {};
  const restoredSong = state.songs.find((song) => song.id === playback.currentSongId);
  state.playback.currentSongId = restoredSong?.id || state.settings.lastPlayedSongId || null;
  state.playback.currentTime = Math.max(0, numberOr(playback.currentTime, 0));
  state.playback.selectedGroupId = groupIds.has(playback.selectedGroupId)
    ? playback.selectedGroupId
    : restoredSong?.groupId || "default";
  state.playback.playMode = state.settings.playMode;
  state.playback.isPlaying = false;
}

function schedulePersist() {
  if (!hydrated.value) return;
  window.clearTimeout(persistTimer);
  persistTimer = window.setTimeout(persistNow, 180);
}

function persistNow() {
  if (!hydrated.value) return;
  try {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        groups: state.groups,
        songs: state.songs,
        settings: state.settings,
        playback: state.playback,
      }),
    );
  } catch {
    // localStorage can be unavailable in a private webview; playback still works in memory.
  }
}

function showToast(message, duration = 3600) {
  toast.value = message;
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    toast.value = "";
  }, duration);
}

async function backfillSourceHashes() {
  let changed = false;
  // 尚未下载的云库歌曲没有文件，跳过哈希计算。
  const missingSongs = state.songs.filter((song) => song.filePath && !song.sourceHash);
  let hashError = "";
  for (const song of missingSongs) {
    try {
      song.sourceHash = String(await invoke("hash_audio_file", { filePath: song.filePath }));
      changed = true;
    } catch (error) {
      hashError = String(error);
    }
  }

  const seenHashesByGroup = new Map();
  const uniqueSongs = [];
  for (const song of state.songs) {
    const seenHashes = seenHashesByGroup.get(song.groupId) || new Set();
    if (song.sourceHash && seenHashes.has(song.sourceHash)) {
      changed = true;
      continue;
    }
    if (song.sourceHash) seenHashes.add(song.sourceHash);
    seenHashesByGroup.set(song.groupId, seenHashes);
    uniqueSongs.push(song);
  }

  if (uniqueSongs.length !== state.songs.length) {
    const removedCount = state.songs.length - uniqueSongs.length;
    const removedIds = new Set(state.songs.filter((song) => !uniqueSongs.includes(song)).map((song) => song.id));
    state.songs = uniqueSongs;
    if (removedIds.has(state.playback.currentSongId)) {
      state.playback.currentSongId = null;
      state.playback.currentTime = 0;
      state.playback.isPlaying = false;
    }
    if (removedIds.has(state.settings.lastPlayedSongId)) state.settings.lastPlayedSongId = null;
    showToast(`已移除 ${removedCount} 条重复音源记录（本地文件保留）`);
  }
  if (hashError) showToast(`部分歌曲无法计算音源哈希：${hashError}`);
  return changed;
}

async function initialize() {
  hydrateSavedState();
  try {
    const paths = await invoke("get_app_paths");
    defaults.downloadDir = paths.download_dir;
    defaults.cacheDir = paths.cache_dir;
    if (!state.settings.downloadDir) state.settings.downloadDir = paths.download_dir;
    if (!state.settings.cacheDir) state.settings.cacheDir = paths.cache_dir;
    const configuredPaths = await invoke("set_app_paths", {
      downloadDir: state.settings.downloadDir,
      cacheDir: state.settings.cacheDir,
    });
    state.settings.downloadDir = configuredPaths.download_dir;
    state.settings.cacheDir = configuredPaths.cache_dir;
    const hashesChanged = await backfillSourceHashes();
    await refreshMusicSources();
    const mediaPaths = state.songs.flatMap((song) => [song.filePath, song.coverPath, song.lyricsPath].filter(Boolean));
    if (mediaPaths.length) await invoke("allow_media_files", { paths: mediaPaths });
    if (hashesChanged) changedHashes = true;
  } catch (error) {
    showToast(`初始化目录失败：${error}`);
  }
  mediaReady.value = true;
  hydrated.value = true;
  if (changedHashes) persistNow();
  void applyGlobalShortcuts();
  // 后台准备云库"来源筛选"（执行音源脚本较重，放到启动后再做，避免打开云库时卡顿）
  window.setTimeout(() => {
    void refreshCloudPlatforms();
  }, 1200);
}

async function refreshMusicSources() {
  try {
    const imported = await invoke("list_music_sources", {
      cacheDir: state.settings.cacheDir || defaults.cacheDir,
    });
    musicSources.value = Array.isArray(imported) ? imported : [];
    if (!musicSources.value.some((source) => source.id === state.settings.activeSourceId)) {
      state.settings.activeSourceId = musicSources.value[0]?.id || "";
      state.settings.activeSourceName = musicSources.value[0]?.name || "暂无音源";
    }
  } catch {
    musicSources.value = [];
    state.settings.activeSourceId = "";
    state.settings.activeSourceName = "暂无音源";
  }
}

// 云库"来源筛选"下拉：跟随当前音源所支持的平台
async function refreshCloudPlatforms() {
  const source = musicSources.value.find((item) => item.id === state.settings.activeSourceId);
  let script = source?.script || "";
  if (!script && source?.path) {
    try {
      script = await invoke("read_music_source", {
        sourcePath: source.path,
        cacheDir: state.settings.cacheDir || defaults.cacheDir,
      });
    } catch {
      script = "";
    }
  }
  if (!script) {
    cloudPlatforms.value = [...FALLBACK_CLOUD_PLATFORMS];
    return;
  }
  try {
    const platforms = await getMusicSourcePlatforms(source, script);
    cloudPlatforms.value = platforms.length ? platforms : [...FALLBACK_CLOUD_PLATFORMS];
  } catch {
    cloudPlatforms.value = [...FALLBACK_CLOUD_PLATFORMS];
  }
}

const orderedGroups = computed(() =>
  [...state.groups].sort((left, right) => {
    if (left.isDefault !== right.isDefault) return left.isDefault ? -1 : 1;
    return left.order - right.order;
  }),
);

const themeVars = computed(() => themeStyle(state.settings.theme));

const cacheKeepPaths = computed(() =>
  state.songs
    .flatMap((song) => [song.filePath, song.coverPath, song.lyricsPath])
    .filter(Boolean),
);

// 已添加的歌单分组名（云库详情里判断"已添加"）
const playlistNames = computed(() =>
  state.groups.filter((group) => !group.isDefault).map((group) => group.name),
);

const currentSong = computed(
  () => state.songs.find((song) => song.id === state.playback.currentSongId) || null,
);

const defaultCloudKeys = computed(() =>
  state.songs
    .filter((song) => song.groupId === "default" && song.cloudId)
    .map((song) => `${song.cloudSourceId || state.settings.activeSourceId}|${song.cloudId}`),
);

const queue = computed(() =>
  state.songs
    // 播放队列跟随"正在播放歌曲"所在的分组，避免点击别的分组后切歌跳组。
    .filter((song) => song.groupId === currentGroupId())
    .sort((left, right) => left.order - right.order),
);

function selectGroup(groupId) {
  if (state.groups.some((group) => group.id === groupId)) {
    state.playback.selectedGroupId = groupId;
    schedulePersist();
  }
}

function toggleGroup(groupId) {
  const group = state.groups.find((item) => item.id === groupId);
  if (!group) return;
  group.collapsed = !group.collapsed;
  schedulePersist();
}

function createGroup(name) {
  const trimmed = String(name || "").trim();
  if (!trimmed) return;
  const maxOrder = state.groups.reduce((maximum, group) => Math.max(maximum, numberOr(group.order, 0)), 0);
  state.groups.push({
    id: createId("group"),
    name: trimmed.slice(0, 40),
    isDefault: false,
    order: maxOrder + 1,
    createdAt: Date.now(),
    collapsed: false,
  });
  showToast(`已创建分组“${trimmed.slice(0, 40)}”`);
  schedulePersist();
}

function renameGroup({ groupId, name }) {
  const group = state.groups.find((item) => item.id === groupId);
  if (!group || group.isDefault || !name.trim()) return;
  group.name = name.trim().slice(0, 40);
  schedulePersist();
}

function deleteGroup(groupId) {
  const group = state.groups.find((item) => item.id === groupId);
  if (!group || group.isDefault) return;
  const groupSongs = state.songs.filter((song) => song.groupId === groupId);
  if (groupSongs.length) {
    deleteGroupTarget.value = groupId;
    return;
  }
  commitDeleteGroup(groupId, true);
}

function cancelDeleteGroup() {
  deleteGroupTarget.value = null;
}

function requestClearDefault() {
  clearDefaultTarget.value = true;
}

function cancelClearDefault() {
  clearDefaultTarget.value = false;
}

async function confirmClearDefault() {
  clearDefaultTarget.value = false;
  const removedSongs = state.songs.filter((song) => song.groupId === "default");
  if (!removedSongs.length) {
    showToast("试听列表已经是空的");
    return;
  }
  const removedIds = new Set(removedSongs.map((song) => song.id));
  state.songs = state.songs.filter((song) => song.groupId !== "default");
  if (removedIds.has(state.playback.currentSongId)) {
    state.playback.currentSongId = null;
    state.playback.currentTime = 0;
    state.playback.isPlaying = false;
  }
  if (removedIds.has(state.settings.lastPlayedSongId)) state.settings.lastPlayedSongId = null;
  state.playback.selectedGroupId = "default";
  showToast("已清空试听列表（音乐文件保留，可在设置中清除缓存）");
  schedulePersist();
}

function confirmDeleteGroup(moveSongs) {
  const groupId = deleteGroupTarget.value;
  deleteGroupTarget.value = null;
  if (groupId) commitDeleteGroup(groupId, moveSongs);
}

function commitDeleteGroup(groupId, moveSongs) {
  const group = state.groups.find((item) => item.id === groupId);
  if (!group || group.isDefault) return;
  const defaultGroup = state.groups.find((item) => item.id === "default") || makeDefaultGroup();
  const groupSongs = state.songs.filter((song) => song.groupId === groupId);
  if (moveSongs) {
    const defaultMax = state.songs
      .filter((song) => song.groupId === "default")
      .reduce((maximum, song) => Math.max(maximum, numberOr(song.order, -1)), -1);
    const defaultHashes = new Set(
      state.songs.filter((song) => song.groupId === "default" && song.sourceHash).map((song) => song.sourceHash),
    );
    const removedDuplicateIds = new Set();
    groupSongs.forEach((song, index) => {
      if (song.sourceHash && defaultHashes.has(song.sourceHash)) {
        removedDuplicateIds.add(song.id);
        return;
      }
      song.groupId = "default";
      song.order = defaultMax + 1 + index;
      if (song.sourceHash) defaultHashes.add(song.sourceHash);
    });
    if (removedDuplicateIds.size) {
      state.songs = state.songs.filter((song) => !removedDuplicateIds.has(song.id));
      if (removedDuplicateIds.has(state.playback.currentSongId)) {
        state.playback.currentSongId = null;
        state.playback.currentTime = 0;
        state.playback.isPlaying = false;
        state.settings.lastPlayedSongId = null;
      }
    }
  } else {
    const removedIds = new Set(groupSongs.map((song) => song.id));
    state.songs = state.songs.filter((song) => !removedIds.has(song.id));
    if (removedIds.has(state.playback.currentSongId)) {
      state.playback.currentSongId = null;
      state.playback.currentTime = 0;
      state.playback.isPlaying = false;
      state.settings.lastPlayedSongId = null;
    }
  }
  state.groups = state.groups.filter((item) => item.id !== groupId);
  if (state.playback.selectedGroupId === groupId) state.playback.selectedGroupId = defaultGroup.id;
  showToast(moveSongs ? `已将 ${groupSongs.length} 首歌曲移回试听列表` : "已删除分组，未将歌曲加入试听列表（本地文件保留）");
  schedulePersist();
}

function renumberGroup(groupId) {
  state.songs
    .filter((song) => song.groupId === groupId)
    .sort((left, right) => left.order - right.order)
    .forEach((song, index) => {
      song.order = index;
    });
}

function moveSong({ songId, groupId, targetSongId = "", afterTarget = false }) {
  const song = state.songs.find((item) => item.id === songId);
  const targetGroup = state.groups.find((group) => group.id === groupId);
  if (!song || !targetGroup) return;
  if (song.id === targetSongId) return;
  const oldGroupId = song.groupId;
  if (
    oldGroupId !== groupId &&
    song.sourceHash &&
    state.songs.some((item) => item.groupId === groupId && item.sourceHash === song.sourceHash)
  ) {
    showToast("目标分组已存在相同音源，未移动");
    return;
  }
  const targetSongs = state.songs
    .filter((item) => item.groupId === groupId && item.id !== songId)
    .sort((left, right) => left.order - right.order);
  let insertAt = targetSongs.length;
  if (targetSongId) {
    const targetIndex = targetSongs.findIndex((item) => item.id === targetSongId);
    if (targetIndex >= 0) insertAt = targetIndex + (afterTarget ? 1 : 0);
  }
  targetSongs.splice(insertAt, 0, song);
  song.groupId = groupId;
  targetSongs.forEach((item, index) => {
    item.order = index;
  });
  if (oldGroupId !== groupId) renumberGroup(oldGroupId);
  state.playback.selectedGroupId = groupId;
  schedulePersist();
}

function copySong({ songId, groupId, targetSongId = "", afterTarget = false }) {
  const sourceSong = state.songs.find((item) => item.id === songId);
  const targetGroup = state.groups.find((group) => group.id === groupId);
  if (!sourceSong || !targetGroup || sourceSong.groupId === groupId) return;
  if (
    sourceSong.sourceHash &&
    state.songs.some((item) => item.groupId === groupId && item.sourceHash === sourceSong.sourceHash)
  ) {
    showToast("目标分组已存在相同音源，未复制");
    return;
  }
  const targetSongs = state.songs
    .filter((item) => item.groupId === groupId)
    .sort((left, right) => left.order - right.order);
  let insertAt = targetSongs.length;
  if (targetSongId) {
    const targetIndex = targetSongs.findIndex((item) => item.id === targetSongId);
    if (targetIndex >= 0) insertAt = targetIndex + (afterTarget ? 1 : 0);
  }
  const copiedSong = {
    ...sourceSong,
    id: createId("song"),
    groupId,
    addedAt: Date.now(),
  };
  targetSongs.splice(insertAt, 0, copiedSong);
  targetSongs.forEach((item, index) => {
    item.order = index;
  });
  state.songs.push(copiedSong);
  state.playback.selectedGroupId = groupId;
  showToast(`已复制到“${targetGroup.name}”`);
  schedulePersist();
}

async function cleanupRemovedSongs(removedSongs) {
  const sharedPaths = new Set(
    state.songs
      .flatMap((song) => [song.filePath, song.coverPath, song.lyricsPath].filter(Boolean))
      .map((path) => path.toLowerCase()),
  );
  const cacheRoot = (state.settings.cacheDir || defaults.cacheDir).replace(/[\\/]+$/, "").toLowerCase();
  const cachedPaths = [
    ...new Set(
      removedSongs
        .flatMap((song) => [song.filePath, song.coverPath, song.lyricsPath].filter(Boolean))
        .filter((path) => path.toLowerCase().startsWith(`${cacheRoot}\\`) || path.toLowerCase().startsWith(`${cacheRoot}/`))
        .filter((path) => !sharedPaths.has(path.toLowerCase())),
    ),
  ];
  if (!cachedPaths.length) return 0;
  await invoke("remove_cached_song_files", {
    cacheDir: state.settings.cacheDir || defaults.cacheDir,
    paths: cachedPaths,
  });
  return cachedPaths.length;
}

async function removeSong(songId) {
  const index = state.songs.findIndex((song) => song.id === songId);
  if (index < 0) return;
  const wasCurrent = state.playback.currentSongId === songId;
  state.songs.splice(index, 1);
  if (wasCurrent) {
    state.playback.currentSongId = null;
    state.playback.currentTime = 0;
    state.playback.isPlaying = false;
    state.settings.lastPlayedSongId = null;
  }
  for (const queue of Object.values(playNextQueue.value)) {
    if (!Array.isArray(queue)) continue;
    const index = queue.indexOf(songId);
    if (index >= 0) queue.splice(index, 1);
  }
  const historyPos = playHistory.value.indexOf(songId);
  if (historyPos >= 0) {
    playHistory.value.splice(historyPos, 1);
    if (playHistoryIndex > historyPos) playHistoryIndex -= 1;
    if (playHistoryIndex >= playHistory.value.length) playHistoryIndex = playHistory.value.length - 1;
  }
  state.songs
    .filter((song) => song.groupId === state.playback.selectedGroupId)
    .sort((left, right) => left.order - right.order)
    .forEach((song, order) => {
      song.order = order;
    });
  // 仅从列表移除，不删除音乐文件；缓存文件由设置里的"清除缓存"统一清理。
  showToast("已从列表移除（音乐文件保留）");
  schedulePersist();
}

async function revealSong(songId) {
  const song = state.songs.find((item) => item.id === songId);
  if (!song) return;
  try {
    await revealItemInDir(song.filePath);
  } catch (error) {
    showToast(`无法打开文件夹：${error}`);
  }
}

async function importSongs() {
  if (importing.value) return;
  importing.value = true;
  try {
    const sourcePaths = await invoke("pick_audio_files");
    if (!sourcePaths?.length) return;
    const outcome = await invoke("import_songs", {
      downloadDir: state.settings.downloadDir || defaults.downloadDir,
      cacheDir: state.settings.cacheDir || defaults.cacheDir,
      sourcePaths,
      existingHashes: state.songs.filter((song) => song.groupId === "default").map((song) => song.sourceHash).filter(Boolean),
    });
    const defaultGroup = state.groups.find((group) => group.id === "default") || makeDefaultGroup();
    if (!state.groups.some((group) => group.id === defaultGroup.id)) state.groups.unshift(defaultGroup);
    // 新导入的歌曲放在试听列表最上面
    const defaultOrders = state.songs
      .filter((song) => song.groupId === "default")
      .map((song) => numberOr(song.order, 0));
    let order = (defaultOrders.length ? Math.min(...defaultOrders) : 0) - outcome.songs.length;
    outcome.songs.forEach((song) => {
      state.songs.push({
        id: createId("song"),
        title: song.title,
        artist: song.artist,
        album: song.album,
        duration: song.duration,
        filePath: song.file_path,
        originalPath: song.original_path,
        coverPath: song.cover_path || null,
        lyrics: song.lyrics || "",
        lyricsPath: song.lyrics_path || null,
        cloudId: "",
        cloudSourceId: "",
        quality: "",
        sourceHash: song.source_hash,
        groupId: "default",
        order: order++,
        addedAt: Date.now(),
      });
    });
    state.playback.selectedGroupId = "default";
    const messages = [`已导入 ${outcome.songs.length} 首歌曲到试听列表`];
    if (outcome.errors?.length) messages.push(`跳过 ${outcome.errors.length} 个文件`);
    showToast(messages.join("；"));
    schedulePersist();
  } catch (error) {
    showToast(`导入失败：${error}`);
  } finally {
    importing.value = false;
  }
}

const CLOUD_FAILURE_MESSAGE = "无法解析或播放该歌曲，未添加到试听列表；建议切换音源后重试";

function sourceResultUrl(value) {
  if (typeof value === "string") return value;
  if (value && typeof value === "object") return value.url || value.data || value.result || "";
  return "";
}

async function resolveImportedSource(media, cloudSong) {
  const source = musicSources.value.find(
    (item) => item.id === (cloudSong.sourceId || state.settings.activeSourceId),
  );
  if (!source) return media;
  let script = source.script;
  if (!script && source.path) {
    script = await invoke("read_music_source", {
      sourcePath: source.path,
      cacheDir: state.settings.cacheDir || defaults.cacheDir,
    });
  }
  if (!script) return media;
  const platformSource = {
    netease: "wy",
    tencent: "tx",
    kugou: "kg",
    kuwo: "kw",
  };
  const preferredSource = platformSource[cloudSong.platform || "netease"] || "wy";
  const musicInfo = {
    id: cloudSong.cloudId,
    songmid: cloudSong.cloudId,
    hash: cloudSong.hash || cloudSong.cloudId,
    copyrightId: cloudSong.cloudId,
    albumId: cloudSong.albumId || "",
    strMediaMid: cloudSong.strMediaMid || "",
    name: media.title || cloudSong.title,
    title: media.title || cloudSong.title,
    artist: media.artist || cloudSong.artist,
    album: media.album || cloudSong.album,
    duration: media.duration || cloudSong.duration || 0,
    source: preferredSource,
  };
  // 优先用歌曲自己选定的音质，其次用设置里的优先音质
  const quality = cloudSong.quality || state.settings.quality || "320k";
  let url = "";
  try {
    url = sourceResultUrl(
      await callMusicSource(source, script, "musicUrl", musicInfo, quality, preferredSource),
    );
  } catch (error) {
    if (/^https?:\/\//i.test(media.url || "")) return media;
    throw error;
  }
  if (!/^https?:\/\//i.test(url)) {
    if (/^https?:\/\//i.test(media.url || "")) return media;
    throw new Error(CLOUD_FAILURE_MESSAGE);
  }
  const resolved = { ...media, url };
  try {
    const cover = sourceResultUrl(
      await callMusicSource(source, script, "pic", musicInfo, quality, preferredSource),
    );
    if (cover) resolved.cover_url = cover;
  } catch {}
  try {
    const lyrics = await callMusicSource(source, script, "lyric", musicInfo, quality, preferredSource);
    if (typeof lyrics === "string" && lyrics) resolved.lyrics = lyrics;
  } catch {}
  return resolved;
}

function ensureDefaultGroup() {
  const defaultGroup = state.groups.find((group) => group.id === "default") || makeDefaultGroup();
  if (!state.groups.some((group) => group.id === defaultGroup.id)) state.groups.unshift(defaultGroup);
  return defaultGroup;
}

function nextGroupOrder(groupId) {
  return (
    state.songs
      .filter((song) => song.groupId === groupId)
      .reduce((maximum, song) => Math.max(maximum, numberOr(song.order, -1)), -1) + 1
  );
}

function topGroupOrder(groupId) {
  const orders = state.songs
    .filter((song) => song.groupId === groupId)
    .map((song) => numberOr(song.order, 0));
  return orders.length ? Math.min(...orders) - 1 : 0;
}

function existingCloudSong(cloudSong, excludedSongId = "", groupId = "") {
  const cloudId = String(cloudSong?.cloudId || "");
  if (!cloudId) return null;
  const sourceId = String(cloudSong?.sourceId || state.settings.activeSourceId || "");
  return (
    state.songs.find(
      (song) =>
        song.id !== excludedSongId &&
        (!groupId || song.groupId === groupId) &&
        song.cloudId === cloudId &&
        (!song.cloudSourceId || !sourceId || song.cloudSourceId === sourceId),
    ) || null
  );
}

function existingCloudSongInGroup(cloudSong, groupId) {
  return existingCloudSong(cloudSong, "", groupId);
}

function beginMediaVerification(timeoutMs = 8000) {
  finishMediaVerification(false);
  return new Promise((resolve) => {
    mediaVerificationResolve = resolve;
    mediaVerificationTimer = window.setTimeout(() => {
      finishMediaVerification(false);
    }, timeoutMs);
  });
}

function finishMediaVerification(succeeded) {
  const resolve = mediaVerificationResolve;
  mediaVerificationResolve = null;
  window.clearTimeout(mediaVerificationTimer);
  mediaVerificationTimer = 0;
  if (resolve) resolve(Boolean(succeeded));
}

function discardTemporaryCloudSong(record) {
  const index = state.songs.findIndex((song) => song.id === record?.id);
  if (index >= 0) state.songs.splice(index, 1);
  if (state.playback.currentSongId === record?.id) {
    state.playback.currentSongId = null;
    state.playback.currentTime = 0;
    state.playback.isPlaying = false;
    state.settings.lastPlayedSongId = null;
  }
}

async function removeTemporaryCloudSong(record) {
  discardTemporaryCloudSong(record);
  try {
    await cleanupRemovedSongs([record]);
  } catch {
    // The playback failure message remains the primary error shown to the user.
  }
  schedulePersist();
}

function addSongRecordToGroup(songData, groupId) {
  if (
    songData.sourceHash &&
    state.songs.some((song) => song.groupId === groupId && song.sourceHash === songData.sourceHash)
  ) {
    return null;
  }
  const record = {
    ...songData,
    id: createId("song"),
    groupId,
    order: nextGroupOrder(groupId),
    addedAt: Date.now(),
  };
  state.songs.push(record);
  return record;
}

async function prepareCloudSong(cloudSong, groupId, options = {}) {
  // 必须通过响应式代理写入，否则 PlayerPanel 不会感知到 filePath 变化而设置音源。
  const targetRecord = options.targetRecord ? reactive(options.targetRecord) : null;
  const verifyPlayback = Boolean(options.verifyPlayback && targetRecord);
  const existing = options.skipExisting ? null : existingCloudSong(cloudSong, targetRecord?.id || "");
  if (existing) {
    if (targetRecord) discardTemporaryCloudSong(targetRecord);
    if (existing.groupId === groupId) return existing;
    return addSongRecordToGroup(existing, groupId);
  }

  let media = null;
  if (cloudSong?.cloudId) {
    if ((cloudSong.platform || "netease") === "netease") {
      media = await invoke("get_cloud_media", {
        songId: cloudSong.cloudId,
        sourceId: cloudSong.sourceId || state.settings.activeSourceId,
        quality: cloudSong.quality || state.settings.quality || "320k",
        fallbackUrl: cloudSong.url || "",
      });
    } else {
      media = {
        url: cloudSong.url || "",
        title: cloudSong.title || "未知歌曲",
        artist: cloudSong.artist || "未知歌手",
        album: cloudSong.album || "未知专辑",
        duration: cloudSong.duration || 0,
        cover_url: cloudSong.coverUrl || "",
        lyrics: cloudSong.lyrics || "",
      };
    }
    media = await resolveImportedSource(media, cloudSong);
  }
  const url = media?.url || cloudSong?.url || "";
  if (!url) throw new Error(CLOUD_FAILURE_MESSAGE);
  const title = cloudSong.title && cloudSong.title !== "未知歌曲"
    ? cloudSong.title
    : media?.title && media.title !== "未知歌曲"
      ? media.title
      : "未知歌曲";
  const artist = cloudSong.artist && cloudSong.artist !== "未知歌手"
    ? cloudSong.artist
    : media?.artist && media.artist !== "未知歌手"
      ? media.artist
      : "未知歌手";
  const album = cloudSong.album && cloudSong.album !== "未知专辑"
    ? cloudSong.album
    : media?.album && media.album !== "未知专辑"
      ? media.album
      : "未知专辑";
  const outcome = await invoke("download_remote_song", {
    downloadDir: state.settings.cacheDir || defaults.cacheDir,
    cacheDir: state.settings.cacheDir || defaults.cacheDir,
    url,
    suggestedName: `${title} - ${artist}`,
    existingHashes: state.songs
      .filter((song) => song.groupId === groupId)
      .map((song) => song.sourceHash)
      .filter(Boolean),
    coverUrl: media?.cover_url || cloudSong.coverUrl || "",
    lyrics: media?.lyrics || cloudSong.lyrics || "",
    expectedDuration: Math.max(0, Math.round(media?.duration || cloudSong.duration || 0)),
  });
  await invoke("allow_media_files", {
    paths: [outcome.file_path, outcome.cover_path, outcome.lyrics_path].filter(Boolean),
  });
  const recordData = {
    title,
    artist,
    album: outcome.album && outcome.album !== "未知专辑" ? outcome.album : album,
    duration: outcome.duration,
    filePath: outcome.file_path,
    originalPath: outcome.original_path,
    coverPath: outcome.cover_path || null,
    lyrics: outcome.lyrics || media?.lyrics || cloudSong.lyrics || "",
    lyricsPath: outcome.lyrics_path || null,
    cloudId: String(cloudSong.cloudId || ""),
    cloudSourceId: String(cloudSong.sourceId || state.settings.activeSourceId || ""),
    quality: cloudSong.quality || state.settings.quality || "320k",
    sourceHash: outcome.source_hash,
  };

  if (!targetRecord) return addSongRecordToGroup(recordData, groupId);

  Object.assign(targetRecord, recordData, { loading: true });
  if (
    targetRecord.sourceHash &&
    state.songs.some(
      (song) =>
        song.id !== targetRecord.id &&
        song.groupId === groupId &&
        song.sourceHash === targetRecord.sourceHash,
    )
  ) {
    throw new Error(CLOUD_FAILURE_MESSAGE);
  }
  if (!verifyPlayback) return targetRecord;

  const verification = beginMediaVerification();
  await nextTick();
  state.playback.currentSongId = targetRecord.id;
  state.playback.selectedGroupId = groupId;
  state.playback.currentTime = 0;
  state.playback.isPlaying = true;
  state.settings.lastPlayedSongId = targetRecord.id;
  const succeeded = await verification;
  if (!succeeded) throw new Error(CLOUD_FAILURE_MESSAGE);
  targetRecord.loading = false;
  schedulePersist();
  return targetRecord;
}

// 同一首云库歌曲 + 相同音质已经下载过时，直接复用缓存文件，避免重复下载。
function reusableCloudSource(cloudSong, excludedId = "") {
  const cloudId = String(cloudSong?.cloudId || "");
  if (!cloudId) return null;
  const sourceId = String(cloudSong?.sourceId || state.settings.activeSourceId || "");
  const quality = String(cloudSong?.quality || state.settings.quality || "320k");
  return (
    state.songs.find(
      (song) =>
        song.id !== excludedId &&
        song.filePath &&
        song.cloudId === cloudId &&
        (!song.cloudSourceId || !sourceId || song.cloudSourceId === sourceId) &&
        String(song.cloudQuality || song.quality || "") === quality,
    ) || null
  );
}

function createCloudPlaceholder(cloudSong, groupId) {
  const reusable = reusableCloudSource(cloudSong);
  const data = {
    id: createId("song"),
    title: String(cloudSong.title || "未知歌曲"),
    artist: String(cloudSong.artist || "未知歌手"),
    album: String(cloudSong.album || "未知专辑"),
    duration: reusable?.duration || Math.max(0, Number(cloudSong.duration) || 0),
    filePath: reusable?.filePath || "",
    originalPath: reusable?.originalPath || "",
    coverPath: reusable?.coverPath || null,
    lyrics: reusable?.lyrics || "",
    lyricsPath: reusable?.lyricsPath || null,
    cloudId: String(cloudSong.cloudId || ""),
    cloudSourceId: String(cloudSong.sourceId || state.settings.activeSourceId || ""),
    cloudPlatform: String(cloudSong.platform || "netease"),
    cloudHash: String(cloudSong.hash || ""),
    cloudAlbumId: String(cloudSong.albumId || ""),
    cloudStrMediaMid: String(cloudSong.strMediaMid || ""),
    cloudUrl: String(cloudSong.url || ""),
    cloudCoverUrl: String(cloudSong.coverUrl || ""),
    cloudLyrics: String(cloudSong.lyrics || ""),
    cloudQuality: String(cloudSong.quality || state.settings.quality || "320k"),
    cloudQualities: Array.isArray(cloudSong.qualities) ? cloudSong.qualities.map(String) : [],
    quality: String(cloudSong.quality || state.settings.quality || "320k"),
    sourceHash: reusable?.sourceHash || "",
    groupId,
    // 试听列表默认新歌在最上面；其它分组按顺序追加。
    order: groupId === "default" ? topGroupOrder(groupId) : nextGroupOrder(groupId),
    addedAt: Date.now(),
    loading: false,
  };
  state.songs.push(data);
  return state.songs[state.songs.length - 1];
}

function cloudSongFromRecord(record) {
  return {
    quality: record.cloudQuality || state.settings.quality || "320k",
    cloudId: record.cloudId,
    sourceId: record.cloudSourceId,
    platform: record.cloudPlatform || "netease",
    hash: record.cloudHash || "",
    albumId: record.cloudAlbumId || "",
    strMediaMid: record.cloudStrMediaMid || "",
    url: record.cloudUrl || "",
    coverUrl: record.cloudCoverUrl || "",
    lyrics: record.cloudLyrics || "",
    title: record.title,
    artist: record.artist,
    album: record.album,
    duration: record.duration,
  };
}

// 只有真正要播放时才解析 + 下载；失败则把这条记录和它的缓存文件一起删除。
async function materializeCloudRecordFor(record, token) {
  if (record.filePath) return true;
  if (!record.cloudId) return false;
  if (record.loading) return false;
  const target = reactive(record);
  // 已有相同歌曲 + 相同音质的缓存文件：直接复用，不重复下载
  const reusable = reusableCloudSource(cloudSongFromRecord(target), target.id);
  if (reusable) {
    Object.assign(target, {
      filePath: reusable.filePath,
      originalPath: reusable.originalPath,
      coverPath: reusable.coverPath,
      lyrics: reusable.lyrics,
      lyricsPath: reusable.lyricsPath,
      sourceHash: reusable.sourceHash,
      duration: reusable.duration || target.duration,
      quality: reusable.quality || target.quality,
    });
    return true;
  }
  state.playback.selectedGroupId = target.groupId;
  state.playback.currentSongId = target.id;
  state.playback.currentTime = 0;
  state.playback.isPlaying = false;
  target.loading = true;
  await nextTick();
  try {
    await prepareCloudSong(cloudSongFromRecord(target), target.groupId, {
      targetRecord: target,
      verifyPlayback: true,
      skipExisting: true,
    });
    return true;
  } catch (error) {
    // 已被更新的播放请求取代（例如下载中又点了下一曲）：保留记录，交给新请求处理。
    if (token !== playIntentToken) {
      target.loading = false;
      return false;
    }
    console.error("cloud song failed", error);
    await removeTemporaryCloudSong(target);
    showToast(`云库播放失败，已移除该歌曲：${String(error)}`, 6000);
    return false;
  }
}

async function addCloudSong(cloudSong) {
  if (importing.value || !cloudSong) return;
  // 已经在"试听列表"里就直接播放；在其它分组里不影响，仍加入试听列表。
  const existingInDefault = existingCloudSongInGroup(cloudSong, "default");
  if (existingInDefault) {
    await playSongObject(existingInDefault);
    return;
  }
  ensureDefaultGroup();
  const record = createCloudPlaceholder(cloudSong, "default");
  state.playback.selectedGroupId = "default";
  showToast(`已添加到试听列表：${record.title}`);
  schedulePersist();
  // 添加到列表不下载，播放时才解析下载。
  await playSongObject(record);
}

async function addCloudSongNext(cloudSong) {
  if (!cloudSong?.cloudId) return;
  ensureDefaultGroup();
  const sourceId = String(cloudSong.sourceId || state.settings.activeSourceId || "");
  let record = state.songs.find(
    (song) =>
      song.groupId === "default" &&
      song.cloudId === String(cloudSong.cloudId) &&
      (!song.cloudSourceId || !sourceId || song.cloudSourceId === sourceId),
  );
  if (!record) record = createCloudPlaceholder(cloudSong, "default");
  // 加入"下一首播放"队列：切歌时优先播放队列里的歌，与播放模式无关。
  // 队列按分组维护，避免正在播放其它分组时被这里的歌插队。
  const queue = playNextQueue.value[record.groupId] || (playNextQueue.value[record.groupId] = []);
  if (!queue.includes(record.id)) queue.push(record.id);
  showToast(`已加入下一首播放：${record.title}`);
  schedulePersist();
}

function uniqueGroupName(name) {
  const base = String(name || "云库歌单").trim().slice(0, 36) || "云库歌单";
  const names = new Set(state.groups.map((group) => group.name));
  if (!names.has(base)) return base;
  let index = 2;
  while (names.has(`${base} ${index}`)) index += 1;
  return `${base} ${index}`;
}

async function addCloudPlaylist(playlist) {
  if (importing.value || !playlist?.tracks?.length) return;
  importing.value = true;
  // 同名歌单已存在时，只补齐缺失的歌曲，不再新建分组（避免重复添加）
  const rawName = String(playlist.name || "").trim().slice(0, 36) || "云库歌单";
  const existingGroup = state.groups.find((item) => !item.isDefault && item.name === rawName);
  if (existingGroup) {
    try {
      const existingIds = new Set(
        state.songs.filter((song) => song.groupId === existingGroup.id).map((song) => song.cloudId),
      );
      let appended = 0;
      for (const track of playlist.tracks) {
        const cloudId = String(track.cloudId || "");
        if (!cloudId || existingIds.has(cloudId)) continue;
        createCloudPlaceholder(track, existingGroup.id);
        existingIds.add(cloudId);
        appended += 1;
      }
      state.playback.selectedGroupId = existingGroup.id;
      schedulePersist();
      showToast(
        appended
          ? `已为“${existingGroup.name}”补充 ${appended} 首（播放时再下载）`
          : `歌单“${existingGroup.name}”已在列表中，未重复添加`,
      );
    } finally {
      importing.value = false;
    }
    return;
  }
  const maxOrder = state.groups.reduce((maximum, group) => Math.max(maximum, numberOr(group.order, 0)), 0);
  const group = {
    id: createId("group"),
    name: uniqueGroupName(playlist.name),
    isDefault: false,
    order: maxOrder + 1,
    createdAt: Date.now(),
    collapsed: false,
  };
  state.groups.push(group);
  let added = 0;
  try {
    for (const track of playlist.tracks) {
      const cloudId = String(track.cloudId || "");
      if (!cloudId) continue;
      if (state.songs.some((song) => song.groupId === group.id && song.cloudId === cloudId)) continue;
      createCloudPlaceholder(track, group.id);
      added += 1;
    }
    if (!added) {
      state.groups = state.groups.filter((item) => item.id !== group.id);
      showToast("歌单里没有可添加的歌曲");
      return;
    }
    state.playback.selectedGroupId = group.id;
    const firstSong = state.songs
      .filter((song) => song.groupId === group.id)
      .sort((left, right) => left.order - right.order)[0];
    schedulePersist();
    showToast(`已添加 ${added} 首到“${group.name}”（播放时再下载）`);
    if (firstSong) await playSongObject(firstSong);
  } finally {
    importing.value = false;
  }
}

async function downloadSongToDownload(songId) {
  const song = state.songs.find((item) => item.id === songId);
  if (!song) return;
  if (importing.value) return;
  importing.value = true;
  try {
    const destination = await invoke("copy_song_to_download", {
      filePath: song.filePath,
      downloadDir: state.settings.downloadDir || defaults.downloadDir,
    });
    song.filePath = destination;
    showToast(`已保存到下载目录：${song.title}`);
    schedulePersist();
  } catch (error) {
    showToast(`下载失败：${error}`);
  } finally {
    importing.value = false;
  }
}

async function playSong(songId) {
  const song = state.songs.find((item) => item.id === songId);
  if (!song) return;
  if (state.playback.currentSongId === songId && song.filePath) {
    state.playback.isPlaying = true;
    schedulePersist();
    return;
  }
  await playSongObject(song);
}

function togglePlay() {
  if (!state.playback.currentSongId) {
    const firstSong = queue.value[0];
    if (firstSong) void playSong(firstSong.id);
    return;
  }
  state.playback.isPlaying = !state.playback.isPlaying;
  schedulePersist();
}

function chooseNextsong(direction = 1) {
  const items = queue.value;
  if (!items.length) return null;
  const currentIndex = items.findIndex((song) => song.id === state.playback.currentSongId);
  if (state.playback.playMode === "random" && items.length > 1) {
    const candidates = items.filter((song) => song.id !== state.playback.currentSongId);
    return candidates[Math.floor(Math.random() * candidates.length)];
  }
  if (currentIndex < 0) return direction > 0 ? items[0] : items[items.length - 1];
  const nextIndex = (currentIndex + direction + items.length) % items.length;
  return items[nextIndex];
}

async function playSongObject(song) {
  if (!song) return;
  const token = ++playIntentToken;
  // 云库歌曲在真正播放时才解析并下载；失败会把该记录移除。
  if (!song.filePath && song.cloudId) {
    const ready = await materializeCloudRecordFor(song, token);
    // 期间又切了歌：本次请求作废，不再自动跳歌。
    if (token !== playIntentToken) return;
    if (!ready) {
      await playFallbackAfterFailure(song);
      return;
    }
  }
  if (token !== playIntentToken) return;
  autoSkipDepth = 0;
  state.playback.currentSongId = song.id;
  state.playback.selectedGroupId = song.groupId;
  state.playback.currentTime = 0;
  state.playback.isPlaying = true;
  state.settings.lastPlayedSongId = song.id;
  schedulePersist();
  recordPlayed(song.id);
}

function recordPlayed(songId) {
  const song = state.songs.find((item) => item.id === songId);
  const groupId = song?.groupId || "";
  // "播放过"列表只对当前分组有效；换了分组就清空，避免上一曲跳到别的分组。
  if (groupId && playHistoryGroup && playHistoryGroup !== groupId) {
    playHistory.value = [];
    playHistoryIndex = -1;
  }
  if (groupId) playHistoryGroup = groupId;
  // 已经在当前位置（例如从历史回退播放）时不改动历史。
  if (playHistoryIndex >= 0 && playHistory.value[playHistoryIndex] === songId) return;
  playHistory.value.splice(playHistoryIndex + 1);
  const existing = playHistory.value.indexOf(songId);
  if (existing >= 0) playHistory.value.splice(existing, 1);
  playHistory.value.push(songId);
  if (playHistory.value.length > 20) playHistory.value.shift();
  playHistoryIndex = playHistory.value.length - 1;
}

// 消费"当前分组"的下一首播放队列；成功播放返回 true
async function playQueuedNext() {
  const groupId = currentGroupId();
  const queue = playNextQueue.value[groupId];
  while (queue && queue.length) {
    const songId = queue.shift();
    if (songId === state.playback.currentSongId) continue;
    const song = state.songs.find((item) => item.id === songId);
    if (!song) continue;
    await playSongObject(song);
    return true;
  }
  return false;
}

function currentGroupId() {
  return (
    state.songs.find((song) => song.id === state.playback.currentSongId)?.groupId ||
    state.playback.selectedGroupId
  );
}

async function restartCurrentSong() {
  state.playback.currentTime = 0;
  await nextTick();
  if (!state.playback.isPlaying) state.playback.isPlaying = true;
  schedulePersist();
}

async function nextSong() {
  if (await playQueuedNext()) return;
  const song = chooseNextsong(1);
  if (song?.id === state.playback.currentSongId && song.filePath) await restartCurrentSong();
  else await playSongObject(song);
}

function pausePlayback() {
  state.playback.isPlaying = false;
  state.playback.currentTime = 0;
  schedulePersist();
}

// 播放失败后：当前分组下一首 → 试听列表 → 暂停
async function playFallbackAfterFailure(failed) {
  if (autoSkipDepth >= 5) {
    autoSkipDepth = 0;
    pausePlayback();
    return;
  }
  autoSkipDepth += 1;
  const orderOf = (song) => numberOr(song.order, 0);
  const sameGroup = state.songs
    .filter((song) => song.groupId === failed?.groupId)
    .sort((left, right) => orderOf(left) - orderOf(right));
  const nextInGroup =
    sameGroup.find((song) => orderOf(song) > orderOf(failed)) || sameGroup[0] || null;
  if (nextInGroup) {
    await playSongObject(nextInGroup);
    return;
  }
  if (failed?.groupId !== "default") {
    const defaultSong = state.songs
      .filter((song) => song.groupId === "default")
      .sort((left, right) => orderOf(left) - orderOf(right))[0];
    if (defaultSong) {
      await playSongObject(defaultSong);
      return;
    }
  }
  autoSkipDepth = 0;
  pausePlayback();
}

async function previousSong() {
  // 优先播放"播放过"列表里的上一首
  const groupId = currentGroupId();
  for (let index = playHistoryIndex - 1; index >= 0; index -= 1) {
    const song = state.songs.find((item) => item.id === playHistory.value[index]);
    // 只认当前分组的历史，防止跳到其它分组
    if (!song || song.groupId !== groupId) continue;
    playHistoryIndex = index;
    await playSongObject(song);
    return;
  }
  const song = chooseNextsong(-1);
  if (song?.id === state.playback.currentSongId && song.filePath) await restartCurrentSong();
  else await playSongObject(song);
}

async function handleEnded() {
  // 无论播放模式如何，队列里的"下一首"优先
  if (await playQueuedNext()) return;
  if (state.playback.playMode === "repeat") {
    await restartCurrentSong();
    return;
  }
  await nextSong();
}

function handleSeek(value) {
  state.playback.currentTime = Math.max(0, Number(value) || 0);
  schedulePersist();
}

function handleDuration(duration) {
  if (!currentSong.value || !duration) return;
  const rounded = Math.round(duration);
  if (Math.abs((currentSong.value.duration || 0) - rounded) > 500) currentSong.value.duration = rounded;
}

function handleMediaReady() {
  finishMediaVerification(true);
}

function handlePlaying() {
  state.playback.isPlaying = true;
  finishMediaVerification(true);
}

function handlePaused() {
  state.playback.isPlaying = false;
  schedulePersist();
}

function handleMediaError() {
  finishMediaVerification(false);
}

function setVolume(value) {
  state.settings.volume = Math.min(1, Math.max(0, Number(value) || 0));
  if (state.settings.volume > 0) state.settings.muted = false;
  schedulePersist();
}

function toggleMute() {
  state.settings.muted = !state.settings.muted;
  schedulePersist();
}

function shortcutHandlers() {
  return {
    prev: () => previousSong(),
    next: () => nextSong(),
    playPause: () => togglePlay(),
    volumeUp: () => setVolume(state.settings.volume + 0.05),
    volumeDown: () => setVolume(state.settings.volume - 0.05),
    mute: () => toggleMute(),
  };
}

function onAppKeydown(event) {
  if (event.repeat || settingsOpen.value) return;
  const target = event.target;
  if (target instanceof HTMLElement) {
    const tag = target.tagName;
    if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable) return;
  }
  const actions = shortcutHandlers();
  for (const [action, handler] of Object.entries(actions)) {
    const accelerator = state.settings.shortcuts?.[action];
    if (accelerator && acceleratorMatches(event, accelerator)) {
      event.preventDefault();
      handler();
      return;
    }
  }
}

async function applyGlobalShortcuts() {
  try {
    await unregisterAllShortcuts();
  } catch {
    // 全局快捷键插件不可用时（例如浏览器调试）直接跳过。
    return;
  }
  if (!state.settings.globalShortcutsEnabled) return;
  const actions = shortcutHandlers();
  const failures = [];
  for (const [action, handler] of Object.entries(actions)) {
    const accelerator = String(state.settings.globalShortcuts?.[action] || "").trim();
    if (!accelerator) continue;
    try {
      await registerShortcut(accelerator, (event) => {
        if (!event || event.state === "Pressed") handler();
      });
    } catch {
      failures.push(accelerator);
    }
  }
  if (failures.length) showToast(`全局快捷键注册失败：${failures.join("、")}`);
}

function setPlayMode(mode) {
  if (!ALLOWED_PLAY_MODES.includes(mode)) return;
  state.playback.playMode = mode;
  state.settings.playMode = mode;
  showToast(
    mode === "sequence" ? "顺序播放" : mode === "random" ? "随机播放" : "单曲循环",
    1500,
  );
  schedulePersist();
}

async function saveSettings(draft) {
  if (!draft.downloadDir || !draft.cacheDir) {
    showToast("下载目录和缓存目录不能为空");
    return;
  }
  try {
    const paths = await invoke("set_app_paths", {
      downloadDir: draft.downloadDir,
      cacheDir: draft.cacheDir,
    });
    state.settings.downloadDir = paths.download_dir;
    state.settings.cacheDir = paths.cache_dir;
    if (draft.activeSourceId) state.settings.activeSourceId = draft.activeSourceId;
    if (draft.activeSourceName) state.settings.activeSourceName = draft.activeSourceName;
    state.settings.quality = ["128k", "320k", "flac", "flac24bit"].includes(draft.quality) ? draft.quality : "320k";
    state.settings.theme = themeById(draft.theme).id;
    state.settings.globalShortcutsEnabled = Boolean(draft.globalShortcutsEnabled);
    state.settings.shortcuts = normalizeShortcuts(draft.shortcuts, DEFAULT_SHORTCUTS);
    state.settings.globalShortcuts = normalizeShortcuts(draft.globalShortcuts, emptyGlobalShortcuts());
    settingsOpen.value = false;
    showToast("设置已保存");
    schedulePersist();
    void syncWindowWidth();
    void applyGlobalShortcuts();
  } catch (error) {
    showToast(`保存设置失败：${error}`);
  }
}

function selectCloudSource(source) {
  if (!source?.id) return;
  state.settings.activeSourceId = source.id;
  state.settings.activeSourceName = source.name || source.id;
  schedulePersist();
  void refreshCloudPlatforms();
}

function playExistingCloudSong(cloudSong) {
  const sourceId = cloudSong?.sourceId || state.settings.activeSourceId;
  const record = state.songs.find(
    (song) =>
      song.groupId === "default" &&
      song.cloudId === String(cloudSong?.cloudId || "") &&
      (!song.cloudSourceId || !sourceId || song.cloudSourceId === sourceId),
  );
  if (record) void playSong(record.id);
}

async function handleSourcesChanged(sources) {
  musicSources.value = Array.isArray(sources) ? sources : [];
  if (!musicSources.value.some((source) => source.id === state.settings.activeSourceId)) {
    state.settings.activeSourceId = musicSources.value[0]?.id || "";
    state.settings.activeSourceName = musicSources.value[0]?.name || "暂无音源";
  }
  await refreshMusicSources();
  schedulePersist();
}

function readWindowWidth() {
  return Math.max(0, Math.round(window.innerWidth) || 0);
}

function fitPlayerWidthToWindow(value) {
  return Math.min(windowWidth.value, Math.max(0, Math.round(value) || 0));
}

function updateWindowWidths(maximized) {
  windowWidth.value = readWindowWidth();
  if (cloudOpen.value) {
    const desiredWidth = savedPlayerWidth || playerWidth.value || windowWidth.value;
    playerWidth.value = fitPlayerWidthToWindow(desiredWidth);
    return;
  }
  if (!maximized) {
    savedPlayerWidth = windowWidth.value;
    playerWidth.value = windowWidth.value;
  }
}

function applyMaximizedWindowState() {
  windowWasMaximized = true;
  if (cloudOpen.value) {
    const desiredWidth = savedPlayerWidth || playerWidth.value || windowWidth.value || DEFAULT_WINDOW_WIDTH;
    playerWidth.value = fitPlayerWidthToWindow(desiredWidth);
    savedPlayerWidth = playerWidth.value;
    restoreWindowWidth = playerWidth.value + PAGE_WIDTH;
  } else if (playerWidth.value) {
    savedPlayerWidth = playerWidth.value;
    restoreWindowWidth = savedPlayerWidth;
    playerWidth.value = 0;
  } else {
    playerWidth.value = 0;
  }
}

async function trackWindowWidth() {
  const currentWindow = getCurrentWindow();
  try {
    const maximized = await currentWindow.isMaximized();
    updateWindowWidths(maximized);
    if (maximized) {
      applyMaximizedWindowState();
      return;
    }

    if (windowWasMaximized && restoreWindowWidth) {
      const targetWidth = Math.max(0, Math.round(restoreWindowWidth));
      restoreWindowWidth = 0;
      windowWasMaximized = false;
      windowWidth.value = targetWidth;
      if (cloudOpen.value) {
        const desiredWidth = savedPlayerWidth || playerWidth.value || DEFAULT_WINDOW_WIDTH;
        playerWidth.value = Math.min(targetWidth, Math.max(0, desiredWidth));
        if (Math.round(window.innerWidth) !== targetWidth) {
          await currentWindow.setSize(new LogicalSize(targetWidth, window.innerHeight));
        }
      } else {
        if (Math.round(window.innerWidth) !== targetWidth) {
          await currentWindow.setSize(new LogicalSize(targetWidth, window.innerHeight));
        }
        savedPlayerWidth = playerWidth.value = targetWidth;
      }
      return;
    }

    windowWasMaximized = false;
  } catch {
    // 窗口状态不可用时保留当前宽度。
  }
}

function startPlayerWidthResize(event) {
  if (
    !cloudOpen.value ||
    !playerWidth.value ||
    windowWidth.value < MIN_PLAYER_WIDTH ||
    event.button !== 0
  ) return;
  event.preventDefault();
  resizeStartX = event.clientX;
  resizeStartWidth = playerWidth.value;
  resizingPlayerWidth.value = true;
  event.currentTarget.setPointerCapture?.(event.pointerId);
  window.addEventListener("pointermove", movePlayerWidthResize);
  window.addEventListener("pointerup", stopPlayerWidthResize);
  window.addEventListener("pointercancel", stopPlayerWidthResize);
}

function movePlayerWidthResize(event) {
  if (windowWidth.value < MIN_PLAYER_WIDTH) return;
  const requestedWidth = resizeStartWidth + event.clientX - resizeStartX;
  const nextWidth = Math.min(windowWidth.value, Math.max(MIN_PLAYER_WIDTH, requestedWidth));
  playerWidth.value = nextWidth;
  savedPlayerWidth = nextWidth;
  if (windowWasMaximized) restoreWindowWidth = nextWidth + PAGE_WIDTH;
}

function stopPlayerWidthResize() {
  window.removeEventListener("pointermove", movePlayerWidthResize);
  window.removeEventListener("pointerup", stopPlayerWidthResize);
  window.removeEventListener("pointercancel", stopPlayerWidthResize);
  resizingPlayerWidth.value = false;
}

async function syncWindowWidth() {
  const currentWindow = getCurrentWindow();
  try {
    if (await currentWindow.isMaximized()) {
      updateWindowWidths(true);
      applyMaximizedWindowState();
      return;
    }
    if (!restoreWindowWidth) windowWasMaximized = false;
    if (cloudOpen.value) {
      const target = Math.max(0, playerWidth.value + PAGE_WIDTH);
      await currentWindow.setSize(new LogicalSize(target, window.innerHeight));
      windowWidth.value = target;
      return;
    }
    const restore = playerWidth.value || savedPlayerWidth || readWindowWidth();
    if (restore) {
      await currentWindow.setSize(new LogicalSize(restore, window.innerHeight));
    }
    windowWidth.value = restore;
    savedPlayerWidth = playerWidth.value = restore;
  } catch {
    // 窗口 API 拒绝改尺寸时保持当前大小
  }
}

async function toggleCloud() {
  if (!cloudOpen.value) {
    const maximized = await getCurrentWindow().isMaximized().catch(() => false);
    updateWindowWidths(maximized);
    const desiredWidth = maximized
      ? savedPlayerWidth || DEFAULT_WINDOW_WIDTH
      : windowWidth.value;
    playerWidth.value = maximized
      ? fitPlayerWidthToWindow(desiredWidth)
      : desiredWidth;
    savedPlayerWidth = playerWidth.value;
    cloudOpen.value = true;
    settingsOpen.value = false;
  } else {
    cloudOpen.value = false;
  }
  await nextTick();
  await syncWindowWidth();
}

// 设置页覆盖整个窗口（叠加在上面，不影响云库/窗口尺寸）
function openSettings() {
  settingsOpen.value = true;
}

function closeSettings() {
  settingsOpen.value = false;
}

async function handleWindowClose() {
  await getCurrentWindow().hide();
}

async function toggleCollapsed() {
  if (cloudOpen.value) {
    await toggleCloud();
    await nextTick();
  }
  const currentWindow = getCurrentWindow();
  if (!collapsed.value) {
    const titlebarHeight = document.querySelector(".titlebar")?.getBoundingClientRect().height || 46;
    const playerHeight = document.querySelector(".player-panel")?.getBoundingClientRect().height || 0;
    expandedWindowHeight.value = window.innerHeight;
    const targetHeight = Math.max(1, Math.round(titlebarHeight + playerHeight));
    collapsed.value = true;
    await nextTick();
    try {
      if (!(await currentWindow.isMaximized())) {
        suppressResizeUncollapseUntil = Date.now() + 250;
        await currentWindow.setMinSize(new LogicalSize(MIN_WINDOW_WIDTH, targetHeight));
        await currentWindow.setSize(new LogicalSize(window.innerWidth, targetHeight));
      }
    } catch {
      // Keep the current bounds if the window API rejects a dynamic resize.
    }
    return;
  }

  collapsed.value = false;
  await nextTick();
  try {
    await currentWindow.setMinSize(new LogicalSize(MIN_WINDOW_WIDTH, 520));
    if (expandedWindowHeight.value && !(await currentWindow.isMaximized())) {
      await currentWindow.setSize(new LogicalSize(window.innerWidth, expandedWindowHeight.value));
    }
  } catch {
    // Keep the current bounds if the window API rejects restoring the size.
  }
}

function onWindowResize() {
  windowWidth.value = readWindowWidth();
  if (cloudOpen.value) {
    playerWidth.value = Math.min(playerWidth.value, windowWidth.value);
  }
  void trackWindowWidth();
  const heightChanged = window.innerHeight !== lastWindowHeight;
  lastWindowHeight = window.innerHeight;
  if (Date.now() < suppressResizeUncollapseUntil || !heightChanged || !collapsed.value) return;
  collapsed.value = false;
  void getCurrentWindow().setMinSize(new LogicalSize(MIN_WINDOW_WIDTH, 520));
}

function handleBeforeUnload() {
  // 退出时删除窗口大小/位置记录，下次启动是全新的默认窗口。
  state.settings.windowBounds = null;
  persistNow();
}

onMounted(() => {
  void initialize();
  lastWindowHeight = window.innerHeight;
  void trackWindowWidth();
  window.addEventListener("resize", onWindowResize);
  window.addEventListener("beforeunload", handleBeforeUnload);
  window.addEventListener("keydown", onAppKeydown);
});

onBeforeUnmount(() => {
  stopPlayerWidthResize();
  window.removeEventListener("resize", onWindowResize);
  window.removeEventListener("beforeunload", handleBeforeUnload);
  window.removeEventListener("keydown", onAppKeydown);
  window.clearTimeout(persistTimer);
  window.clearTimeout(toastTimer);
  finishMediaVerification(false);
});

watch(
  state,
  () => {
    schedulePersist();
  },
  { deep: true },
);
</script>

<template>
  <div class="window-shell" :style="themeVars">
    <div class="app-shell" :class="{ 'playlist-is-collapsed': collapsed }" :style="appShellStyle">
      <TitleBar @collapse="toggleCollapsed" @settings="openSettings" @close="handleWindowClose" />
      <main class="app-main">
      <div class="main-column">
      <PlayerPanel
        :state="state"
        :current-song="currentSong"
        :media-ready="mediaReady"
        @toggle="togglePlay"
        @previous="previousSong"
        @next="nextSong"
        @seek="handleSeek"
        @timeupdate="handleSeek"
        @duration="handleDuration"
        @canplay="handleMediaReady"
        @playing="handlePlaying"
        @paused="handlePaused"
        @media-error="handleMediaError"
        @ended="handleEnded"
        @volume="setVolume"
        @mute="toggleMute"
        @mode="setPlayMode"
      />

      <div class="playlist-region" :class="{ 'playlist-collapsed': collapsed }">
        <PlaylistManager
          v-if="!collapsed"
          :state="state"
          :groups="orderedGroups"
          :songs="state.songs"
          :current-song-id="state.playback.currentSongId || ''"
          :selected-group-id="state.playback.selectedGroupId"
          @select-group="selectGroup"
          @toggle-group="toggleGroup"
          @play="playSong"
          @create-group="createGroup"
          @rename-group="renameGroup"
          @delete-group="deleteGroup"
          @clear-group="requestClearDefault"
          @move-song="moveSong"
          @copy-song="copySong"
          @remove-song="removeSong"
          @reveal="revealSong"
          @download-song="downloadSongToDownload"
          @import="importSongs"
          @cloud="toggleCloud"
        />
        <div v-else class="collapsed-placeholder">
          <span>列表已折叠</span>
          <button type="button" @click="toggleCollapsed">展开</button>
        </div>
      </div>
      </div>
      </main>
      <div
        v-if="cloudOpen"
        class="page-resizer"
        :class="{ active: resizingPlayerWidth }"
        role="separator"
        aria-orientation="vertical"
        aria-label="调整播放页宽度"
        @pointerdown.prevent="startPlayerWidthResize"
      />
    </div>

    <CloudPanel
      v-show="cloudOpen"
      :style="cloudPanelStyle"
      :sources="musicSources"
      :active-source="{ id: state.settings.activeSourceId, name: state.settings.activeSourceName }"
      :platforms="cloudPlatforms"
      :preferred-quality="state.settings.quality || '320k'"
      :playlist-names="playlistNames"
      :default-cloud-keys="defaultCloudKeys"
      @close="toggleCloud"
      @add="addCloudSong"
      @play-next="addCloudSongNext"
      @add-playlist="addCloudPlaylist"
      @play-existing="playExistingCloudSong"
      @select-source="selectCloudSource"
      @settings="openSettings"
      @toast="showToast"
    />

    <SettingsModal
      v-if="settingsOpen"
      :settings="state.settings"
      :defaults="defaults"
      :cache-keep-paths="cacheKeepPaths"
      @close="closeSettings"
      @save="saveSettings"
      @sources-changed="handleSourcesChanged"
    />

    <div v-if="deleteGroupTarget" class="confirm-overlay" @click.self="cancelDeleteGroup">
      <section class="confirm-dialog" role="dialog" aria-modal="true">
        <h3>删除分组</h3>
        <p>该分组中有歌曲。要将它们移动到“试听列表”吗？</p>
        <div class="confirm-actions">
          <button type="button" @click="confirmDeleteGroup(true)">移动到试听列表</button>
          <button type="button" @click="confirmDeleteGroup(false)">不移动并删除</button>
          <button type="button" @click="cancelDeleteGroup">取消</button>
        </div>
      </section>
    </div>
    <div v-if="clearDefaultTarget" class="confirm-overlay" @click.self="cancelClearDefault">
      <section class="confirm-dialog" role="dialog" aria-modal="true">
        <h3>清空试听列表</h3>
        <p>确定清空试听列表吗？缓存目录中的相关文件会一并删除，下载目录文件不会删除。</p>
        <div class="confirm-actions">
          <button type="button" @click="confirmClearDefault">确认清空</button>
          <button type="button" @click="cancelClearDefault">取消</button>
        </div>
      </section>
    </div>
    <div v-if="toast" class="toast" role="status">{{ toast }}</div>
    <div v-if="importing" class="importing-mask">正在导入歌曲…</div>
  </div>
</template>

<style scoped>
.window-shell {
  height: 100vh;
  min-height: 520px;
  display: flex;
  flex-direction: row;
  overflow: hidden;
  background: var(--canvas);
}

.app-shell {
  position: relative;
  min-width: 0;
  min-height: 0;
  flex: 1 1 auto;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--canvas);
}

.page-resizer {
  position: absolute;
  z-index: 20;
  top: 0;
  right: 0;
  bottom: 0;
  width: 6px;
  cursor: col-resize;
  touch-action: none;
  user-select: none;
  background: var(--line);
  opacity: 0.72;
}

.page-resizer:hover,
.page-resizer.active {
  background: var(--accent);
  opacity: 1;
}

.app-shell.playlist-is-collapsed {
  min-height: 0;
}

.app-main {
  min-height: 0;
  flex: 1;
  display: flex;
  flex-direction: column;
}

.main-column {
  min-width: 0;
  min-height: 0;
  flex: 1;
  display: flex;
  flex-direction: column;
}

.playlist-region {
  min-height: 0;
  flex: 1 1 auto;
  display: flex;
}

.playlist-region.playlist-collapsed {
  flex: 0 0 76px;
}

.playlist-region > * {
  min-width: 0;
  flex: 1;
}

.collapsed-placeholder {
  min-height: 90px;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 10px;
  color: var(--muted);
  font-size: 13px;
}

.collapsed-placeholder button {
  border: 1px solid var(--line);
  border-radius: 5px;
  padding: 5px 8px;
  background: var(--surface);
  color: var(--accent);
  cursor: pointer;
  font-size: 12px;
}

.toast,
.importing-mask {
  position: fixed;
  z-index: 40;
  left: 50%;
  transform: translateX(-50%);
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--surface);
  color: var(--text);
  box-shadow: var(--shadow);
  font-size: 13px;
}

.toast {
  bottom: 16px;
  max-width: calc(100vw - 30px);
  padding: 8px 11px;
}

.importing-mask {
  top: 58px;
  padding: 7px 12px;
  color: var(--accent);
}

.confirm-overlay {
  position: fixed;
  inset: 0;
  z-index: 35;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgb(10 14 22 / 38%);
}

.confirm-dialog {
  width: min(330px, calc(100vw - 30px));
  padding: 16px;
  border: 1px solid var(--line);
  border-radius: 9px;
  background: var(--surface);
  box-shadow: var(--shadow);
}

.confirm-dialog h3 {
  margin: 0;
  color: var(--text);
  font-size: 16px;
}

.confirm-dialog p {
  margin: 9px 0 14px;
  color: var(--muted);
  font-size: 13px;
  line-height: 1.6;
}

.confirm-actions {
  display: flex;
  flex-wrap: wrap;
  justify-content: flex-end;
  gap: 6px;
}

.confirm-actions button {
  padding: 6px 8px;
  border: 1px solid var(--line);
  border-radius: 5px;
  background: var(--surface-2);
  color: var(--muted);
  cursor: pointer;
  font-size: 12px;
}

.confirm-actions button:first-child {
  border-color: var(--accent);
  color: var(--accent);
}
</style>

<style>
:root {
  color-scheme: light dark;
  font-family:
    Inter, "Segoe UI", "Microsoft YaHei", system-ui, -apple-system, sans-serif;
  font-size: 16px;
  line-height: 1.45;
  color: var(--text);
  background: var(--canvas);
  font-synthesis: none;
  text-rendering: optimizeLegibility;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  --canvas: #f4f6fa;
  --surface: #ffffff;
  --surface-2: #f8f9fc;
  --line: #e3e7ef;
  --text: #1d2433;
  --muted: #667085;
  --faint: #98a2b3;
  --hover: #eef2f8;
  --track: #e8ebf2;
  --accent: #5b7cff;
  --accent-2: #795eff;
  --shadow: 0 14px 38px rgba(27, 36, 61, 0.16);
  --shadow-sm: 0 3px 10px rgba(27, 36, 61, 0.06);
}

* {
  box-sizing: border-box;
}

html,
body,
#app {
  width: 100%;
  height: 100%;
  margin: 0;
}

body {
  overflow: hidden;
}

button,
input {
  font: inherit;
}

button:focus-visible,
input:focus-visible {
  outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent);
  outline-offset: 1px;
}

/* 主题：所有内置图标都是单色 (#d4237a)，用滤镜统一换色 */
img.titlebar-icon,
img.brand-logo,
img.button-icon,
img.play-icon,
img.mode-icon,
img.toolbar-icon,
img.group-chevron,
img.song-playing,
.song-action img,
.cloud-row-action img,
.cloud-close img,
.back-button img {
  filter: var(--icon-filter, none);
}

@media (prefers-color-scheme: dark) {
  :root {
    --canvas: #141821;
    --surface: #1b202c;
    --surface-2: #202634;
    --line: #303747;
    --text: #eef1f7;
    --muted: #aab3c4;
    --faint: #737d91;
    --hover: #293143;
    --track: #303747;
    --shadow: 0 16px 42px rgba(0, 0, 0, 0.34);
    --shadow-sm: 0 3px 10px rgba(0, 0, 0, 0.2);
  }
}

::-webkit-scrollbar {
  width: 8px;
  height: 8px;
}

::-webkit-scrollbar-thumb {
  border: 2px solid transparent;
  border-radius: 99px;
  background: var(--track);
  background-clip: padding-box;
}

::-webkit-scrollbar-track {
  background: transparent;
}
</style>
