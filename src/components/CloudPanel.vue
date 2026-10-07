<script setup>
import { computed, nextTick, ref, watch } from "vue";
import { invoke } from "@tauri-apps/api/core";
import closeIcon from "../assets/icons/guanbi1.svg";
import playIcon from "../assets/icons/bofang1.svg";
import nextIcon from "../assets/icons/yinpin.svg";

const props = defineProps({
  activeSource: { type: Object, default: () => ({ id: "", name: "暂无音源" }) },
  sources: { type: Array, default: () => [] },
  platforms: { type: Array, default: () => [] },
  preferredQuality: { type: String, default: "320k" },
  playlistNames: { type: Array, default: () => [] },
  defaultCloudKeys: { type: Array, default: () => [] },
});
const emit = defineEmits([
  "close",
  "add",
  "add-playlist",
  "play-next",
  "play-existing",
  "select-source",
  "settings",
  "toast",
]);
const keyword = ref("");
const tab = ref("songs");
const platform = ref("netease");
const loading = ref(false);
const message = ref("");
const songs = ref([]);
const playlists = ref([]);
const playlist = ref(null);
const searchCache = new Map();
const pendingSongClicks = new Map();
const PAGE_SIZE = 50;
const currentPage = ref(0);
const hasMore = ref(false);
const loadingMore = ref(false);
const playlistPage = ref(0);
const playlistHasMore = ref(false);
let sourceOverrideId = "";
let searchSequence = 0;

const FALLBACK_PLATFORMS = [
  { id: "netease", name: "网易" },
  { id: "tencent", name: "企鹅" },
  { id: "kugou", name: "酷狗" },
  { id: "kuwo", name: "酷我" },
];

const platformOptions = computed(() =>
  props.platforms?.length ? props.platforms : FALLBACK_PLATFORMS,
);

watch(
  platformOptions,
  (options) => {
    if (options.some((item) => item.id === platform.value)) return;
    const preferred = options.find((item) => item.id === "netease");
    platform.value = preferred?.id || options[0]?.id || "netease";
  },
  { immediate: true },
);

function qualityOptions(song) {
  const list = Array.isArray(song?.qualities) ? song.qualities.filter(Boolean) : [];
  return list.length ? list : ["320k", "128k"];
}

function qualityLabel(quality) {
  if (quality === "flac24bit") return "FLAC 24bit";
  if (quality === "flac") return "FLAC";
  return quality;
}

function effectiveQuality(song) {
  const options = qualityOptions(song);
  if (song?._quality && options.includes(song._quality)) return song._quality;
  if (options.includes(props.preferredQuality)) return props.preferredQuality;
  return options[0];
}

function chooseQuality(song, quality) {
  song._quality = quality;
}

function isPlaylistAdded(name) {
  const target = String(name || "").trim().slice(0, 36);
  if (!target) return false;
  return props.playlistNames.includes(target);
}

function normalizeSong(item) {
  const artists = item.artists || item.ar || item.artist || [];
  const artist = Array.isArray(artists)
    ? artists.map((entry) => entry?.name || entry?.artistName || entry).filter(Boolean).join(" / ")
    : String(artists || "未知歌手");
  const album = item.album?.name || item.al?.name || item.albumName || "未知专辑";
  const id = item.id ?? item.songmid ?? item.songId ?? item.hash ?? "";
  return {
    cloudId: String(id),
    title: item.name || item.title || item.songname || "未知歌曲",
    artist: artist || "未知歌手",
    album,
    duration: Math.max(0, Number(item.duration || item.dt || item.interval * 1000 || 0)),
    url: item.url || item.playUrl || "",
    coverUrl: item.picUrl || item.coverUrl || item.album?.picUrl || item.al?.picUrl || item.cover || "",
    lyrics: item.lyrics || item.lrc || "",
    hash: item.hash || "",
    albumId: item.albumId || item.album_id || "",
    strMediaMid: item.strMediaMid || item.mediaMid || "",
    qualities: Array.isArray(item.qualities) ? item.qualities.map(String) : [],
    sourceId: item.sourceId || props.activeSource?.id || "",
    platform: item.platform || "netease",
  };
}

