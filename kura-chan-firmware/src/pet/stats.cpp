#include "pet.h"
#include <LittleFS.h>
#include <ArduinoJson.h>

// Growth / needs model for the desktop pet. Pure logic + LittleFS persistence;
// no rendering. The renderer reads level via pet::setLevel for appearance.
namespace pet {

static const char* STATE_PATH = "/petstate.json";

static int s_level = 1;
static int s_xp = 0;        // xp toward next level
static int s_bond = 20;     // 0..100
static int s_energy = 100;  // 0..100
static long s_turns = 0;

static bool s_dirty = false;
static bool s_levelUp = false;
static uint32_t s_lastSave = 0;
static uint32_t s_lastDrift = 0;

static int xpNeed(int level) { return level * 100; }  // L1->2 needs 100, etc.

static int clampi(int v, int lo, int hi) { return v < lo ? lo : (v > hi ? hi : v); }

static void save() {
    JsonDocument doc;
    doc["level"] = s_level;
    doc["xp"] = s_xp;
    doc["bond"] = s_bond;
    doc["energy"] = s_energy;
    doc["turns"] = s_turns;
    File f = LittleFS.open(STATE_PATH, "w");
    if (f) { serializeJson(doc, f); f.close(); }
    s_dirty = false;
}

void statsBegin() {
    LittleFS.begin(true);  // idempotent; format on first use if needed
    File f = LittleFS.open(STATE_PATH, "r");
    if (f) {
        JsonDocument doc;
        if (!deserializeJson(doc, f)) {
            s_level = doc["level"] | 1;
            s_xp = doc["xp"] | 0;
            s_bond = doc["bond"] | 20;
            s_energy = doc["energy"] | 100;
            s_turns = doc["turns"] | 0;
        }
        f.close();
    } else {
        save();  // seed defaults
    }
    if (s_level < 1) s_level = 1;
    setLevel(s_level);
}

static void addXp(int amount) {
    s_xp += amount;
    while (s_xp >= xpNeed(s_level)) {
        s_xp -= xpNeed(s_level);
        s_level++;
        s_levelUp = true;
        setLevel(s_level);
    }
    s_dirty = true;
}

void onInteraction() {
    s_turns++;
    addXp(12);
    s_bond = clampi(s_bond + 2, 0, 100);
    s_energy = clampi(s_energy - 4, 0, 100);
    s_dirty = true;
}

void onHeadPat() {
    addXp(3);
    s_bond = clampi(s_bond + 3, 0, 100);
    s_dirty = true;
}

void statsTick(uint32_t now, bool resting) {
    // drift once per minute
    if (now - s_lastDrift >= 60000) {
        s_lastDrift = now;
        if (resting) {
            s_energy = clampi(s_energy + 4, 0, 100);  // recover while asleep/charging
        } else {
            s_energy = clampi(s_energy - 1, 0, 100);   // tire slowly while awake
        }
        static int decayCnt = 0;
        if (++decayCnt >= 30) { decayCnt = 0; s_bond = clampi(s_bond - 1, 0, 100); }  // gentle decay ~30min
        s_dirty = true;
    }
    // autosave at most every 30s when dirty
    if (s_dirty && now - s_lastSave >= 30000) {
        s_lastSave = now;
        save();
    }
}

Stats getStats() {
    return Stats{s_level, s_xp, xpNeed(s_level), s_bond, s_energy, s_turns};
}

bool consumeLevelUp() {
    if (s_levelUp) { s_levelUp = false; return true; }
    return false;
}

}  // namespace pet
