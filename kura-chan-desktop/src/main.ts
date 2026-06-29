import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize, LogicalPosition, currentMonitor } from "@tauri-apps/api/window";

let httpBase = "http://127.0.0.1:18099";
let gender = "girl";
let appearance: Record<string, any> = {};
let subtitle = "";
let recording = false;

// expression / animation
let currentExpr = "neutral";
let speaking = false;
let talkTimer: number | undefined;

// window / layout state — only the CHARACTER scales; controls are fixed-size and
// the window expands on hover WITHOUT moving the character (position compensated).
const PET_SIZES = [96, 180, 300]; // 小 / 中 / 大 (character height)
const PET_RATIO = 0.83;
const CTRL_W = 260; // fixed control width when expanded
const MENU_H = 40; // menu row
const FORM_H = 48; // text input row (when open)
let sizeIdx = 1;
let expanded = false;
let curWinW = 0; // tracked current window logical width
let curCH = 0; // tracked current control height (0 when collapsed)
let curMode: "collapsed" | "down" | "up" = "collapsed";
let textOpen = false;

function petDims(): [number, number] {
  const h = PET_SIZES[sizeIdx];
  return [Math.max(72, Math.round(h * PET_RATIO)), h];
}
function ctrlH(): number {
  return textOpen ? MENU_H + FORM_H : MENU_H;
}
async function applyWindow(expand: boolean) {
  const win = getCurrentWindow();
  const [pw, ph] = petDims();
  const ch = expand ? ctrlH() : 0;
  const newW = expand ? Math.max(pw, CTRL_W) : pw;
  const newH = ph + ch;
  try {
    const pos = await win.outerPosition();
    const sf = await win.scaleFactor();
    const lx = pos.x / sf;
    const ly = pos.y / sf;
    // pet is horizontally centered; reconstruct its fixed screen center/top
    const petCenterX = lx + (curWinW || newW) / 2;
    const petTop = curMode === "up" ? ly + curCH : ly;

    // edge detection: open controls above the pet if no room below
    let ctrlTop = false;
    if (expand && ch > 0) {
      try {
        const mon = await currentMonitor();
        if (mon) {
          const msf = mon.scaleFactor || sf;
          const screenBottom = (mon.position.y + mon.size.height) / msf;
          ctrlTop = petTop + ph + ch + 8 > screenBottom;
        }
      } catch {
        /* default downward */
      }
    }

    const winX = petCenterX - newW / 2;
    const winY = ctrlTop ? petTop - ch : petTop;

    // Hide content while we resize+move, then reveal after layout settles —
    // the geometry jump happens invisibly so the menu never appears to jump.
    document.body.classList.add("settling");
    document.documentElement.style.setProperty("--pet-h", ph + "px");
    document.documentElement.style.setProperty("--ctrl-h", ch + "px");
    document.body.classList.toggle("ctrl-top", ctrlTop);
    await win.setSize(new LogicalSize(newW, newH));
    await win.setPosition(new LogicalPosition(Math.round(winX), Math.round(winY)));
    curWinW = newW;
    curCH = ch;
    curMode = !expand ? "collapsed" : ctrlTop ? "up" : "down";
    expanded = expand;
    requestAnimationFrame(() =>
      requestAnimationFrame(() => document.body.classList.remove("settling")),
    );
  } catch (err) {
    document.body.classList.remove("settling");
    console.error("applyWindow failed", err);
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
const SUBTITLE_MS = 5000; // auto-hide subtitle after N ms (will be configurable)
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
  await listen<any>("response", (e) => {
    const m = e.payload?.emotion;
    if (typeof m === "string" && m) {
      currentExpr = moodToFace(m);
      if (!speaking) setFace(currentExpr);
    }
  });
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

  // drag-and-drop: drop a text file onto the pet → read it and send as a message
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

  try {
    const cfg = await invoke<{ httpBase: string; status: string }>("get_config");
    if (cfg?.httpBase) httpBase = cfg.httpBase;
    if (cfg?.status) setStatus(label(cfg.status));
  } catch {
    /* keep defaults */
  }
  loadPortrait();
  scheduleBlink();

  // text input form
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

  // pointer hover via global cursor polling (works even when window isn't focused).
  // expand immediately; debounce collapse to absorb edge flicker + resize races.
  let collapseTimer: number | undefined;
  await listen<boolean>("hover", (e) => {
    if (e.payload) {
      if (collapseTimer) {
        clearTimeout(collapseTimer);
        collapseTimer = undefined;
      }
      if (!expanded) void applyWindow(true);
    } else {
      if (collapseTimer) clearTimeout(collapseTimer);
      collapseTimer = window.setTimeout(() => {
        collapseTimer = undefined;
        const input = el("#chat-input") as HTMLInputElement;
        if (document.activeElement === input || recording) return;
        if (textOpen) {
          textOpen = false;
          form.classList.add("hidden");
        }
        if (expanded) void applyWindow(false);
      }, 500);
    }
  });

  // menu: voice / text / sizes / settings / close
  el("#voice-btn").addEventListener("click", () => {
    if (!recording) void startVoice();
    else void stopVoice();
  });
  el("#text-btn").addEventListener("click", () => {
    textOpen = !textOpen;
    form.classList.toggle("hidden", !textOpen);
    void applyWindow(true);
    if (textOpen) (el("#chat-input") as HTMLInputElement).focus();
  });
  document.querySelectorAll(".size-btn").forEach((b) =>
    b.addEventListener("click", () => {
      sizeIdx = Number((b as HTMLElement).dataset.size);
      void applyWindow(expanded);
    })
  );
  el("#settings-btn").addEventListener("click", () => {
    setBubble("设置面板开发中…（下个版本）");
  });

  await applyWindow(false);
}

window.addEventListener("DOMContentLoaded", init);