function normalizePlaylist(item) {
  return {
    id: String(item.id ?? item.dissid ?? item.playlistId ?? ""),
    name: item.name || item.title || item.dissname || "未命名歌单",
    trackCount: Number(item.trackCount || item.size || item.total || 0),
    creator: item.creator?.nickname || item.creator?.name || item.nick || "未知用户",
    coverUrl: item.coverImgUrl || item.picUrl || item.coverUrl || item.cover || "",
    description: item.description || item.intro || item.copywriter || "",
    platform: item.platform || "netease",
  };
}

function sourceKey() {
  return sourceOverrideId || props.activeSource?.id || "";
}

function cacheKey(query = keyword.value.trim()) {
  return `${sourceKey()}|${platform.value}|${tab.value}|${query}`;
}

function takeSnapshot() {
  return JSON.parse(
    JSON.stringify({
      songs: songs.value,
      playlists: playlists.value,
      playlist: playlist.value,
      message: message.value,
      page: currentPage.value,
      hasMore: hasMore.value,
      playlistPage: playlistPage.value,
      playlistHasMore: playlistHasMore.value,
    }),
  );
}

function saveSnapshot(key = cacheKey()) {
  if (!keyword.value.trim()) return;
  searchCache.set(key, takeSnapshot());
}

function restoreSnapshot(snapshot) {
  searchSequence += 1;
  songs.value = snapshot.songs || [];
  playlists.value = snapshot.playlists || [];
  playlist.value = snapshot.playlist || null;
  message.value = snapshot.message || "";
  currentPage.value = Number(snapshot.page) || 0;
  hasMore.value = Boolean(snapshot.hasMore);
  playlistPage.value = Number(snapshot.playlistPage) || 0;
  playlistHasMore.value = Boolean(snapshot.playlistHasMore);
  loading.value = false;
}

function searchItems(payload, isPlaylist) {
  if (isPlaylist) return payload.result?.playlists || payload.playlists || [];
  return payload.result?.songs || payload.songs || [];
}

function searchTotal(payload, isPlaylist) {
  const result = payload.result || {};
  const value = isPlaylist
    ? result.playlistCount ?? result.playListCount ?? result.total
    : result.songCount ?? result.total;
  const total = Number(value);
  return Number.isFinite(total) ? total : 0;
}

function playlistTracks(payload) {
  return (
    payload.playlist?.tracks ||
    payload.result?.playlist?.tracks ||
    payload.result?.tracks ||
    payload.tracks ||
    []
  );
}

function playlistDetail(payload) {
  return payload.playlist || payload.result?.playlist || payload.result || payload;
}

function mergeUnique(items, existing, getId) {
  const seen = new Set(existing.map(getId));
  const merged = [...existing];
  items.forEach((item) => {
    const id = getId(item);
    if (!id || seen.has(id)) return;
    seen.add(id);
    merged.push(item);
  });
  return merged;
}

function updateHasMore(payload, items, page, isPlaylist) {
  const total = searchTotal(payload, isPlaylist);
  const loaded = (page + 1) * PAGE_SIZE;
  hasMore.value =
    items.length > 0 &&
    (total ? loaded < total : items.length === PAGE_SIZE);
}

async function search() {
  const query = keyword.value.trim();
  if (!query) {
    message.value = "请输入歌曲或歌单名称";
    return;
  }
  const key = cacheKey(query);
  const cached = searchCache.get(key);
  if (cached) {
    restoreSnapshot(cached);
    return;
  }
  const sequence = ++searchSequence;
  loading.value = true;
  message.value = "";
  playlist.value = null;
  try {
    const raw = await invoke("search_music", {
      keyword: query,
      sourceId: sourceKey(),
      platform: platform.value,
      kind: tab.value === "playlists" ? 1000 : 1,
      page: 0,
      limit: PAGE_SIZE,
    });
    const payload = JSON.parse(raw || "{}");
    if (sequence !== searchSequence) return;
    const isPlaylist = tab.value === "playlists";
    const items = searchItems(payload, isPlaylist);
    currentPage.value = 0;
    if (isPlaylist) {
      playlists.value = items.map(normalizePlaylist);
      songs.value = [];
      message.value = playlists.value.length ? "" : "没有找到歌单";
    } else {
      songs.value = items.map(normalizeSong);
      playlists.value = [];
      message.value = songs.value.length ? "" : "没有找到歌曲";
    }
    updateHasMore(payload, items, 0, isPlaylist);
    saveSnapshot(key);
  } catch (error) {
    if (sequence === searchSequence) message.value = `搜索失败：${error}`;
  } finally {
    if (sequence === searchSequence) loading.value = false;
  }
}

