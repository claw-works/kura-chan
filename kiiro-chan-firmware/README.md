# Kiiro-chan（黄ちゃん）

学而思编程掌机重刷固件项目。原厂固件已通过 `esptool erase_flash` 完全擦除，现在是一块空白 ESP32，从零开发新固件。

## 硬件信息

通过 `esptool` 读取 + 用户确认：

- **主控**: ESP32-WROVER-B（ESP32-D0WD 芯片 + PSRAM 封装）
- **特性**: Wi-Fi, BT, Dual Core + LP Core, 240MHz
- **晶振**: 40MHz
- **MAC**: `90:38:0c:d1:ff:34`
- **Flash**: 4MB
- **屏幕**: ST7735 驱动，128×160（物理），横屏使用为 160×128
- **串口**: 因电脑/端口而异，当前 `/dev/cu.usbmodem1101`（USB-CDC 桥接芯片: GigaDevice GD32F3x0）

外设信息（引脚分配）参考社区开源项目 [pysn2012/xueersi-xiaomiao](https://github.com/pysn2012/xueersi-xiaomiao)（学而思"小喵掌机" ESP32 掌机），硬件资源：

- 显示：SPI TFT（驱动型号未知，暂按 ST7789 尝试），与 MicroSD 共享 SPI2
  - TFT: SCK=18, MOSI=23, CS=5, DC=4, RES=19
  - SD 卡: SCK=18(共享), MOSI=23(共享), MISO=19(共享), CS=22
- 按键（6键）：上=GPIO2, 下=GPIO13, 左=GPIO27, 右=GPIO35, A=GPIO34, B=GPIO12
- 无源蜂鸣器：GPIO14（PWM/LEDC）
- 传感器：光照 GPIO36（ADC1_CH0），热敏电阻 GPIO39（ADC1_CH3）
- I2C：SCL=GPIO15, SDA=GPIO21
- UART0（原生）：TX=GPIO1, RX=GPIO3
- 预留扩展：GPIO33/32/26/25

注意事项：
- GPIO34/35/36/39 仅输入，不可做输出
- GPIO12（B键）上电阶段不能有外部高电平（strapping pin）
- TFT/SD 共享 SPI 总线，靠各自 CS 分时复用

## 与 kura-chan 的关系

参考 [`kura-chan-firmware`](../kura-chan/kura-chan-firmware) 的方案（PlatformIO + Arduino framework + C/C++），但**代码不兼容**：kura-chan 面向 M5Stack CoreS3（ESP32-S3 + M5Unified 库），这台掌机是经典 ESP32 且外设完全不同，需要重新适配。

## 编译与刷写

```bash
cd kiiro-chan-firmware
pio run -e kiiro-chan -t upload      # 编译并刷固件
pio device monitor -b 115200         # 串口日志
```

## 当前状态

联网桌面助手（与 kura-chan-server 共享 actor 的文字终端）：

- **提醒推送**：服务端定时任务（cron/一次性）经 WS 推送 → 中文气泡 + 蜂鸣 + 自动亮屏，驻留到按 B 确认
- **交互**（B=返回，A=确认）：详情 → 菜单（最新信息/信息列表/任务列表/快捷语）；长文上下键滚动
- **消息历史**：NVS 持久化最近 10 条，带接收时间戳，重启保留
- **中文渲染**：HZK16/ASC16 点阵字库嵌入 flash（约 300KB），自研 UTF-8→GB2312 渲染器（`src/cn_font.*`）
- **角色立绘**：服务端 KRA1（RGB565+A8）合成图，设备端叠加表情层，右下角与气泡同屏
- **节能**：WiFi modem sleep 保持长连接、CPU 80MHz、20s 无操作熄屏（黑屏在线）
- **连接自愈**：WiFi 断线 30s 强制重连；WS 90s 未就绪重建连接

首次使用：复制 `src/secrets.example.h` 为 `src/secrets.h`，填入 WiFi 凭证与服务端 api_key。
