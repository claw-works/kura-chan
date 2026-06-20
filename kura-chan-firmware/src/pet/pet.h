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

// ===== Growth / needs model =====
struct Stats {
    int level;
    int xpInLevel;   // xp accumulated toward next level
    int xpNeed;      // xp required for next level
    int bond;        // 0..100 affection
    int energy;      // 0..100
    long totalTurns; // lifetime conversation turns
};

void statsBegin();                       // load persisted state (LittleFS)
void onInteraction();                    // a conversation turn completed
void onHeadPat();                        // head was patted
void statsTick(uint32_t nowMs, bool resting);  // drift + autosave (call from loop)
Stats getStats();
bool consumeLevelUp();                   // returns true once right after a level-up

}  // namespace pet