async function loadMore() {
  if (playlist.value) {
    await loadMorePlaylist();
    return;
  }
  const query = keyword.value.trim();
  if (!query || playlist.value || loading.value || loadingMore.value || !hasMore.value) return;
  const key = cacheKey(query);
  const sequence = ++searchSequence;
  loadingMore.value = true;
  try {
    const raw = await invoke("search_music", {
      keyword: query,
      sourceId: sourceKey(),
      platform: platform.value,
      kind: tab.value === "playlists" ? 1000 : 1,
      page: currentPage.value + 1,
      limit: PAGE_SIZE,
    });
    const payload = JSON.parse(raw || "{}");
    if (sequence !== searchSequence) return;
    const isPlaylist = tab.value === "playlists";
    const items = searchItems(payload, isPlaylist);
    currentPage.value += 1;
    if (isPlaylist) {
      playlists.value = mergeUnique(items.map(normalizePlaylist), playlists.value, (item) => item.id);
    } else {
      songs.value = mergeUnique(items.map(normalizeSong), songs.value, (item) => item.cloudId);
    }
    updateHasMore(payload, items, currentPage.value, isPlaylist);
    saveSnapshot(key);
  } catch (error) {
    if (sequence === searchSequence) message.value = `加载更多失败：${error}`;
  } finally {
    loadingMore.value = false;
  }
}

async function loadMorePlaylist() {
  if (!playlist.value || loading.value || loadingMore.value || !playlistHasMore.value) return;
  const key = cacheKey();
  const sequence = ++searchSequence;
  loadingMore.value = true;
  try {
    const raw = await invoke("get_playlist", {
      playlistId: playlist.value.id,
      sourceId: sourceKey(),
      offset: (playlistPage.value + 1) * PAGE_SIZE,
      limit: PAGE_SIZE,
    });
    const payload = JSON.parse(raw || "{}");
    if (sequence !== searchSequence) return;
    const detail = playlistDetail(payload);
    const tracks = playlistTracks(payload).map(normalizeSong);
    const seen = new Set(playlist.value.tracks.map((song) => song.cloudId));
    const appended = tracks.filter((song) => !seen.has(song.cloudId));
    playlistPage.value += 1;
    playlist.value = {
      ...playlist.value,
      description: playlist.value.description || detail.description || "",
      trackCount: Number(detail.trackCount || playlist.value.trackCount || 0),
      tracks: [...playlist.value.tracks, ...appended],
    };
    const total = Number(detail.total || detail.trackCount || playlist.value.trackCount || 0);
    playlistHasMore.value =
      appended.length > 0 &&
      (total ? playlistPage.value * PAGE_SIZE < total : tracks.length === PAGE_SIZE);
    saveSnapshot(key);
  } catch (error) {
    if (sequence === searchSequence) message.value = `加载更多失败：${error}`;
  } finally {
    loadingMore.value = false;
  }
}

function onResultsScroll(event) {
  const element = event.currentTarget;
  if (element.scrollTop + element.clientHeight < element.scrollHeight - 36) return;
  void loadMore();
}

