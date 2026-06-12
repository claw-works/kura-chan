#include "serial_cmd.h"
#include "config_store.h"

static String cmd_buffer;

static void process_command(const String& cmd) {
    String trimmed = cmd;
    trimmed.trim();

    if (trimmed.startsWith("wifi add ")) {
        String rest = trimmed.substring(9);
        int space = rest.indexOf(' ');
        if (space > 0) {
            String ssid = rest.substring(0, space);
            String pass = rest.substring(space + 1);
            configStore.addWifi(ssid, pass);
            Serial.println("OK");
        } else {
            Serial.println("Usage: wifi add <ssid> <password>");
        }
    } else if (trimmed == "wifi list") {
        auto wifis = configStore.getWifiList();
        Serial.printf("%d networks:\n", wifis.size());
        for (auto& w : wifis) {
            Serial.printf("  %s\n", w.ssid.c_str());
        }
    } else if (trimmed == "wifi clear") {
        configStore.clearWifi();
        Serial.println("OK");
    } else if (trimmed.startsWith("server ")) {
        String host = trimmed.substring(7);
        host.trim();
        configStore.setServerHost(host);
        Serial.println("OK (reboot to apply)");
    } else if (trimmed.startsWith("port ")) {
        uint16_t port = trimmed.substring(5).toInt();
        configStore.setServerPort(port);
        Serial.println("OK (reboot to apply)");
    } else if (trimmed == "config") {
        configStore.dump();
    } else if (trimmed == "reboot") {
        Serial.println("Rebooting...");
        delay(100);
        ESP.restart();
    } else if (trimmed.length() > 0) {
        Serial.println("Commands: wifi add/list/clear, server <ip>, port <n>, config, reboot");
    }
}

void serial_cmd_update() {
    while (Serial.available()) {
        char c = Serial.read();
        if (c == '\n' || c == '\r') {
            if (cmd_buffer.length() > 0) {
                process_command(cmd_buffer);
                cmd_buffer = "";
            }
        } else {
            cmd_buffer += c;
        }
    }
}
