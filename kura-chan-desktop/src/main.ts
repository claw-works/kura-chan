import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize, LogicalPosition, currentMonitor } from "@tauri-apps/api/window";

let httpBase = "http://127.0.0.1:18099";
let gender = "girl";
let appearance: Record<string, any> = {};
let subtitle = "";
let recording = false;
let lastInputText = false; // last turn came from typing → reply text-only (no TTS)

// expression / animation
let currentExpr = "neutral";
let speaking = false;
let talkTimer: number | undefined;

// layout: window = pet + a narrow vertical menu strip beside it.
const PET_SIZES = [96, 160, 260]; // 小 / 中 / 大 (cycled by one button)
const PET_RATIO = 0.83;
const MENU_W = 44; // vertical menu strip width
const MENU_MIN_H = 170; // min height to fit the vertical menu
const FORM_H = 42; // text input row height (when open)
let sizeIdx = 1;
let textOpen = false;

function applyDims(): { pw: number; ph: number } {
  const ph = PET_SIZES[sizeIdx];
  const pw = Math.round(ph * PET_RATIO);
  document.documentElement.style.setProperty("--pet-h", ph + "px");
  document.documentElement.style.setProperty("--pet-w", pw + "px");
  return { pw, ph };
}
async function applySize() {
  const { pw, ph } = applyDims();
  const winW = pw + MENU_W;
  const winH = Math.max(ph, MENU_MIN_H) + (textOpen ? FORM_H : 0);
  try {
    await getCurrentWindow().setSize(new LogicalSize(winW, winH));
    await clampToScreen();
  } catch (err) {
    console.error("setSize failed", err);
  }
}
// put the menu on the side with more room: window in right half → menu on left
async function updateSide() {
  try {
    const win = getCurrentWindow();
    const [pos, size, sf, mon] = await Promise.all([
      win.outerPosition(),
      win.outerSize(),
      win.scaleFactor(),
      currentMonitor(),
    ]);
    if (!mon) return;
    const msf = mon.scaleFactor || sf;
    const winCenterX = (pos.x + size.width / 2) / msf;
    const winCenterY = (pos.y + size.height / 2) / msf;
    const screenCenterX = (mon.position.x + mon.size.width / 2) / msf;
    const screenCenterY = (mon.position.y + mon.size.height / 2) / msf;
    document.body.classList.toggle("menu-left", winCenterX > screenCenterX);
    document.body.classList.toggle("pet-bottom", winCenterY > screenCenterY);
  } catch {
    /* ignore */
  }
}

// keep the whole window (pet + menu) on screen so the menu is never clipped
async function clampToScreen() {
  try {
    const win = getCurrentWindow();
    const [pos, size, sf, mon] = await Promise.all([
      win.outerPosition(),
      win.outerSize(),
      win.scaleFactor(),
      currentMonitor(),
    ]);
    if (!mon) return;
    const msf = mon.scaleFactor || sf;
    const x = pos.x / msf;
    const y = pos.y / msf;
    const w = size.width / msf;
    const h = size.height / msf;
    const sx = mon.position.x / msf;
    const sy = mon.position.y / msf;
    const sw = mon.size.width / msf;
    const sh = mon.size.height / msf;
    const nx = Math.min(Math.max(x, sx), sx + sw - w);
    const ny = Math.min(Math.max(y, sy), sy + sh - h);
    if (Math.round(nx) !== Math.round(x) || Math.round(ny) !== Math.round(y)) {
      await win.setPosition(new LogicalPosition(Math.round(nx), Math.round(ny)));
    }
  } catch {
    /* ignore */
  }
}

