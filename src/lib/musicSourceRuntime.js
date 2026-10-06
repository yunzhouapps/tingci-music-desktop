import { invoke } from "@tauri-apps/api/core";

const runtimeCache = new Map();
let runtimeQueue = Promise.resolve();

function rotateLeft(value, count) {
  return (value << count) | (value >>> (32 - count));
}

function md5(value) {
  const bytes = new TextEncoder().encode(String(value));
  const paddedLength = (((bytes.length + 8) >> 6) + 1) << 6;
  const data = new Uint8Array(paddedLength);
  data.set(bytes);
  data[bytes.length] = 0x80;
  const view = new DataView(data.buffer);
  const bitLength = bytes.length * 8;
  view.setUint32(paddedLength - 8, bitLength >>> 0, true);
  view.setUint32(paddedLength - 4, Math.floor(bitLength / 4294967296), true);
  const shifts = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];
  const constants = Array.from({ length: 64 }, (_, index) =>
    Math.floor(Math.abs(Math.sin(index + 1)) * 4294967296),
  );
  let a = 1732584193;
  let b = 4023233417;
  let c = 2562383102;
  let d = 271733878;
  for (let offset = 0; offset < paddedLength; offset += 64) {
    const words = Array.from({ length: 16 }, (_, index) => view.getUint32(offset + index * 4, true));
    const originalA = a;
    const originalB = b;
    const originalC = c;
    const originalD = d;
    for (let index = 0; index < 64; index += 1) {
      let mixed;
      let wordIndex;
      if (index < 16) {
        mixed = (b & c) | (~b & d);
        wordIndex = index;
      } else if (index < 32) {
        mixed = (d & b) | (~d & c);
        wordIndex = (5 * index + 1) % 16;
      } else if (index < 48) {
        mixed = b ^ c ^ d;
        wordIndex = (3 * index + 5) % 16;
      } else {
        mixed = c ^ (b | ~d);
        wordIndex = (7 * index) % 16;
      }
      const shift = shifts[Math.floor(index / 16) * 4 + (index % 4)];
      const next = d;
      d = c;
      c = b;
      b = (b + rotateLeft((a + mixed + constants[index] + words[wordIndex]) | 0, shift)) | 0;
      a = next;
    }
    a = (a + originalA) | 0;
    b = (b + originalB) | 0;
    c = (c + originalC) | 0;
    d = (d + originalD) | 0;
  }
  return [a, b, c, d]
    .flatMap((part) => [part & 255, (part >>> 8) & 255, (part >>> 16) & 255, (part >>> 24) & 255])
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

function bytesFrom(value, encoding = "utf8") {
  if (value instanceof Uint8Array) return new Uint8Array(value);
  if (Array.isArray(value)) return new Uint8Array(value);
  if (typeof value !== "string") return new Uint8Array();
  if (encoding === "hex") return new Uint8Array((value.match(/.{1,2}/g) || []).map((part) => parseInt(part, 16) || 0));
  if (encoding === "base64") {
    const binary = typeof atob === "function" ? atob(value) : "";
    return Uint8Array.from(binary, (character) => character.charCodeAt(0));
  }
  return new TextEncoder().encode(value);
}

function bytesToString(value, format = "utf8") {
  const bytes = value instanceof Uint8Array ? value : new Uint8Array(value || []);
  if (format === "hex") return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
  if (format === "base64") {
    let binary = "";
    bytes.forEach((byte) => {
      binary += String.fromCharCode(byte);
    });
    return typeof btoa === "function" ? btoa(binary) : "";
  }
  if (format === "binary") return Array.from(bytes);
  return new TextDecoder().decode(bytes);
}

function normalizeHeaders(headers) {
  const result = {};
  if (!headers) return result;
  if (typeof headers.forEach === "function") {
    headers.forEach((value, key) => {
      result[key] = value;
    });
    return result;
  }
  Object.entries(headers).forEach(([key, value]) => {
    if (value !== undefined && value !== null) result[key] = String(value);
  });
  return result;
}

