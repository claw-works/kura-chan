import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize } from "@tauri-apps/api/window";

let httpBase = "http://127.0.0.1:18099";
let gender = "girl";
let appearance: Record<string, any> = {};
let subtitle = "";
let recording = false;

// expression / animation state
let currentExpr = "neutral"; // resting face from latest mood
let speaking = false;
let talkTimer: number | undefined;

// ---- audio playback (TTS): PCM16/16k chunks scheduled back-to-back ----
let audioCtx: AudioContext | null = null;
let nextAudioTime = 0;
function ensureCtx(): AudioContext {
  if (!audioCtx) audioCtx = new AudioContext();
  if (audioCtx.state === "suspended") void audioCtx.resume();
  return audioCtx;
}
function playPcmChunk(b64: string) {
  const ctx = ensureCtx();
  const bin = atob(b64);
  const n = bin.length >> 1;
  if (n === 0) return;
  const buf = ctx.createBuffer(1, n, 16000);
  const ch = buf.getChannelData(0);
  for (let i = 0; i < n; i++) {
    const lo = bin.charCodeAt(i * 2);
    const hi = bin.charCodeAt(i * 2 + 1);
    let s = (hi << 8) | lo;
    if (s >= 0x8000) s -= 0x10000;
    ch[i] = s / 32768;
  }
  const src = ctx.createBufferSource();
  src.buffer = buf;
  src.connect(ctx.destination);
  const t = Math.max(nextAudioTime, ctx.currentTime);
  src.start(t);
  nextAudioTime = t + buf.duration;
}

function el(sel: string): HTMLElement {
  return document.querySelector(sel)!;
}
function setStatus(s: string) {
  el("#status").textContent = s;
}
function setBubble(s: string) {
  const b = el("#bubble");
  b.textContent = s;
  b.classList.toggle("show", !!s);
}

// Only the CHARACTER scales (via --pet-h); controls stay fixed-size and the
// window expands on hover to fit them, then collapses back to just the pet.
const PET_SIZES = [64, 120, 200, 300];
const PET_RATIO = 0.83; // character width / height
const CTRL_W = 240; // fixed control area width (expanded)
const CTRL_H = 92; // fixed control area height (toolbar + chat row)
let sizeIdx = 2;

function petDims(): [number, number] {
  const h = PET_SIZES[sizeIdx];
  return [Math.max(60, Math.round(h * PET_RATIO)), h];
}
async function collapse() {
  const [w, h] = petDims();
  document.documentElement.style.setProperty("--pet-h", h + "px");
  try {
    await getCurrentWindow().setSize(new LogicalSize(w, h));
  } catch (err) {
    console.error("setSize failed", err);
  }
}
async function expand() {
  const [w, h] = petDims();
  document.documentElement.style.setProperty("--pet-h", h + "px");
  try {
    await getCurrentWindow().setSize(new LogicalSize(Math.max(w, CTRL_W), h + CTRL_H));
  } catch (err) {
    console.error("setSize failed", err);
  }
}

function label(s: string): string {
  if (s === "connected") return "已连接";
  if (s === "connecting") return "连接中…";
  if (s === "closed") return "连接已关闭，重连中…";
  if (s.startsWith("disconnected")) return "已断开，重连中…";
  return s;
}

function variant(val: any, slot: string): string {
  if (typeof val !== "string" || !val) return "";
  let v = val.replace(/\.(png|webp|jpe?g)$/i, "");
  const i = v.indexOf(slot + "_");
  if (i >= 0) v = v.slice(i + slot.length + 1);
  return v;
}

// ---- face layer (expression / blink / talk) ----
function setFace(expr: string) {
  (el("#face") as HTMLImageElement).src =
    `${httpBase}/assets/${gender}/60_face_${expr}.png?h=480`;
}
function moodToFace(m: string): string {
  switch (m) {
    case "happy": return "happy_1";
    case "love": return "happy_2";
    case "sad": return "sad_1";
    case "angry": return "angry";
    case "surprised": return "surprise";
    case "confused": return "awkward";
    default: return "neutral";
  }
}
function startTalk() {
  if (talkTimer) return;
  let on = false;
  talkTimer = window.setInterval(() => {
    on = !on;
    setFace(on ? "base_talk" : currentExpr);
  }, 160);
}
function stopTalk() {
  if (talkTimer) {
    clearInterval(talkTimer);
    talkTimer = undefined;
  }
  setFace(currentExpr);
}
function scheduleBlink() {
  const delay = 2500 + Math.random() * 3500;
  window.setTimeout(() => {
    if (!speaking && currentExpr === "neutral") {
      setFace("base_blink");
      window.setTimeout(() => {
        if (!speaking) setFace(currentExpr);
      }, 130);
    }
    scheduleBlink();
  }, delay);
}

