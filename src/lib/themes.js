// 所有内置图标都是单色（#d4237a），因此用 hue-rotate 滤镜即可为图标统一换色。
// 基准色 #d4237a 的色相约 330.5 度，iconFilter 里的角度即相对它的偏移。
export const THEMES = [
  {
    id: "rose",
    name: "玫红",
    accent: "#e0407a",
    accent2: "#ff6aa2",
    iconFilter: "none",
  },
  {
    id: "classic",
    name: "经典蓝",
    accent: "#5b7cff",
    accent2: "#795eff",
    iconFilter: "none",
  },
  {
    id: "sky",
    name: "天蓝",
    accent: "#3b82f6",
    accent2: "#6366f1",
    iconFilter: "hue-rotate(-108deg) saturate(1.05)",
  },
  {
    id: "violet",
    name: "紫罗兰",
    accent: "#8b5cf6",
    accent2: "#a855f7",
    iconFilter: "hue-rotate(-60deg) saturate(1.1)",
  },
  {
    id: "teal",
    name: "青碧",
    accent: "#14b8a6",
    accent2: "#06b6d4",
    iconFilter: "hue-rotate(-150deg) saturate(1.05)",
  },
  {
    id: "green",
    name: "森绿",
    accent: "#22c55e",
    accent2: "#10b981",
    iconFilter: "hue-rotate(170deg) saturate(1.05)",
  },
  {
    id: "amber",
    name: "暖橙",
    accent: "#f59e0b",
    accent2: "#f97316",
    iconFilter: "hue-rotate(58deg) saturate(1.1) brightness(1.02)",
  },
  {
    id: "black",
    name: "炫黑",
    accent: "#111827",
    accent2: "#374151",
    iconFilter: "grayscale(1) brightness(0.3)",
  },
  {
    id: "azure",
    name: "天青",
    accent: "#0ea5e9",
    accent2: "#22d3ee",
    iconFilter: "hue-rotate(-132deg) saturate(1.1)",
  },
];

export const DEFAULT_THEME_ID = THEMES[0].id;

export function themeById(id) {
  return THEMES.find((theme) => theme.id === id) || THEMES[0];
}

export function themeStyle(id) {
  const theme = themeById(id);
  return {
    "--accent": theme.accent,
    "--accent-2": theme.accent2,
    "--icon-filter": theme.iconFilter,
  };
}
