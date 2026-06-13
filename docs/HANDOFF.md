# Kura-chan 工作交接文档

> 桌面 AI 伴侣机器人「小爪」。本文件用于换机/出差交接,描述**当前真实状态**与上手步骤。
> 早期设计文档(architecture.md / server-design.md / firmware-design.md / protocol.md)部分内容
> 与实现已有出入(如最初设想的 opus/ESP-SR/LVGL 未采用),**以本文件为准**。

---

## 1. 这是什么

三层架构:

```
M5Stack StackChan K151 (ESP32-S3) 固件        Rust 服务端 (axum WS)          AWS Bedrock AgentCore
  录音 / 播放 / 表情 / 舵机 / 灯      <—WS—>   STT→Agent→TTS 流式中继    <—>   harness "小爪"人设
  摸头唤醒 / 能量VAD                          火山(豆包)ASR + TTS
```

完整语音对话已打通:**摸头唤醒 → VAD 聆听 → 火山 ASR → AgentCore(小爪人设)→ 火山 TTS 流式 → 设备边收边播**,
并带状态表情、舵机姿态、RGB 灯、情绪联动、设备部件控制(音量/转头/灯)、电量上报。

## 2. 仓库结构

```
kura-chan/
├── kura-chan-firmware/        # PlatformIO 固件 (ESP32-S3, M5Unified)
│   ├── src/main.cpp           # 主程序(语音+表情+舵机+灯+控制 全集成)
│   ├── src/config/            # NVS 配置存储 + 串口命令
│   ├── src/ws/                # WebSocket 客户端
│   └── lib/FTServo/           # vendored 飞特舵机库(改过 UART 时钟源)
├── kura-chan-server/          # Rust 服务端 (edition 2024)
│   ├── src/ws/                # WS 握手/会话/协议/分句流式
│   ├── src/harness/           # AgentCore invoke_harness 流式
│   ├── src/speech/volc/       # 火山 ASR/TTS + 自定义二进制协议
│   ├── config/default.toml    # 可提交配置 + 小爪 system_prompt
│   └── patches/aws-runtime/   # ★ 必须保留的本地 patch(见下)
└── docs/
    ├── HANDOFF.md             # 本文件
    └── skill-device-control.md# 设备控制指令 skill(可配到 harness/server)
```

## 3. 环境依赖

- **服务端**:Rust(stable, edition 2024)、`cargo`。AWS 凭证(见 §5)。
- **固件**:PlatformIO(`pio`)。macOS 当前用 `/dev/cu.usbmodem2101` 烧录。
- 当前开发机 PlatformIO 自带 python:`/opt/homebrew/Cellar/platformio/*/libexec/bin/python`(UDP 遥测脚本用)。
- 注意:macOS 无 `timeout` 命令。

## 4. 密钥与配置(重要)

- **火山 API Key 只放运行时环境变量** `VOLC_API_KEY`,**绝不入 git**。
  请通过安全渠道(密码管理器/私下传递)携带该值,不要写进代码、配置或文档。
- `config/default.toml` 放可提交内容:火山 resource id、音色、小爪 system_prompt。
- **AgentCore harness ARN 也走环境变量** `HARNESS_ARN`(账号相关,不入 git);
  `config/default.toml` 里的 `harness_arn` 留空。region 可用 `AWS_REGION` 覆盖(默认 us-west-2)。
- 设备鉴权:`Authorization: Bearer dev_key_001` + `X-Device-Id`(server 侧目前鉴权放宽便于调试)。

## 5. 运行服务端

需要 AWS 凭证(`aws configure` 或环境变量),region us-west-2,有权限调用该 harness。

```bash
cd kura-chan-server
cargo build
VOLC_API_KEY=<你的key> HARNESS_ARN=<harness arn> RUST_LOG=info,kura_chan_server=debug \
  ./target/debug/kura-chan-server
# 监听 0.0.0.0:8080
```

后台跑(开发常用):
```bash
VOLC_API_KEY=<key> HARNESS_ARN=<arn> RUST_LOG=info,kura_chan_server=debug \
  nohup ./target/debug/kura-chan-server > /tmp/kura-server.log 2>&1 &
tail -f /tmp/kura-server.log
```

### ★ 必须保留的 patch
`Cargo.toml` 里有:
```toml
[patch.crates-io]
aws-runtime = { path = "patches/aws-runtime" }
```
这是为修复上游 `aws-runtime` 1.7.4 在 event-stream SigV4 下的编译 bug。**删了会编译失败**,换机器务必带上 `patches/` 目录。

## 6. 编译 / 烧录固件

```bash
cd kura-chan-firmware
pio run                                              # 编译
pio run -t upload --upload-port /dev/cu.usbmodem2101 # 烧录(按实际串口改)
```
- 串口在 macOS 是 HWCDC,普通 `Serial` 打印不稳;调试主要靠**屏幕显示**或 **UDP 遥测**(见 §10)。
- `src/main.cpp.voice.bak` 是开发中的旧备份,已 gitignore,可忽略。

## 7. 换机器后:把设备指向新服务端 IP(关键步骤)

