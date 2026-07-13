// WiFi 密钥配置模板 —— 复制为 secrets.h 并填入真实值（secrets.h 已 gitignored）

#pragma once

// 按顺序依次尝试，先连上的先用
struct WifiCred {
    const char* ssid;
    const char* password;
};

static const WifiCred WIFI_CREDS[] = {
    {"your-ssid-1", "your-password-1"},
    {"your-ssid-2", "your-password-2"},
};

// kura-chan-server 连接信息（POST /register 获取 api_key）
#define KURA_SERVER_HOST "your-server-host"
#define KURA_SERVER_PORT 26021
#define KURA_API_KEY "kc_..."
#define KURA_DEVICE_ID "KIIRO_CHAN_001"
