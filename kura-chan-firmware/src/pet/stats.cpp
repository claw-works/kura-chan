#include "pet.h"
#include <LittleFS.h>
#include <ArduinoJson.h>

// Growth state is now server-authoritative. The device only displays the values
// pushed via sync; it caches them to LittleFS so something sensible shows before
// the first sync (offline). Level-ups are detected to trigger a celebration.
namespace pet {

static const char* STATE_PATH = "/petstate.json";

static int s_level = 1;
static int s_xp = 0;
static int s_xpNeed = 100;
static int s_bond = 20;
static int s_energy = 100;
static bool s_levelUp = false;

static void save() {
    JsonDocument doc;
    doc["level"] = s_level;
    doc["xp"] = s_xp;
    doc["xpNeed"] = s_xpNeed;
    doc["bond"] = s_bond;
    doc["energy"] = s_energy;
    File f = LittleFS.open(STATE_PATH, "w");
    if (f) { serializeJson(doc, f); f.close(); }
}

void statsBegin() {
    LittleFS.begin(true);
    File f = LittleFS.open(STATE_PATH, "r");
    if (f) {
        JsonDocument doc;
        if (!deserializeJson(doc, f)) {
            s_level = doc["level"] | 1;
            s_xp = doc["xp"] | 0;
            s_xpNeed = doc["xpNeed"] | 100;
            s_bond = doc["bond"] | 20;
            s_energy = doc["energy"] | 100;
        }
        f.close();
    }
    if (s_level < 1) s_level = 1;
    setLevel(s_level);
}

void setStats(int level, int xp, int xpNeed, int bond, int energy) {
    if (level > s_level) s_levelUp = true;
    s_level = level < 1 ? 1 : level;
    s_xp = xp;
    s_xpNeed = xpNeed > 0 ? xpNeed : 100;
    s_bond = bond;
    s_energy = energy;
    setLevel(s_level);
    save();
}

Stats getStats() {
    return Stats{s_level, s_xp, s_xpNeed, s_bond, s_energy, 0};
}

bool consumeLevelUp() {
    if (s_levelUp) { s_levelUp = false; return true; }
    return false;
}

}  // namespace pet
