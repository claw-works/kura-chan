#pragma once
#include <Arduino.h>

// Procedural chibi desktop-pet character engine.
// Owns the LCD via its own render task. Fed by high-level mood/state from the
// rest of the firmware; all animation (breathe/blink/look/talk/react) is local.
namespace pet {

enum class Mood : uint8_t { Neutral, Happy, Sad, Angry, Surprised, Love, Confused, Sleepy };

// One-shot reactions layered on top of the current mood/idle.
enum class React : uint8_t { None, Hop, Nuzzle, Startle, Wave };

// Create the off-screen canvas and start the render task. Call after M5.begin()
// and after SD is mounted. `charId` selects the asset folder /pet/<charId>/.
void init(const char* charId);

// High-level inputs (thread-safe: simple scalar writes read by the render task).
void setMood(Mood m);
void setMoodByName(const char* emotion);   // "happy"/"sad"/... -> Mood
void setSpeaking(bool speaking);            // talking -> mouth animates
void setListening(bool listening);          // attentive pose
void setThinking(bool thinking);            // pondering pose
void setAsleep(bool asleep);                // sleep + Zzz
void react(React r);                        // trigger a one-shot reaction

// Appearance level (1..N): tweaks size/accessory as the pet "grows".
void setLevel(int level);

// ===== Dynamic appearance (dialog-driven) =====
void wear(const char* token);     // select a hair/costume/blush variant by name token
void setBlush(bool on);           // show/hide blush (default off)
void setAccessory(bool on);       // show/hide accessory e.g. glasses (default on)
void setCharacter(const char* id);// switch character folder /pet/<id>/ (reloads)

// ===== Growth / needs model (server-authoritative; device displays synced values) =====
struct Stats {
    int level;
    int xpInLevel;
    int xpNeed;
    int bond;
    int energy;
    long totalTurns;
};

void statsBegin();                       // load cached state (offline display before first sync)
void setStats(int level, int xp, int xpNeed, int bond, int energy);  // from server sync
Stats getStats();
bool consumeLevelUp();                   // true once right after level increased

// ===== Server-driven appearance + assets =====
void setServer(const char* host, uint16_t port);          // for fetching art over HTTP
void applySync(const char* gender, const char* appearanceJson); // gender + appearance from sync
String appearanceJson();                 // current appearance selection (for status report)
void setBg(const char* name);            // scene background (fetched from /assets/bg/<name>.png; "" = none)

// ===== Status overlay =====
// Drawn by the render task (never by main) to avoid cross-task LCD access.
// main feeds the page text and toggles visibility.
void showStatus(bool on);              // show/hide the status page
void setStatusText(const char* text);  // '\n'-separated lines for the status page

}  // namespace pet