function loadPortrait() {
  const img = el("#portrait") as HTMLImageElement;
  const a = appearance || {};
  const p = new URLSearchParams();
  p.set("h", "480");
  const hairBack = variant(a.hair_back, "hair_back") || "short_black";
  const hairFront = variant(a.hair_front, "hair_front") || hairBack;
  const costume = variant(a.costume, "costume") || "jacket";
  p.set("hair_back", hairBack);
  p.set("hair_front", hairFront);
  p.set("costume", costume);
  if (a.blush === true || (typeof a.blush === "string" && a.blush)) {
    p.set("blush", typeof a.blush === "string" ? variant(a.blush, "blush") || "faint" : "faint");
  }
  if (a.glasses === true) p.set("glasses", "1");
  p.set("face", "none"); // body only; face is a separate animated layer
  p.set("t", String(Date.now()));
  img.src = `${httpBase}/assets/composite_png/${gender}?${p.toString()}`;
  setFace(currentExpr);
}

async function init() {
  await listen<string>("ws-status", (e) => setStatus(label(String(e.payload))));
  await listen<any>("subtitle", (e) => {
    subtitle += e.payload?.text ?? "";
    if (subtitle) setBubble(subtitle);
  });
  await listen<any>("sync", (e) => {
    const p = e.payload;
    if (p?.gender) gender = p.gender;
    if (p?.appearance) appearance = p.appearance;
    loadPortrait();
  });
  // mood from the agent → switch resting expression
  await listen<any>("response", (e) => {
    const m = e.payload?.emotion;
    if (typeof m === "string" && m) {
      currentExpr = moodToFace(m);
      if (!speaking) setFace(currentExpr);
    }
  });
  // TTS audio: reply starts → talk animation; play chunks
  await listen("audio-start", () => {
    const ctx = ensureCtx();
    nextAudioTime = ctx.currentTime;
    speaking = true;
    startTalk();
  });
  await listen<string>("audio", (e) => playPcmChunk(e.payload as string));
  await listen("speak_done", () => {
    speaking = false;
    stopTalk();
  });

  try {
    const cfg = await invoke<{ httpBase: string; status: string }>("get_config");
    if (cfg?.httpBase) httpBase = cfg.httpBase;
    if (cfg?.status) setStatus(label(cfg.status));
  } catch {
    /* keep defaults */
  }
  loadPortrait();
  scheduleBlink();

  const form = el("#chat-form") as HTMLFormElement;
  form.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const input = el("#chat-input") as HTMLInputElement;
    const text = input.value.trim();
    if (!text) return;
    ensureCtx();
    subtitle = "";
    setBubble("…");
    input.value = "";
    try {
      await invoke("send_text", { text });
    } catch (err) {
      setBubble("发送失败：" + err);
    }
  });

  const mic = el("#mic-btn");
  mic.addEventListener("click", async () => {
    ensureCtx();
    if (!recording) {
      recording = true;
      mic.classList.add("recording");
      subtitle = "";
      setBubble("聆听中…");
      try {
        await invoke("start_recording");
      } catch (err) {
        recording = false;
        mic.classList.remove("recording");
        setBubble("录音失败：" + err);
      }
    } else {
      recording = false;
      mic.classList.remove("recording");
      setBubble("思考中…");
      try {
        await invoke("stop_recording");
      } catch (err) {
        setBubble("发送失败：" + err);
      }
    }
  });

  // hover expands the window to fit the fixed-size controls; collapse back to
  // just the character when the pointer leaves (unless typing or recording).
  const app = el("#app");
  app.addEventListener("mouseenter", () => void expand());
  app.addEventListener("mouseleave", () => {
    const input = el("#chat-input") as HTMLInputElement;
    if (document.activeElement === input || recording) return;
    void collapse();
  });

  // toolbar: cycle character size / close window
  el("#size-btn").addEventListener("click", () => {
    sizeIdx = (sizeIdx + 1) % PET_SIZES.length;
    void expand(); // pointer is over the toolbar → stay expanded
  });
  el("#close-btn").addEventListener("click", () => {
    void getCurrentWindow().close();
  });
  await collapse();
}

window.addEventListener("DOMContentLoaded", init);