async function openPlaylist(item) {
  if (!item.id) return;
  const key = cacheKey();
  const sequence = ++searchSequence;
  loading.value = true;
  message.value = "";
  try {
    const raw = await invoke("get_playlist", {
      playlistId: item.id,
      sourceId: sourceKey(),
      offset: 0,
      limit: PAGE_SIZE,
    });
    const payload = JSON.parse(raw || "{}");
    if (sequence !== searchSequence) return;
    const detail = playlistDetail(payload);
    const tracks = playlistTracks(payload).map(normalizeSong);
    const total = Number(detail.trackCount || item.trackCount || tracks.length);
    playlist.value = {
      ...item,
      description: item.description || detail.description || "",
      trackCount: total,
      tracks,
    };
    playlistPage.value = 0;
    playlistHasMore.value = tracks.length > 0 && (total ? tracks.length < total : tracks.length === PAGE_SIZE);
    songs.value = [];
    message.value = playlist.value.tracks.length ? "" : "歌单中没有可播放歌曲";
    saveSnapshot(key);
  } catch (error) {
    if (sequence === searchSequence) message.value = `读取歌单失败：${error}`;
  } finally {
    if (sequence === searchSequence) loading.value = false;
  }
}

async function requestAddPlaylist(item) {
  if (loading.value) return;
  loading.value = true;
  message.value = "";
  try {
    const raw = await invoke("get_playlist", {
      playlistId: item.id,
      sourceId: sourceKey(),
      offset: 0,
      limit: Math.max(1, Number(item.trackCount) || 1000),
    });
    const payload = JSON.parse(raw || "{}");
    const tracks = playlistTracks(payload).map(normalizeSong);
    if (!tracks.length) {
      emit("toast", "歌单中没有可添加歌曲");
      return;
    }
    emit("add-playlist", {
      ...item,
      tracks: tracks.map((track) => ({ ...track, quality: effectiveQuality(track) })),
    });
  } catch (error) {
    emit("toast", `读取歌单失败：${error}`);
  } finally {
    loading.value = false;
  }
}

function closePlaylist() {
  playlist.value = null;
  playlistPage.value = 0;
  playlistHasMore.value = false;
  message.value = "";
  saveSnapshot();
}

function trialSong(song) {
  emit("add", { ...song, quality: effectiveQuality(song) });
}

function playNextSong(song) {
  emit("play-next", { ...song, quality: effectiveQuality(song) });
}

function songKey(song) {
  return `${sourceKey()}|${song?.cloudId || ""}`;
}

function isDefaultSong(song) {
  return props.defaultCloudKeys.includes(songKey(song));
}

function handleSongClick(song) {
  const key = songKey(song);
  window.clearTimeout(pendingSongClicks.get(key));
  pendingSongClicks.set(
    key,
    window.setTimeout(() => {
      pendingSongClicks.delete(key);
      if (isDefaultSong(song)) emit("play-existing", song);
    }, 220),
  );
}

function handleSongDoubleClick(song) {
  const key = songKey(song);
  window.clearTimeout(pendingSongClicks.get(key));
  pendingSongClicks.delete(key);
  if (isDefaultSong(song)) emit("play-existing", song);
  else emit("add", { ...song, quality: effectiveQuality(song) });
}

function selectTab(nextTab) {
  if (tab.value === nextTab) return;
  tab.value = nextTab;
  playlist.value = null;
  playlistPage.value = 0;
  playlistHasMore.value = false;
  const query = keyword.value.trim();
  if (!query) {
    currentPage.value = 0;
    hasMore.value = false;
    playlist.value = null;
    songs.value = [];
    playlists.value = [];
    message.value = "";
    return;
  }
  const cached = searchCache.get(cacheKey(query));
  if (cached) restoreSnapshot(cached);
  else void search();
}

async function refreshSearch() {
  const query = keyword.value.trim();
  if (!query) return;
  const cached = searchCache.get(cacheKey(query));
  if (cached) restoreSnapshot(cached);
  else await search();
}

async function changePlatform(event) {
  platform.value = event.target.value;
  await refreshSearch();
}

async function changeSource(event) {
  const source = props.sources.find((item) => item.id === event.target.value);
  if (!source || source.id === sourceKey()) return;
  sourceOverrideId = source.id;
  emit("select-source", source);
  await nextTick();
  await refreshSearch();
}

watch(
  () => props.activeSource?.id,
  (activeId) => {
    if (activeId && activeId === sourceOverrideId) sourceOverrideId = "";
  },
);

