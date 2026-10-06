export const SHORTCUT_ACTIONS = ["prev", "next", "playPause", "volumeUp", "volumeDown", "mute"];

// 应用内快捷键默认值（窗口聚焦时生效，不受系统占用影响）
export const DEFAULT_SHORTCUTS = {
  prev: "ArrowLeft",
  next: "ArrowRight",
  playPause: "Space",
  volumeUp: "ArrowUp",
  volumeDown: "ArrowDown",
  mute: "M",
};

export const SHORTCUT_ROWS = [
  { action: "prev", label: "上一曲" },
  { action: "next", label: "下一曲" },
  { action: "playPause", label: "播放 / 暂停" },
  { action: "volumeUp", label: "音量 +" },
  { action: "volumeDown", label: "音量 -" },
  { action: "mute", label: "静音开关" },
];

export function emptyGlobalShortcuts() {
  return Object.fromEntries(SHORTCUT_ACTIONS.map((action) => [action, ""]));
}

export function normalizeShortcuts(value, defaults) {
  const result = { ...defaults };
  if (value && typeof value === "object") {
    SHORTCUT_ACTIONS.forEach((action) => {
      if (typeof value[action] === "string") result[action] = value[action].trim();
    });
  }
  return result;
}

const MODIFIER_KEYS = [
  "Control",
  "Shift",
  "Alt",
  "Meta",
  "AltGraph",
  "CapsLock",
  "Dead",
  "Unidentified",
];

export function normalizeShortcutKey(key) {
  if (key === " " || key === "Spacebar") return "Space";
  if (/^Arrow(Left|Right|Up|Down)$/.test(key)) return key;
  if (/^F\d{1,2}$/.test(key)) return key;
  if (key.length === 1) return key.toUpperCase();
  if (["Enter", "Tab", "Backspace", "Delete", "Insert", "Home", "End", "PageUp", "PageDown", "Space"].includes(key)) {
    return key;
  }
  return "";
}

// 从键盘事件生成快捷键字符串；requireModifier 为真时必须有 Ctrl/Alt/Shift/Super
export function acceleratorFromEvent(event, requireModifier) {
  if (MODIFIER_KEYS.includes(event.key)) return { value: "", modifierOnly: true };
  const parts = [];
  if (event.ctrlKey) parts.push("Control");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  if (event.metaKey) parts.push("Super");
  if (requireModifier && !parts.length) return { value: "", reason: "modifier" };
  const key = normalizeShortcutKey(event.key);
  if (!key) return { value: "", reason: "unsupported" };
  return { value: [...parts, key].join("+") };
}

export function acceleratorMatches(event, accelerator) {
  const parts = String(accelerator || "")
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean);
  if (!parts.length) return false;
  const key = parts[parts.length - 1];
  const modifiers = parts.slice(0, -1).map((part) => part.toLowerCase());
  if (modifiers.includes("control") !== event.ctrlKey) return false;
  if (modifiers.includes("alt") !== event.altKey) return false;
  if (modifiers.includes("shift") !== event.shiftKey) return false;
  if (modifiers.includes("super") !== event.metaKey) return false;
  return normalizeShortcutKey(event.key) === key;
}