// ---- audio playback (TTS) ----
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
  const dot = el("#dot");
  dot.classList.toggle("connected", s === "connected");
  dot.setAttribute("title", label(s));
}
const SUBTITLE_MS = 5000;
let bubbleTimer: number | undefined;
function setBubble(s: string) {
  const b = el("#bubble");
  b.textContent = s;
  b.classList.toggle("show", !!s);
  if (bubbleTimer) clearTimeout(bubbleTimer);
  if (s) bubbleTimer = window.setTimeout(() => b.classList.remove("show"), SUBTITLE_MS);
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

// ---- face layer ----
function setFace(expr: string) {
  (el("#face") as HTMLImageElement).src = `${httpBase}/assets/${gender}/60_face_${expr}.png?h=480`;
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
  p.set("face", "none");
  p.set("t", String(Date.now()));
  img.src = `${httpBase}/assets/composite_png/${gender}?${p.toString()}`;
  setFace(currentExpr);
}

async function startVoice() {
  lastInputText = false; // mic → voice + text output
  ensureCtx();
  recording = true;
  el("#voice-btn").classList.add("recording");
  subtitle = "";
  setBubble("聆听中…");
  try {
    await invoke("start_recording");
  } catch (err) {
    recording = false;
    el("#voice-btn").classList.remove("recording");
    setBubble("录音失败：" + err);
  }
}
async function stopVoice() {
  recording = false;
  el("#voice-btn").classList.remove("recording");
  setBubble("思考中…");
  try {
    await invoke("stop_recording");
  } catch (err) {
    setBubble("发送失败：" + err);
  }
}

async function init() {
  await listen<string>("ws-status", (e) => setStatus(String(e.payload)));
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
  await listen<any>("response", (e) => {
    const m = e.payload?.emotion;
    if (typeof m === "string" && m) {
      currentExpr = moodToFace(m);
      if (!speaking) setFace(currentExpr);
    }
  });
  await listen("audio-start", () => {
    if (lastInputText) return; // text chat → no voice output / mouth animation
    const ctx = ensureCtx();
    nextAudioTime = ctx.currentTime;
    speaking = true;
    startTalk();
  });
  await listen<string>("audio", (e) => {
    if (lastInputText) return;
    playPcmChunk(e.payload as string);
  });
  await listen("speak_done", () => {
    speaking = false;
    currentExpr = "neutral";
    stopTalk();
  });

  // drag-and-drop: drop a text file onto the pet → read & send
  await getCurrentWindow().onDragDropEvent(async (ev) => {
    const pl = ev.payload as any;
    if (pl?.type !== "drop" || !Array.isArray(pl.paths)) return;
    for (const path of pl.paths) {
      const name = String(path).split(/[\\/]/).pop();
      try {
        const content = await invoke<string>("read_dropped", { path });
        ensureCtx();
        subtitle = "";
        setBubble("📎 " + name);
        await invoke("send_text", {
          text: `（我拖给你一个文件「${name}」，内容如下）\n${content}`,
        });
      } catch (err) {
        setBubble("📎 " + name + " 读取失败：" + err);
      }
    }
  });

  // re-evaluate menu side when the window is moved (debounced)
  let moveTimer: number | undefined;
  await getCurrentWindow().onMoved(() => {
    if (moveTimer) clearTimeout(moveTimer);
    moveTimer = window.setTimeout(async () => {
      document.body.classList.remove("dragging");
      await updateSide();
      await clampToScreen();
    }, 250);
  });

  try {
    const cfg = await invoke<{ httpBase: string; status: string }>("get_config");
    if (cfg?.httpBase) httpBase = cfg.httpBase;
    if (cfg?.status) setStatus(cfg.status);
  } catch {
    /* keep defaults */
  }
  loadPortrait();
  scheduleBlink();
  await applySize();
  await updateSide();

  // drag the pet to move the window; a plain click just focuses it (so the
  // OS lets the focused window receive hover events for the menu).
  const stage = el("#stage");
  let dragStartX = 0;
  let dragStartY = 0;
  let didDrag = false;
  stage.addEventListener("mousedown", (e) => {
    const me = e as MouseEvent;
    if (me.button !== 0) return;
    dragStartX = me.screenX;
    dragStartY = me.screenY;
    didDrag = false;
  });
  stage.addEventListener("mousemove", (e) => {
    const me = e as MouseEvent;
    if (me.buttons !== 1) return;
    if (!didDrag && (Math.abs(me.screenX - dragStartX) > 4 || Math.abs(me.screenY - dragStartY) > 4)) {
      didDrag = true;
      document.body.classList.add("dragging");
      void getCurrentWindow().startDragging();
    }
  });
  stage.addEventListener("click", () => {
    if (didDrag) {
      didDrag = false;
      return; // was a drag, not a click
    }
    document.body.classList.toggle("menu-open"); // click pet → toggle menu
  });

  // text input form
  const form = el("#chat-form") as HTMLFormElement;
  form.addEventListener("submit", async (ev) => {
    ev.preventDefault();
    const input = el("#chat-input") as HTMLInputElement;
    const text = input.value.trim();
    if (!text) return;
    lastInputText = true; // typed → reply as text only
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

  // menu buttons
  el("#voice-btn").addEventListener("click", () => {
    if (!recording) void startVoice();
    else void stopVoice();
  });
  el("#text-btn").addEventListener("click", async () => {
    textOpen = !textOpen;
    form.classList.toggle("hidden", !textOpen);
    await applySize();
    if (textOpen) (el("#chat-input") as HTMLInputElement).focus();
  });
  el("#size-btn").addEventListener("click", () => {
    sizeIdx = (sizeIdx + 1) % PET_SIZES.length;
    void applySize();
  });
  el("#settings-btn").addEventListener("click", () => {
    setBubble("设置面板开发中…（下个版本）");
  });
}

window.addEventListener("DOMContentLoaded", init);