设备 NVS 里存着旧服务端 IP(默认 `192.168.31.249:8080`)。换机器后服务端 IP 会变,需要重指向。
通过串口(115200)发命令(注意 HWCDC,可能需要合适的串口工具):

```
server <新机器局域网IP>     # 例: server 192.168.1.50
port 8080
config                      # 查看当前配置
reboot                      # 重启生效
```
其它串口命令:`wifi add <ssid> <pw>` / `wifi list` / `wifi clear`。
WiFi 首次默认 `yiyi-pro` / `99999999`(写在 NVS 默认值)。
若串口不通,可改 `src/config/config_store.cpp` 里的默认 `srv_host` 后重新烧录(需先 `wifi clear` 或工厂复位让默认值生效)。

设备 IP(参考):`192.168.31.123`;设备 ID:`KURA_CHAN_001`。

## 8. 硬件参数(舵机 / PY32 / 摸头)

- **舵机**(飞特 SCS 系列,UART1 / 1Mbps / TX=6 RX=7,半双工):
  - id1 = yaw(左右),**中心 470**,范围 ±80(≈±23°)。left = raw 低 / right = raw 高(方向已确认正确)。
  - id2 = pitch(上下),**中心 560 = 平视 = 最低**。**只能抬头,不能低头**(再低撞底座),上限 +130(≈+38°)。
  - 0.293°/步,WritePos 的 Time 字段必须用小值(代码用 20);**大 Time(如 800)舵机不动**——这是踩过的坑。
  - 平滑靠每 ~100ms 重复下发 look() 实现,不用大 Time。
  - 开机若舵机角度限位为 0/0(轮式模式)会自动恢复成位置模式。
- **PY32 IO 扩展**(I2C 0x6F):舵机电源 = pin0(输出+上拉+高);RGB 灯 = pin13(输出+上拉+推挽,12 颗),
  颜色 RGB565 写 0x30,刷新置 0x24 bit6。
- **摸头**:Si12T 电容触摸(内部 I2C 0x68),`head_touched()`。屏幕触控暂禁用,只用摸头唤醒。

## 9. 交互流程

休眠(暗屏 Zzz)─摸头→ 聆听 ─静音3.5s→ 思考 ─→ 说话 ─→ 追问聆听30秒(说则继续 / 静默回休眠)。
- VAD 能量阈值:起说 250 / 保持 150;静音结束 3.5s;最长录音 ~30s。
- 表情:Sleep/Idle/Listen/Think/Speak,说话由 `M5.Speaker.isPlaying()` 驱动嘴型,句间隙显示 think 脸。
- 舵机姿态:聆听微抬头、思考偶尔瞥一下(不再持续摆头)、说话轻微上下点头+小幅摆动、idle 回正。
- 灯:休眠灭 / idle 暗白 / 聆听蓝 / 思考琥珀 / 说话按情绪上色。

## 10. 设备控制 Skill(对话控制部件)

机制:小爪在回复里内联标记,服务端解析剥离后下发设备:
- `[mood:happy|sad|angry|surprised|love|confused|neutral]` 情绪 → 表情+灯色
- `[do:volume=N]` 音量、`[do:turn=left|right|up|center|nod|shake|auto]` 头部动作、`[do:led=COLOR|auto]` 灯色
- 设备每 30s 上报 `status`(电量/充电/音量),服务端注入 `[设备状态]` 到每轮上下文,小爪可回答电量/音量。

详见 **`docs/skill-device-control.md`**(含可直接粘贴到 harness/server 的提示词片段 + 协议参考)。
当前 system_prompt 写在 `kura-chan-server/config/default.toml` 的 `[agent]` 段。

## 11. 调试技巧

- 服务端日志:`tail -f /tmp/kura-server.log`(`RUST_LOG=...,kura_chan_server=debug`)。
- 固件:屏幕状态栏显示 IP / WS 状态 / 当前状态(LIS/THINK/SPK/idle)与 VAD 数值。
- UDP 遥测:固件 `dbg()` 发到 `192.168.31.249:8089`(换机后改 IP),可用 python 脚本监听。

## 12. 当前状态 / 已知改进点

已完成:语音全链路、表情、舵机(校准定稿)、灯、情绪联动、设备控制(音量/转头/抬头/点头/摇头/灯)、电量上报、skill 文档。

待办 / 已知点:
- **摄像头**(「你看到了什么」):harness 暂不支持图像输入,待多模态或 function-calling 方案。
- **情绪真实性**:依赖小爪输出 `[mood:]`,需在线验证 harness 是否稳定带标记。
- 服务端处理是**接收循环内同步阻塞**(STT→harness→TTS),长推理期间不读 ping;固件因此**关闭了 heartbeat**。
  后续可改异步任务 + 共享 sender 提升健壮性。
- 舵机 nod/shake 幅度/速度、抬头角度可按手感再调(`update_hardware` 内)。
- server 鉴权目前放宽,上线前需收紧。

## 13. 提交约定

- 不提交:`VOLC_API_KEY`、`target/`、`.pio/`、`*.bak`、`.playwright-mcp/`、`.DS_Store`。
- 可提交:`config/default.toml`(resource id/音色/人设)、`lib/FTServo/`(vendored)、`patches/`(必须)。
