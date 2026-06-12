#pragma once

#include <Arduino.h>

// Processes serial commands for runtime config.
// Commands:
//   wifi add <ssid> <password>   — Add WiFi network
//   wifi list                    — List saved networks
//   wifi clear                   — Remove all networks
//   server <host>                — Set server host IP
//   port <port>                  — Set server port
//   config                       — Show all config
//   reboot                       — Restart device
void serial_cmd_update();
