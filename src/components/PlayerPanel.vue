<script setup>
import { computed, ref, watch } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import playIcon from "../assets/icons/bofang1.svg";
import pauseIcon from "../assets/icons/zanting.svg";
import previousIcon from "../assets/icons/shangyiqu.svg";
import nextIcon from "../assets/icons/xiayiqu.svg";
import sequenceIcon from "../assets/icons/pailie.svg";
import randomIcon from "../assets/icons/suiji.svg";
import repeatIcon from "../assets/icons/shuaxin.svg";
import volumeIcon from "../assets/icons/diyinliang.svg";
import mutedIcon from "../assets/icons/jingyin.svg";

const props = defineProps({
  state: { type: Object, required: true },
  currentSong: { type: Object, default: null },
  mediaReady: { type: Boolean, default: true },
});

const emit = defineEmits([
  "toggle",
  "previous",
  "next",
  "seek",
  "timeupdate",
  "duration",
  "canplay",
  "playing",
  "paused",
  "ended",
  "media-error",
  "volume",
  "mute",
  "mode",
]);

const audio = ref(null);
const localTime = ref(props.state.playback.currentTime || 0);
const audioDuration = ref(0);
let sourceReady = false;
let playRequested = false;
let resumeAfterLoad = false;
let sourceSwitching = false;
let endedDispatched = false;

const fallbackCover = new URL("../assets/icon.svg", import.meta.url).href;
const coverSrc = ref(fallbackCover);
const duration = computed(() => audioDuration.value || props.currentSong?.duration / 1000 || 0);
const progress = computed(() => {
  if (!duration.value) return 0;
  return Math.min(100, Math.max(0, (localTime.value / duration.value) * 100));
});
const modeLabel = computed(
  () =>
    ({
      sequence: "顺序播放",
      random: "随机播放",
      repeat: "单曲循环",
    })[props.state.playback.playMode] || "顺序播放",
);
const modeIcon = computed(() => {
  if (props.state.playback.playMode === "random") return randomIcon;
  if (props.state.playback.playMode === "repeat") return repeatIcon;
  return sequenceIcon;
});

const currentQuality = computed(() => {
  const value = String(props.currentSong?.quality || props.currentSong?.cloudQuality || "").trim();
  if (!value) return "";
  if (value === "flac24bit") return "FLAC 24bit";
  if (value === "flac") return "FLAC";
  return value;
});

const lyricLines = computed(() => {
  const raw = props.currentSong?.lyrics || "";
  return raw
    .split(/\r?\n/)
    .flatMap((line) => {
      const timestamps = [...line.matchAll(/\[(\d+):(\d+(?:\.\d+)?)\]/g)];
      const text = line.replace(/\[[^\]]+\]/g, "").trim();
      if (!text || !timestamps.length) return [];
      return timestamps.map((match) => ({
        time: Number(match[1]) * 60 + Number(match[2]),
        text,
      }));
    })
    .sort((left, right) => left.time - right.time);
});

const currentLyric = computed(() => {
  if (props.currentSong?.loading) return "加载中";
  if (!lyricLines.value.length) return props.currentSong?.album || "听词";
  let active = "";
  for (const line of lyricLines.value) {
    if (line.time <= localTime.value + 0.15) active = line.text;
    else break;
  }
  return active || lyricLines.value[0].text;
});

function formatTime(value) {
  if (!Number.isFinite(value) || value < 0) return "0:00";
  const seconds = Math.floor(value % 60)
    .toString()
    .padStart(2, "0");
  const minutes = Math.floor(value / 60);
  return `${minutes}:${seconds}`;
}

function setAudioSource() {
  const element = audio.value;
  if (!element) return;
  const path = props.currentSong?.filePath;
  if (!path) {
    sourceReady = false;
    playRequested = false;
    element.autoplay = false;
    element.removeAttribute("src");
    element.load();
    return;
  }
  sourceReady = false;
  playRequested = false;
  endedDispatched = false;
  resumeAfterLoad = props.state.playback.isPlaying;
  sourceSwitching = true;
  element.autoplay = props.state.playback.isPlaying;
  element.src = convertFileSrc(path, "media");
  element.load();
}

async function playAudio() {
  const element = audio.value;
  if (!element || !props.currentSong || !sourceReady || playRequested) return;
  playRequested = true;
  try {
    await element.play();
    // play() 成功即代表播放已开始；若元素本来就在播放（例如单曲循环重播），
    // 浏览器不会再次触发 play 事件，这里必须复位，否则后续恢复会被误判为“正在请求播放”。
    playRequested = false;
  } catch (error) {
    playRequested = false;
    // 切换音源会让上一次 play() 以 AbortError 结束，这不是用户暂停。
    if (error && error.name === "AbortError") return;
    emit("paused");
    emit("media-error");
  }
}

function pauseAudio() {
  playRequested = false;
  resumeAfterLoad = false;
  sourceSwitching = false;
  audio.value?.pause();
}