function parseBody(text, contentType = "") {
  const value = String(text || "").replace(/^\uFEFF/, "");
  if (!value) return "";
  if (contentType.toLowerCase().includes("json") || /^[\[{]/.test(value.trim())) {
    try {
      return JSON.parse(value);
    } catch {
      return value;
    }
  }
  return value;
}

function encodeBody(options) {
  if (options?.binary instanceof Uint8Array) return { fetchBody: options.binary, fallbackBody: "" };
  if (options?.formData) return { fetchBody: options.formData, fallbackBody: "" };
  if (options?.form) {
    const params = new URLSearchParams();
    Object.entries(options.form).forEach(([key, value]) => params.append(key, String(value)));
    return { fetchBody: params, fallbackBody: params.toString() };
  }
  if (typeof options?.body === "string") return { fetchBody: options.body, fallbackBody: options.body };
  if (options?.body && typeof options.body === "object") {
    const value = JSON.stringify(options.body);
    return { fetchBody: value, fallbackBody: value };
  }
  return { fetchBody: undefined, fallbackBody: "" };
}

async function requestHttp(url, options = {}) {
  const method = String(options.method || "GET").toUpperCase();
  const headers = normalizeHeaders(options.headers);
  const encoded = encodeBody(options);
  const fetchOptions = { method, headers: { ...headers } };
  if (encoded.fetchBody !== undefined) fetchOptions.body = encoded.fetchBody;
  try {
    const response = await fetch(url, fetchOptions);
    const text = await response.text();
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const contentType = response.headers.get("content-type") || "";
    return { status: response.status, headers: normalizeHeaders(response.headers), body: parseBody(text, contentType) };
  } catch {
    const raw = await invoke("source_http_request", {
      url,
      method,
      headers,
      body: encoded.fallbackBody || null,
      timeoutSecs: Math.max(1, Math.min(60, Math.round((options.timeout || 20000) / 1000))),
    });
    return { status: 200, headers: {}, body: parseBody(raw) };
  }
}

function createHost(scriptInfo) {
  let requestHandler = null;
  let initInfo = null;
  const eventNames = { request: "request", inited: "inited", updateAlert: "updateAlert" };
  const host = {
    EVENT_NAMES: eventNames,
    request(url, options, callback) {
      let cancelled = false;
      requestHttp(url, options)
        .then((response) => {
          if (!cancelled) callback(null, response, response.body);
        })
        .catch((error) => {
          if (!cancelled) callback(error instanceof Error ? error : new Error(String(error)), null, null);
        });
      return () => {
        cancelled = true;
      };
    },
    async send(eventName, data) {
      if (eventName === eventNames.inited) initInfo = data;
      return undefined;
    },
    async on(eventName, handler) {
      if (eventName !== eventNames.request) throw new Error(`不支持的音源事件：${eventName}`);
      requestHandler = handler;
      return undefined;
    },
    utils: {
      crypto: {
        md5,
        randomBytes(size) {
          const bytes = new Uint8Array(Math.max(0, Number(size) || 0));
          if (typeof globalThis.crypto?.getRandomValues === "function") globalThis.crypto.getRandomValues(bytes);
          else bytes.forEach((_, index) => (bytes[index] = Math.floor(Math.random() * 256)));
          return bytes;
        },
      },
      buffer: {
        from: (value, encoding = "utf8") => bytesFrom(value, encoding),
        bufToString: (value, format = "utf8") => bytesToString(value, format),
      },
    },
    currentScriptInfo: {
      name: scriptInfo.name || "",
      description: scriptInfo.description || "",
      version: scriptInfo.version || "",
      author: scriptInfo.author || "",
      homepage: scriptInfo.homepage || "",
      rawScript: scriptInfo.rawScript || "",
    },
    version: "2.0.0",
    env: "desktop",
  };
  return {
    host,
    getHandler: () => requestHandler,
    getInitInfo: () => initInfo,
  };
}

async function executeSource(source, script) {
  const bridge = createHost({
    ...source,
    rawScript: script,
  });
  const previousLx = globalThis.lx;
  globalThis.lx = bridge.host;
  try {
    new Function(script)();
  } finally {
    if (previousLx === undefined) delete globalThis.lx;
    else globalThis.lx = previousLx;
  }
  const deadline = Date.now() + 15000;
  while (!bridge.getHandler() && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  if (!bridge.getHandler()) throw new Error("音源没有注册请求处理函数");
  // 部分脚本稍后才通过 inited 事件上报支持的来源，这里额外等一小会儿
  const initDeadline = Date.now() + 3000;
  while (!bridge.getInitInfo() && Date.now() < initDeadline) {
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  return { ...bridge, source };
}

export function loadMusicSource(source, script) {
  const key = source?.hash || source?.path || source?.id || "unknown";
  if (runtimeCache.has(key)) return runtimeCache.get(key);
  const task = runtimeQueue.then(
    () => executeSource(source, script),
    () => executeSource(source, script),
  );
  runtimeQueue = task.then(
    () => undefined,
    () => undefined,
  );
  runtimeCache.set(key, task);
  return task.catch((error) => {
    runtimeCache.delete(key);
    throw error;
  });
}

export async function callMusicSource(source, script, action, musicInfo, quality, preferredSource = "") {
  const runtime = await loadMusicSource(source, script);
  const handler = runtime.getHandler();
  const initSources = Object.keys(runtime.getInitInfo()?.sources || {});
  const sourceKey = initSources.includes(preferredSource)
    ? preferredSource
    : initSources.includes(musicInfo.source)
      ? musicInfo.source
      : initSources.includes("wy")
        ? "wy"
        : initSources[0] || "wy";
  return await handler.call(runtime.host, {
    source: sourceKey,
    action,
    info: {
      type: quality,
      musicInfo: {
        ...musicInfo,
        source: musicInfo.source || sourceKey,
      },
    },
  });
}

// 音源脚本上报的来源 id → 本应用搜索接口支持的平台
const PLATFORM_MAP = {
  wy: { id: "netease", label: "网易" },
  tx: { id: "tencent", label: "企鹅" },
  qq: { id: "tencent", label: "企鹅" },
  kg: { id: "kugou", label: "酷狗" },
  kw: { id: "kuwo", label: "酷我" },
};

// 取当前音源支持、且本应用能搜索的来源列表
export async function getMusicSourcePlatforms(source, script) {
  const runtime = await loadMusicSource(source, script);
  const initSources = runtime.getInitInfo()?.sources || {};
  const platforms = [];
  for (const [key, info] of Object.entries(initSources)) {
    const mapped = PLATFORM_MAP[key.toLowerCase()];
    if (!mapped || platforms.some((item) => item.id === mapped.id)) continue;
    const name = info && typeof info.name === "string" && info.name.trim() ? info.name.trim() : mapped.label;
    platforms.push({ id: mapped.id, name });
  }
  return platforms;
}