watch(
  () => props.sources.length,
  (sourceCount) => {
    if (sourceCount) return;
    sourceOverrideId = "";
    searchSequence += 1;
    songs.value = [];
    playlists.value = [];
    playlist.value = null;
    message.value = "";
    loading.value = false;
    loadingMore.value = false;
    currentPage.value = 0;
    hasMore.value = false;
  },
);
</script>

<template>
  <aside class="cloud-panel">
    <header class="cloud-header">
      <select
        v-if="sources.length"
        class="source-select"
        :value="activeSource?.id"
        aria-label="选择音源"
        @change="changeSource"
      >
        <option v-for="source in sources" :key="source.id" :value="source.id">{{ source.name }}</option>
      </select>
      <select
        v-if="sources.length"
        v-model="platform"
        class="source-select platform-select"
        aria-label="来源筛选"
        @change="changePlatform"
      >
        <option v-for="item in platformOptions" :key="item.id" :value="item.id">{{ item.name }}</option>
      </select>
      <button class="cloud-close" type="button" title="关闭云库" @click="emit('close')">
        <img :src="closeIcon" alt="" />
      </button>
    </header>
    <form v-if="sources.length" class="cloud-search" @submit.prevent="search">
      <div class="cloud-tabs">
        <button type="button" :class="{ active: tab === 'songs' }" @click="selectTab('songs')">歌曲</button>
        <button type="button" :class="{ active: tab === 'playlists' }" @click="selectTab('playlists')">歌单</button>
      </div>
      <input v-model="keyword" placeholder="搜索歌曲或歌单" spellcheck="false" />
      <button class="cloud-submit" type="submit" :disabled="loading">{{ loading ? "搜索中" : "搜索" }}</button>
    </form>
    <div v-else class="no-source">
      <strong>暂无可用音源</strong>
      <span>请前往设置导入音源后再搜索云库。</span>
      <button type="button" @click="emit('settings')">前往设置</button>
    </div>
    <div class="cloud-results" @scroll="onResultsScroll">
      <div v-if="message" class="cloud-message">{{ message }}</div>
      <div v-if="loadingMore" class="cloud-message">正在加载更多…</div>
      <template v-if="playlist">
        <button class="playlist-back" type="button" @click="closePlaylist">← 返回歌单列表</button>
        <div class="playlist-detail">
          <img v-if="playlist.coverUrl" class="playlist-detail-cover" :src="playlist.coverUrl" alt="" />
          <div class="playlist-detail-copy">
            <div class="playlist-detail-heading">
              <div class="playlist-title">{{ playlist.name }}</div>
              <button
                class="playlist-detail-add"
                :class="{ added: isPlaylistAdded(playlist.name) }"
                type="button"
                :title="isPlaylistAdded(playlist.name) ? '该歌单已在播放列表中' : '添加为分组并播放'"
                :disabled="loading || isPlaylistAdded(playlist.name)"
                @click="requestAddPlaylist(playlist)"
              >{{ isPlaylistAdded(playlist.name) ? "已添加" : "添加" }}</button>
            </div>
            <p class="playlist-description">{{ playlist.description || "暂无歌单介绍" }}</p>
          </div>
        </div>
        <div
          v-for="song in playlist.tracks"
          :key="song.cloudId"
          class="cloud-result-row"
          @click="handleSongClick(song)"
          @dblclick.stop="handleSongDoubleClick(song)"
        >
          <div class="cloud-result-copy">
            <span class="cloud-result-text">{{ song.title }} - {{ song.artist }}</span>
            <div class="cloud-qualities">
              <button
                v-for="quality in qualityOptions(song)"
                :key="quality"
                type="button"
                class="cloud-quality"
                :class="{ active: effectiveQuality(song) === quality }"
                @click.stop="chooseQuality(song, quality)"
              >{{ qualityLabel(quality) }}</button>
            </div>
          </div>
          <button class="cloud-row-action" type="button" title="试听" @click.stop="trialSong(song)">
            <img :src="playIcon" alt="" />
          </button>
          <button class="cloud-row-action" type="button" title="下一首播放" @click.stop="playNextSong(song)">
            <img :src="nextIcon" alt="" />
          </button>
        </div>
      </template>
      <template v-else-if="tab === 'songs'">
        <div
          v-for="song in songs"
          :key="song.cloudId"
          class="cloud-result-row"
          @click="handleSongClick(song)"
          @dblclick.stop="handleSongDoubleClick(song)"
        >
          <div class="cloud-result-copy">
            <span class="cloud-result-text">{{ song.title }} - {{ song.artist }}</span>
            <div class="cloud-qualities">
              <button
                v-for="quality in qualityOptions(song)"
                :key="quality"
                type="button"
                class="cloud-quality"
                :class="{ active: effectiveQuality(song) === quality }"
                @click.stop="chooseQuality(song, quality)"
              >{{ qualityLabel(quality) }}</button>
            </div>
          </div>
          <button class="cloud-row-action" type="button" title="试听" @click.stop="trialSong(song)">
            <img :src="playIcon" alt="" />
          </button>
          <button class="cloud-row-action" type="button" title="下一首播放" @click.stop="playNextSong(song)">
            <img :src="nextIcon" alt="" />
          </button>
        </div>
      </template>
      <template v-else>
        <div v-for="item in playlists" :key="item.id" class="cloud-playlist" @click="openPlaylist(item)">
          <img v-if="item.coverUrl" class="playlist-cover" :src="item.coverUrl" alt="" />
          <div class="playlist-copy">
            <span>{{ item.name }}</span>
            <small>{{ item.trackCount }} 首 · {{ item.creator }}</small>
          </div>
        </div>
      </template>
    </div>
  </aside>