function onLoadedMetadata() {
  audioDuration.value = Number.isFinite(audio.value?.duration) ? audio.value.duration : 0;
  if (audioDuration.value) emit("duration", audioDuration.value * 1000);
}

function onTimeUpdate() {
  if (!audio.value) return;
  localTime.value = audio.value.currentTime || 0;
  emit("timeupdate", localTime.value);
  if (
    audioDuration.value > 0 &&
    localTime.value >= audioDuration.value - 0.05 &&
    !endedDispatched
  ) {
    dispatchEnded();
  }
}

function onPlay() {
  playRequested = false;
  emit("playing");
}

function onPause() {
  // 切歌/加载期间、或本次播放请求还没真正开始时，忽略浏览器抛出的 pause。
  if (resumeAfterLoad || sourceSwitching || playRequested) return;
  emit("paused");
}

function onCanPlay() {
  sourceReady = true;
  sourceSwitching = false;
  endedDispatched = false;
  emit("canplay");
  if (resumeAfterLoad || props.state.playback.isPlaying) {
    resumeAfterLoad = false;
    void playAudio();
  }
}

function dispatchEnded() {
  if (endedDispatched) return;
  endedDispatched = true;
  emit("ended");
}

function onEnded() {
  dispatchEnded();
}

function onError() {
  sourceReady = false;
  playRequested = false;
  resumeAfterLoad = false;
  sourceSwitching = false;
  emit("paused");
  emit("media-error");
}

function onSeekInput(event) {
  const value = Number(event.target.value);
  localTime.value = value;
  if (audio.value) audio.value.currentTime = value;
  emit("seek", value);
}

function onVolumeInput(event) {
  emit("volume", Number(event.target.value));
}

watch(
  () => [props.currentSong?.id, props.currentSong?.filePath, props.mediaReady],
  () => {
    if (!props.mediaReady) {
      sourceReady = false;
      return;
    }
    localTime.value = 0;
    audioDuration.value = 0;
    void setAudioSource();
  },
  { immediate: true },
);

watch(
  () => [props.currentSong?.coverPath, props.mediaReady],
  ([coverPath, ready]) => {
    coverSrc.value = fallbackCover;
    if (!ready || !coverPath) return;
    coverSrc.value = convertFileSrc(coverPath, "media");
  },
  { immediate: true },
);

watch(
  () => props.state.playback.isPlaying,
  (playing) => {
    if (playing) void playAudio();
    else pauseAudio();
  },
);

watch(
  () => props.state.playback.currentTime,
  (value) => {
    if (!audio.value || Math.abs((audio.value.currentTime || 0) - value) > 0.6) {
      localTime.value = value || 0;
      endedDispatched = false;
      if (audio.value && Number.isFinite(value)) {
        audio.value.currentTime = value || 0;
        if (props.state.playback.isPlaying && audio.value.src) void playAudio();
      }
    }
  },
);

watch(
  () => props.state.settings.volume,
  (value) => {
    if (audio.value) audio.value.volume = Math.min(1, Math.max(0, value));
  },
  { immediate: true },
);

watch(
  () => props.state.settings.muted,
  (value) => {
    if (audio.value) audio.value.muted = Boolean(value);
  },
  { immediate: true },
);

</script>

<template>
  <section class="player-panel">
    <audio
      ref="audio"
      class="audio-element"
      preload="metadata"
      @loadedmetadata="onLoadedMetadata"
      @canplay="onCanPlay"
      @timeupdate="onTimeUpdate"
      @play="onPlay"
      @pause="onPause"
      @ended="onEnded"
      @error="onError"
    ></audio>
    <div class="track-overview">
      <img class="cover" :src="coverSrc" alt="当前歌曲封面" />
      <div class="track-copy">
        <div class="track-title" :title="currentSong?.title || '还没有播放歌曲'">
          <span class="track-title-text">{{ currentSong?.title || "还没有播放歌曲" }}</span>
          <span v-if="currentQuality" class="track-quality">{{ currentQuality }}</span>
        </div>
        <div class="track-artist">{{ currentSong?.artist || "选择一首歌开始试听" }}</div>
        <div
          class="track-album"
          :class="{ 'track-loading': currentSong?.loading }"
          :title="currentLyric"
        >{{ currentLyric }}</div>
      </div>
    </div>

    <div class="progress-row">
      <span class="time">{{ formatTime(localTime) }}</span>
      <input
        class="range progress-range"
        type="range"
        min="0"
        :max="duration || 0"
        step="0.1"
        :value="localTime"
        :disabled="!duration"
        aria-label="播放进度"
        @input="onSeekInput"
      />
      <span class="time">{{ formatTime(duration) }}</span>
    </div>

    <div class="transport-row">
      <div class="volume-row">
        <button class="icon-button" type="button" :title="state.settings.muted ? '取消静音' : '静音'" :aria-label="state.settings.muted ? '取消静音' : '静音'" @click="emit('mute')">
          <img class="button-icon" :src="state.settings.muted || state.settings.volume === 0 ? mutedIcon : volumeIcon" alt="" />
        </button>
        <input
          class="range volume-range"
          type="range"
          min="0"
          max="1"
          step="0.01"
          :value="state.settings.muted ? 0 : state.settings.volume"
          aria-label="音量"
          @input="onVolumeInput"
        />
        <span class="volume-value">{{ Math.round((state.settings.muted ? 0 : state.settings.volume) * 100) }}%</span>
      </div>
      <button class="transport-button" type="button" title="上一曲" aria-label="上一曲" @click="emit('previous')">
        <img class="button-icon" :src="previousIcon" alt="" />
      </button>
      <button class="play-button" type="button" :title="state.playback.isPlaying ? '暂停' : '播放'" :aria-label="state.playback.isPlaying ? '暂停' : '播放'" @click="emit('toggle')">
        <img class="play-icon" :src="state.playback.isPlaying ? pauseIcon : playIcon" alt="" />
      </button>
      <button class="transport-button" type="button" title="下一曲" aria-label="下一曲" @click="emit('next')">
        <img class="button-icon" :src="nextIcon" alt="" />
      </button>
      <button
        class="mode-button"
        type="button"
        :title="modeLabel"
        :aria-label="modeLabel"
        @click="emit('mode', state.playback.playMode === 'sequence' ? 'random' : state.playback.playMode === 'random' ? 'repeat' : 'sequence')"
      >
        <img class="button-icon mode-icon" :src="modeIcon" alt="" />
      </button>
    </div>

  </section>
