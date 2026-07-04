import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow, LogicalSize, LogicalPosition, PhysicalPosition, currentMonitor } from "@tauri-apps/api/window";

let httpBase = "http://127.0.0.1:18099";
let gender = "girl";
let appearance: Record<string, any> = {};
let subtitle = "";
let recording = false;
let voicePoll: number | undefined; // VAD auto-stop poller

// expression / animation
let currentExpr = "neutral";
let speaking = false;
let talkTimer: number | undefined;

// layout: window = pet + a narrow vertical menu strip beside it.
const PET_SIZES = [96, 160, 260]; // 小 / 中 / 大 (cycled by one button)
const PET_RATIO = 0.83;
const MENU_W = 44; // vertical menu strip width
const MENU_MIN_H = 200; // min height to fit the vertical menu (dot + 5 buttons)
let sizeIdx = 1;
let chatMode = false;
let ttsOn = true; // speaker toggle: play TTS audio or not
let streamingBot: HTMLElement | null = null;
let botBuf = "";
let savedPos: { x: number; y: number } | null = null; // float-mode position to restore

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
  const winH = Math.max(ph, MENU_MIN_H);
  try {
    await getCurrentWindow().setSize(new LogicalSize(winW, winH));
    await clampToScreen();
  } catch (err) {
    console.error("setSize failed", err);
  }
}
// restore float-mode size AND the position before settings/chat opened
async function restoreFloat() {
  const { pw, ph } = applyDims();
  try {
    await getCurrentWindow().setSize(new LogicalSize(pw + MENU_W, Math.max(ph, MENU_MIN_H)));
    if (savedPos) {
      await getCurrentWindow().setPosition(new PhysicalPosition(savedPos.x, savedPos.y));
      savedPos = null;
    } else {
      await clampToScreen();
    }
  } catch (err) {
    console.error(err);
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
  if (bubbleTimer) clearTimeout(bubbleTimer);
  if (!s) {
    void invoke("hide_subtitle");
    return;
  }
  void positionAndShowSubtitle(s);
  bubbleTimer = window.setTimeout(() => void invoke("hide_subtitle"), SUBTITLE_MS);
}
// Position the independent subtitle window above/below the pet, clamped to the
// screen so it never runs off any of the four corners.
async function positionAndShowSubtitle(text: string) {
  const SUB_W = 300;
  const SUB_H = 90;
  const GAP = 6;
  try {
    const win = getCurrentWindow();
    const [pos, size, sf, mon] = await Promise.all([
      win.outerPosition(),
      win.outerSize(),
      win.scaleFactor(),
      currentMonitor(),
    ]);
    const msf = mon?.scaleFactor || sf;
    const subW = Math.round(SUB_W * msf);
    const subH = Math.round(SUB_H * msf);
    const gap = Math.round(GAP * msf);
    let x = Math.round(pos.x + size.width / 2 - subW / 2); // center over pet
    // pet in upper half → subtitle below it; lower half → above it
    const petCY = pos.y + size.height / 2;
    const scCY = mon ? mon.position.y + mon.size.height / 2 : petCY;
    let y = petCY < scCY ? pos.y + size.height + gap : pos.y - subH - gap;
    if (mon) {
      x = Math.max(mon.position.x, Math.min(x, mon.position.x + mon.size.width - subW));
      y = Math.max(mon.position.y, Math.min(y, mon.position.y + mon.size.height - subH));
    }
    await invoke("show_subtitle", { text, x, y });
  } catch (err) {
    console.error("subtitle position failed", err);
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
  img.onerror = () => console.error("[portrait] load failed:", img.src);
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
    // VAD: auto-send when the user stops talking (≈3s trailing silence)
    if (voicePoll) clearInterval(voicePoll);
    voicePoll = window.setInterval(async () => {
      try {
        if (recording && (await invoke<boolean>("is_voice_done"))) {
          await stopVoice();
        }
      } catch {
        /* ignore */
      }
    }, 250);
  } catch (err) {
    recording = false;
    el("#voice-btn").classList.remove("recording");
    setBubble("录音失败：" + err);
  }
}
async function stopVoice() {
  if (voicePoll) {
    clearInterval(voicePoll);
    voicePoll = undefined;
  }
  if (!recording) return; // already stopped (avoid double send)
  recording = false;
  el("#voice-btn").classList.remove("recording");
  setBubble("思考中…");
  try {
    await invoke("stop_recording");
  } catch (err) {
    setBubble("发送失败：" + err);
  }
}

function applySync(p: any) {
  if (!p) return;
  if (p.gender) gender = p.gender;
  if (p.appearance) appearance = p.appearance;
  loadPortrait();
}

async function init() {
  await listen<string>("ws-status", (e) => setStatus(String(e.payload)));
  await listen<any>("subtitle", (e) => {
    const t = e.payload?.text ?? "";
    if (chatMode) {
      botBuf += t;
      updateStreamingBot(stripTags(botBuf));
    } else {
      subtitle += t;
      if (subtitle) setBubble(subtitle);
    }
  });
  await listen<any>("sync", (e) => applySync(e.payload));
  // global hotkey: bring to screen, then toggle voice (start / stop-and-send)
  await listen("hotkey", async () => {
    document.body.classList.remove("ghost"); // exiting click-through
    try {
      await clampToScreen();
    } catch {
      /* ignore */
    }
    if (recording) await stopVoice();
    else await startVoice();
  });
  await listen<any>("response", (e) => {
    const m = e.payload?.emotion;
    if (typeof m === "string" && m) {
      currentExpr = moodToFace(m);
      if (!speaking) setFace(currentExpr);
    }
  });
  await listen("audio-start", () => {
    if (!ttsOn) return; // speaker off → no voice / mouth animation
    const ctx = ensureCtx();
    nextAudioTime = ctx.currentTime;
    speaking = true;
    startTalk();
  });
  await listen<string>("audio", (e) => {
    if (!ttsOn) return;
    playPcmChunk(e.payload as string);
  });
  await listen("speak_done", () => {
    speaking = false;
    currentExpr = "neutral";
    stopTalk();
    streamingBot = null;
    botBuf = "";
  });

  // drag-and-drop: drop a text file onto the pet → read & send
  // hide the menu when the window loses focus
  await getCurrentWindow().onFocusChanged((e) => {
    if (!e.payload) document.body.classList.remove("menu-open");
  });
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
      // persist floating-window position (skip chat/settings mode — those resize)
      if (!chatMode && !document.body.classList.contains("settings-open")) {
        try {
          const pos = await getCurrentWindow().outerPosition();
          await invoke("save_window_pos", { x: pos.x, y: pos.y });
        } catch {
          /* ignore */
        }
      }
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
  // restore last floating-window position (before computing menu side)
  try {
    const pos = await invoke<any>("get_window_pos");
    if (pos && typeof pos.x === "number" && typeof pos.y === "number") {
      await getCurrentWindow().setPosition(new PhysicalPosition(pos.x, pos.y));
      await clampToScreen(); // saved pos may now be off-screen (monitor/res changed)
    }
  } catch {
    /* no saved position */
  }
  await updateSide();
  // catch up on the sync we may have missed before this listener was ready
  try {
    const s = await invoke<any>("get_last_sync");
    if (s) applySync(s);
  } catch {
    /* none yet */
  }

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

  // chat (dialog) mode controls
  const chatInput = el("#chat-input") as HTMLInputElement;
  function sendChat() {
    const text = chatInput.value.trim();
    if (!text) return;
    appendMsg("user", text);
    botBuf = "";
    streamingBot = null;
    chatInput.value = "";
    if (ttsOn) ensureCtx();
    void invoke("send_text", { text });
  }
  el("#chat-send").addEventListener("click", sendChat);
  // IME-safe Enter: macOS WKWebView's isComposing alone isn't reliable, so also
  // track composition state + the moment a candidate was just confirmed.
  let composing = false;
  let lastCompositionEnd = 0;
  chatInput.addEventListener("compositionstart", () => {
    composing = true;
  });
  chatInput.addEventListener("compositionend", () => {
    composing = false;
    lastCompositionEnd = Date.now();
  });
  chatInput.addEventListener("keydown", (e) => {
    const ke = e as KeyboardEvent;
    if (ke.key !== "Enter") return;
    if (composing || ke.isComposing || ke.keyCode === 229 || Date.now() - lastCompositionEnd < 120) {
      return; // IME candidate confirmation, not a real send
    }
    e.preventDefault();
    sendChat();
  });
  el("#chat-exit").addEventListener("click", () => void exitChat());
  el("#tts-btn").addEventListener("click", () => {
    ttsOn = !ttsOn;
    syncTtsBtn();
  });

  // menu buttons
  el("#voice-btn").addEventListener("click", () => {
    if (!recording) void startVoice();
    else void stopVoice();
  });
  el("#text-btn").addEventListener("click", () => void enterChat());
  el("#size-btn").addEventListener("click", () => {
    sizeIdx = (sizeIdx + 1) % PET_SIZES.length;
    void applySize();
  });
  el("#settings-btn").addEventListener("click", () => void openSettings());
  el("#ghost-btn").addEventListener("click", async () => {
    document.body.classList.remove("menu-open");
    document.body.classList.add("ghost");
    try {
      await invoke("set_click_through", { on: true });
      setBubble("穿透已开启，按全局热键（默认 ⌘⇧K）唤回小爪");
    } catch (err) {
      document.body.classList.remove("ghost");
      setBubble("穿透失败：" + err);
    }
  });
  el("#set-cancel").addEventListener("click", () => void closeSettings());
  el("#set-save").addEventListener("click", async () => {
    try {
      await invoke("save_settings", {
        wsUrl: (el("#set-ws") as HTMLInputElement).value.trim(),
        httpBase: (el("#set-http") as HTMLInputElement).value.trim(),
        apiKey: (el("#set-key") as HTMLInputElement).value.trim(),
        deviceId: (el("#set-dev") as HTMLInputElement).value.trim(),
      });
      el("#set-msg").textContent = "已保存，重启应用后生效";
    } catch (err) {
      el("#set-msg").textContent = "保存失败：" + err;
    }
  });
}

async function openSettings() {
  try {
    const p = await getCurrentWindow().outerPosition();
    savedPos = { x: p.x, y: p.y };
  } catch {
    savedPos = null;
  }
  document.body.classList.remove("menu-open");
  try {
    const s = await invoke<any>("get_settings");
    (el("#set-ws") as HTMLInputElement).value = s.wsUrl || "";
    (el("#set-http") as HTMLInputElement).value = s.httpBase || "";
    (el("#set-key") as HTMLInputElement).value = s.apiKey || "";
    (el("#set-dev") as HTMLInputElement).value = s.deviceId || "";
  } catch {
    /* ignore */
  }
  el("#set-msg").textContent = "";
  document.body.classList.add("settings-open");
  try {
    await getCurrentWindow().setSize(new LogicalSize(340, 360));
    await clampToScreen();
  } catch (err) {
    console.error(err);
  }
}
async function closeSettings() {
  document.body.classList.remove("settings-open");
  await restoreFloat();
}

// ===== chat (dialog) mode =====
function stripTags(s: string): string {
  return s.replace(/\[[^\]]*\]/g, "").trim();
}
function syncTtsBtn() {
  const b = el("#tts-btn");
  b.textContent = ttsOn ? "🔊" : "🔇";
  b.classList.toggle("off", !ttsOn);
}
function appendMsg(role: string, text: string) {
  if (!text) return;
  const log = el("#chat-log");
  const div = document.createElement("div");
  div.className = "msg " + (role === "user" ? "user" : "bot");
  div.textContent = text;
  log.appendChild(div);
  log.scrollTop = log.scrollHeight;
}
// streaming assistant reply: keep updating the last bubble until speak_done
function updateStreamingBot(text: string) {
  const log = el("#chat-log");
  if (!streamingBot) {
    streamingBot = document.createElement("div");
    streamingBot.className = "msg bot";
    log.appendChild(streamingBot);
  }
  streamingBot.textContent = text;
  log.scrollTop = log.scrollHeight;
}
async function enterChat() {
  try {
    const p = await getCurrentWindow().outerPosition();
    savedPos = { x: p.x, y: p.y };
  } catch {
    savedPos = null;
  }
  chatMode = true;
  document.body.classList.remove("menu-open");
  document.body.classList.add("chat-mode");
  syncTtsBtn();
  try {
    await getCurrentWindow().setSize(new LogicalSize(380, 520));
    await clampToScreen();
  } catch (err) {
    console.error(err);
  }
  const log = el("#chat-log");
  log.innerHTML = "";
  streamingBot = null;
  botBuf = "";
  try {
    const h = await invoke<any>("get_history");
    for (const m of h.messages || []) {
      appendMsg(m.role === "user" ? "user" : "bot", stripTags(m.content));
    }
  } catch {
    /* no history / offline */
  }
  (el("#chat-input") as HTMLInputElement).focus();
}
async function exitChat() {
  chatMode = false;
  document.body.classList.remove("chat-mode");
  streamingBot = null;
  botBuf = "";
  await restoreFloat();
}

window.addEventListener("DOMContentLoaded", init);