</template>

<style scoped>
.cloud-panel {
  min-width: 0;
  flex: 1 1 520px;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border-left: 1px solid var(--line);
  background: var(--surface);
}
.cloud-header,
.cloud-search,
.cloud-tabs {
  display: flex;
  align-items: center;
}
.cloud-header {
  justify-content: space-between;
  padding: 7px 15px;
}
.source-select {
  min-width: 0;
  max-width: 190px;
  flex: 0 1 190px;
  padding: 7px 8px;
  border: 1px solid var(--line);
  border-radius: 6px;
  outline: none;
  background: var(--surface-2);
  color: var(--text);
  font-size: 14px;
}
.source-select:focus {
  border-color: var(--accent);
}
.platform-select {
  max-width: 112px;
  flex-basis: 112px;
  margin-left: 6px;
}
.cloud-close,
.cloud-tabs button,
.cloud-result-row button,
.playlist-back {
  border: 1px solid var(--line);
  background: var(--surface-2);
  color: var(--muted);
  cursor: pointer;
}
.cloud-close {
  width: 28px;
  height: 28px;
  padding: 6px;
  border-radius: 6px;
}
.cloud-close img {
  display: block;
  width: 100%;
  height: 100%;
}
.cloud-header .cloud-close {
  margin-left: auto;
}
.cloud-search {
  gap: 7px;
  margin: 10px 15px;
  padding: 7px 8px;
  border: 1px solid var(--line);
  border-radius: 7px;
  background: var(--canvas);
}
.cloud-search img {
  width: 15px;
  height: 15px;
}
.cloud-search input {
  min-width: 0;
  flex: 1;
  border: 0;
  outline: 0;
  background: transparent;
  color: var(--text);
  font-size: 14px;
}
.cloud-submit {
  border: 0;
  background: transparent;
  color: var(--accent);
  cursor: pointer;
  font-size: 13px;
}
.cloud-tabs {
  flex: 0 0 auto;
  gap: 4px;
  padding: 0;
}
.cloud-tabs button {
  padding: 4px 7px;
  border-radius: 5px;
  font-size: 13px;
}
.no-source {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 7px;
  margin: 18px 15px;
  padding: 20px 14px;
  border: 1px dashed var(--line);
  border-radius: 8px;
  color: var(--muted);
  text-align: center;
}
.no-source strong {
  color: var(--text);
  font-size: 15px;
}
.no-source span {
  font-size: 13px;
}
.no-source button {
  margin-top: 4px;
  padding: 6px 10px;
  border: 1px solid var(--accent);
  border-radius: 6px;
  background: transparent;
  color: var(--accent);
  cursor: pointer;
  font-size: 13px;
}
.cloud-tabs button.active {
  border-color: var(--accent);
  color: var(--accent);
}
.cloud-results {
  min-height: 0;
  flex: 1;
  overflow: auto;
  padding: 0 15px 15px;
}
.cloud-message,
.cloud-result-text,
.playlist-title {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.cloud-result-copy {
  min-width: 0;
  flex: 1;
  display: flex;
  flex-direction: row;
  align-items: center;
  gap: 8px;
}

.cloud-result-text {
  min-width: 0;
  flex: 1;
}

.cloud-qualities {
  flex: 0 0 auto;
  display: flex;
  flex-wrap: nowrap;
  align-items: center;
  gap: 6px;
  overflow: hidden;
  white-space: nowrap;
}

.cloud-quality {
  padding: 0 2px !important;
  border: 0 !important;
  background: transparent !important;
  color: var(--muted);
  cursor: pointer;
  font-size: 11px;
  line-height: 1.5;
}

.cloud-quality:hover {
  color: var(--accent);
}

.cloud-quality.active {
  color: var(--accent);
}
.cloud-message {
  padding: 16px 0;
  color: var(--faint);
  font-size: 13px;
}
.cloud-result-row {
  display: flex;
  align-items: center;
  gap: 8px;
  min-height: 35px;
  border-bottom: 1px solid var(--line);
  cursor: pointer;
}
.cloud-result-text {
  min-width: 0;
  flex: 1;
  color: var(--text);
  font-size: 14px;
}
.cloud-result-row button,
.playlist-back {
  padding: 5px 8px;
  border-radius: 5px;
  font-size: 12px;
}

.cloud-result-row .cloud-row-action {
  flex: 0 0 auto;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  padding: 0;
}

.cloud-result-row .cloud-row-action img {
  width: 15px;
  height: 15px;
  object-fit: contain;
  opacity: 0.85;
}
.playlist-back {
  margin-bottom: 8px;
}
.playlist-title {
  min-width: 0;
  margin-bottom: 0;
  color: var(--text);
  font-size: 15px;
  font-weight: 650;
}
.playlist-detail {
  display: flex;
  align-items: center;
  gap: 9px;
  margin-bottom: 8px;
}
.playlist-detail-copy {
  min-width: 0;
  flex: 1;
}
.playlist-detail-heading {
  display: flex;
  align-items: center;
  gap: 8px;
}
.playlist-detail-add {
  flex: 0 0 auto;
  padding: 7px 14px;
  border: 1px solid var(--accent);
  border-radius: 6px;
  background: color-mix(in srgb, var(--accent) 12%, transparent);
  color: var(--accent);
  cursor: pointer;
  font-size: 14px;
  font-weight: 600;
}
.playlist-detail-add:hover {
  background: color-mix(in srgb, var(--accent) 20%, transparent);
}
.playlist-detail-add.added,
.playlist-detail-add:disabled {
  border-color: var(--line);
  background: var(--surface-2);
  color: var(--muted);
  cursor: default;
  font-weight: 500;
}
.playlist-description {
  display: -webkit-box;
  overflow: hidden;
  margin: 5px 0 0;
  color: var(--muted);
  font-size: 13px;
  line-height: 1.5;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 3;
}
.playlist-detail-cover,
.playlist-cover {
  width: 46px;
  height: 46px;
  flex: 0 0 46px;
  border-radius: 6px;
  object-fit: cover;
  background: var(--surface-2);
}
.playlist-detail-cover {
  width: 58px;
  height: 58px;
  flex-basis: 58px;
}
.cloud-playlist {
  display: flex;
  align-items: center;
  gap: 9px;
  width: 100%;
  padding: 9px 0;
  border: 0;
  border-bottom: 1px solid var(--line);
  background: transparent;
  color: var(--text);
  cursor: pointer;
  text-align: left;
  cursor: pointer;
}
.playlist-copy {
  min-width: 0;
  flex: 1;
}
.cloud-playlist span,
.cloud-playlist small {
  display: block;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
.cloud-playlist span {
  font-size: 14px;
}
.cloud-playlist small {
  margin-top: 3px;
  color: var(--muted);
  font-size: 12px;
}

</style>