</template>

<style scoped>
.player-panel {
  padding: 14px 14px 12px;
  background: var(--surface);
  border-bottom: 1px solid var(--line);
}

.audio-element {
  display: none;
}

.track-overview {
  display: flex;
  align-items: center;
  gap: 11px;
  min-width: 0;
}

.cover {
  width: 58px;
  height: 58px;
  flex: 0 0 auto;
  object-fit: cover;
  border-radius: 8px;
  background: var(--surface-2);
  box-shadow: var(--shadow-sm);
}

.track-copy {
  min-width: 0;
  flex: 1;
}

.track-title,
.track-artist,
.track-album {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.track-title {
  display: flex;
  align-items: baseline;
  gap: 6px;
  min-width: 0;
  font-weight: 650;
  font-size: 17px;
  color: var(--text);
}

.track-title-text {
  min-width: 0;
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.track-quality {
  flex: 0 0 auto;
  color: var(--accent);
  font-size: 12px;
  font-weight: 500;
}

.track-artist {
  margin-top: 2px;
  color: var(--muted);
  font-size: 14px;
}

.track-album {
  margin-top: 2px;
  color: var(--faint);
  font-size: 13px;
}

.track-album.track-loading {
  color: var(--accent);
}

.progress-row {
  display: grid;
  grid-template-columns: auto 1fr auto;
  align-items: center;
  gap: 7px;
  margin-top: 10px;
}

.time,
.volume-value {
  color: var(--muted);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}

.progress-track {
  height: 3px;
  margin-top: -1px;
  border-radius: 99px;
  overflow: hidden;
  background: var(--track);
}

.progress-track span {
  display: block;
  height: 100%;
  border-radius: inherit;
  background: linear-gradient(90deg, var(--accent), var(--accent-2));
}

.range {
  width: 100%;
  height: 4px;
  accent-color: var(--accent);
  cursor: pointer;
}

.transport-row {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 14px;
  margin-top: 12px;
}

.transport-button,
.icon-button,
.mode-button,
.play-button {
  border: 0;
  color: var(--text);
  background: transparent;
  cursor: pointer;
}

.transport-button,
.mode-button {
  width: 30px;
  height: 30px;
  padding: 0;
  border-radius: 7px;
  font-size: 18px;
}

.transport-button:hover,
.mode-button:hover,
.icon-button:hover {
  background: var(--hover);
}

.play-button {
  padding: 0;
  border-radius: 50%;
  background: transparent;
  color: var(--text);
  font-size: 17px;
  box-shadow: none;
}

.play-button:hover {
  background: transparent;
}

.mode-button {
  margin-left: 4px;
  font-size: 21px;
}

.volume-row {
  display: grid;
  grid-template-columns: 26px 1fr 34px;
  align-items: center;
  gap: 6px;
}

.icon-button {
  width: 26px;
  height: 26px;
  padding: 0;
  border-radius: 6px;
  font-size: 16px;
}

.button-icon {
  display: block;
  width: 16px;
  height: 16px;
  object-fit: contain;
  margin: 0 auto;
}

.play-icon {
  display: block;
  width: 32px;
  height: 32px;
  object-fit: contain;
  margin: auto;
}

.mode-icon {
  width: 17px;
  height: 17px;
}

.volume-range {
  height: 3px;
}
</style>
