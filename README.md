# Kura-chan（小爪）

一个语音陪伴型桌面宠物：**M5Stack CoreS3 硬件设备** + **Rust 服务端**。
拍一下头跟它说话，它用 AI 回答并用语音念出来，屏幕上是一只会呼吸、眨眼、换装、随关系成长的角色立绘。

---

## 1. 总览

```
┌────────────────────┐   WebSocket (ws://, Authorization: Bearer <api_key>)   ┌─────────────────────────┐
│  M5Stack CoreS3     │ ─────────────────────────────────────────────────────▶ │  kura-chan-server (Rust) │
│  (kura-chan-firmware)│                                                        │  axum + WebSocket        │
│                     │  上行: 录音 PCM 帧 / 状态 / 事件                          │                          │
│  - 拍头录音 (VAD)    │  下行: STT结果 / 文本 / TTS音频帧 / 控制 / 数值同步        │  ├─ STT/TTS (火山引擎)    │
│  - 立绘渲染/呼吸/表情 │ ◀───────────────────────────────────────────────────── │  ├─ AgentCore harness(LLM)│
│  - 触屏第二屏/时钟   │                                                        │  ├─ PostgreSQL (多租户)   │
│  - RGB LED / 舵机    │                                                        │  └─ 立绘合成 (image)      │
└────────────────────┘                                                        └─────────────────────────┘
```

- **设备**只做：采集语音、播放音频、渲染画面、驱动 LED/舵机；不含业务逻辑。
- **服务端**做：鉴权、STT→LLM→TTS 管线、人格/提示词、养成数值、立绘合成、定时任务。
- 一台设备 = 一个 `api_key` = 一个角色（actor），数据按 actor 隔离（多租户）。

---

## 2. 功能特性

**对话**
- 拍头开始录音，再拍一下发送；或说完停顿自动发送（自适应 VAD，可调灵敏度）。
- STT（火山）→ LLM（可插拔：DeepSeek / AWS Bedrock AgentCore，见 `[llm]` 配置）→ TTS（火山），逐句合成流式播放。

**角色与画面**
- 角色立绘由**服务端预合成**（身体/发型/服装/配饰分层 → 合成 → 缩放 → `RGB565+A8`），设备作为单层透明 sprite 手动 alpha 叠加到固定背景，呼吸只动角色、背景不动（无撕裂）。
- 表情（眨眼/说话/喜怒哀乐）、一次性反应（蹦跳等）、场景背景切换。
- 右上角 NTP 时钟、触屏切换的第二屏状态页（WiFi/IP/WS/电量/音量/成长）。

**养成（server 权威）**
- `level`（经验）/`bond`（亲密度）/`energy`（精力）。
- **level 解锁视觉物料**（发型/服装/配饰按 `min_level` 门槛）。
- **bond 解锁精神层**（人格深化 / 话题开放度 / 边界请求处理，按 `min_bond` 分档）。
- agent 用 `[bond:±N]` 标记按互动动态增减亲密度（越界惩罚可突破常规上限）。

**提示词分层（全部在 PostgreSQL，可热改）**
- `prompt_templates`（公共规则 + 安全底线）/ `prompt_fragments`（分级解锁片段）/ `catalog_items`（物料解锁门槛）。
- system_prompt **每轮对话重新组装**，admin 改完下一句即生效，无需设备重连。
- 提供 token 鉴权的 **admin 网页 `/ui/admin`** 在线编辑。

**主动能力**
- 定时提醒 / workflow（agent 用 `[task:...]` 标记创建，server 调度），到点经 TTS 主动推送到在线设备。

详细数值/内容设计见 [`docs/growth-content-design.md`](docs/growth-content-design.md)。

---

## 3. 仓库结构

```
kura-chan/
├─ docs/                       设计文档
├─ kura-chan-firmware/         设备固件 (PlatformIO / ESP32-S3)
│  ├─ platformio.ini
│  ├─ data/config.json         设备配置(wifi/server/api_key/vad/pet)，刷入 LittleFS；含密钥, gitignored
│  ├─ data/config.example.json 配置模板
│  └─ src/
│     ├─ main.cpp              主循环：VAD/录音/播放/状态/休眠/LED/舵机
│     ├─ pet/                  立绘渲染(KRA1 解码 + alpha 合成 + 呼吸/表情) + 成长 HUD
│     ├─ ws/                   WebSocket 客户端
│     ├─ wifi/                 多 WiFi 管理
│     └─ config/               配置存储(LittleFS) + 串口命令
└─ kura-chan-server/           服务端 (Rust / axum)
   ├─ Cargo.toml
   ├─ config/default.toml      公共配置：system_prompt / speech / session / growth
   ├─ migrations/              PostgreSQL 迁移(启动自动执行)
   ├─ assets/                  立绘分层 PNG (girl/ boy/) 与场景背景 (bg/)
   ├─ deploy/                  部署脚本 + env 模板 + systemd unit
   └─ src/
      ├─ main.rs               启动：连库/迁移/seed/起 agent_loop/挂路由
      ├─ router.rs             路由
      ├─ ws/                   WebSocket 处理 + 协议 + 会话状态
      ├─ harness/              AgentCore LLM 调用
      ├─ speech/               STT/TTS (volc / mock)
      ├─ db.rs                 PostgreSQL 访问 + 多租户 + 养成
      ├─ assets.rs             立绘合成端点 (RGB565+A8) + 目录扫描
      ├─ seed.rs               启动 seed：扫物料 / 公共规则 / 初始片段
      ├─ admin.rs + admin.html token 鉴权的后台编辑页 /ui/admin
      ├─ agent_loop.rs         心跳 + 定时任务调度 + 主动推送
      ├─ tasks.rs/workflows.rs 定时任务与 workflow
      └─ api.rs                注册/资料/任务/workflow 等 HTTP 接口
```

---

## 4. 服务端

### 4.1 依赖
- Rust（stable）、PostgreSQL。
- 运行期环境：AWS 凭证（调用 Bedrock AgentCore；EC2 用实例角色）、火山引擎 API Key。
- 注意 `patches/aws-runtime/`（vendored 修复），克隆时务必带上，否则编译失败。

### 4.2 配置

**环境变量**（`deploy/kura-server.env`，从 `kura-server.env.example` 复制）：

| 变量 | 说明 |
|---|---|
| `VOLC_API_KEY` | 火山 STT/TTS 密钥（必填，否则降级为 mock） |
| `LLM_FORMAT` / `LLM_BASE_URL` / `LLM_API_KEY` / `LLM_MODEL` | LLM 后端选择与凭证（DeepSeek 用 openai-chat）|
| `HARNESS_ARN` | AgentCore harness ARN（仅 `LLM_FORMAT=bedrock-harness` 时必填）|
| `DATABASE_URL` | PostgreSQL 连接串，如 `postgres://user:pass@localhost:5432/kura` |
| `AWS_REGION` | 默认 `us-west-2` |
| `KURA_SERVER_PORT` | 监听端口（映射到 `config.server.port`） |
| `ADMIN_TOKEN` | `/ui/admin` 的访问 token；**留空则 admin 接口全部 403** |
| `RUST_LOG` | 日志级别 |

**通用配置** `config/default.toml`：`llm`（后端格式/调参）、`speech`（STT/TTS provider 与音色）、`session`、`growth`（养成数值曲线）。公共行为规则默认在 `config/common_rules.md`（DB `common_rules` 为权威）。

### 4.3 数据库
启动时 `sqlx` 自动跑 `migrations/`，并 seed：扫 `assets/` 填 `catalog_items`、写入 `common_rules`、插入初始 `prompt_fragments`（仅当表为空，不覆盖已有编辑）。

主要表：`actors`（角色/api_key 哈希/数值/外观）、`sessions`、`messages`、`prompt_templates`、`prompt_fragments`、`catalog_items`。

### 4.4 部署与运行

在**设备能连到**的机器上跑（公网 EC2 / 家里带 DDNS 的机器等）。详见 [`kura-chan-server/deploy/README.md`](kura-chan-server/deploy/README.md)。

```bash
cd kura-chan-server
cp deploy/kura-server.env.example deploy/kura-server.env   # 填 VOLC_API_KEY / HARNESS_ARN / DATABASE_URL / ADMIN_TOKEN

# 后台启动（自动: 停旧实例 → cargo build → 起新实例 → 写 server.pid）
nohup bash deploy/run.sh >> nohup.log 2>&1 &

tail -f nohup.log          # 看日志, 出现 "listening on 0.0.0.0:<port>" 即成功
kill $(cat server.pid)     # 停止
```
> `deploy/run.sh` 目前用 **debug** 二进制并每次增量编译（开发友好）；要优化版改用 `cargo build --release` + `target/release`。
> 开机自启见 `deploy/kura-chan-server.service`。

健康检查：`curl http://<host>:<port>/health` → `ok`

### 4.5 后台管理 `/ui/admin`
浏览器打开 `http://<host>:<port>/ui/admin`，填入 `ADMIN_TOKEN`，即可在线改：
- **数值**：actor 的 level/bond/energy
- **提示词片段**：`prompt_fragments`（含解锁阈值）
- **公共规则**：`prompt_templates`
- **物料解锁**：`catalog_items` 的 minLevel/minBond

改完即时生效（system_prompt 每轮重组装），无需设备重连。

---

## 5. 固件

### 5.1 硬件与依赖
- **M5Stack CoreS3**（ESP32-S3），TF 卡（存立绘缓存 `/pet/...`）。
- **PlatformIO**；库：M5Unified、M5GFX、links2004/WebSockets、ArduinoJson。

### 5.2 配置 `data/config.json`
从 `data/config.example.json` 复制后填写（含密钥，已 gitignored，勿提交）：
```jsonc
{
  "wifi": [ { "ssid": "...", "pass": "..." } ],     // 可多个，依次尝试
  "server": { "host": "<服务端域名/IP>", "port": <端口>, "path": "/ws/device" },
  "auth": { "api_key": "kc_...", "device_id": "KURA_CHAN_001" },
  "vad": { "rise_factor": 2.0, "keep_factor": 1.4, "min_margin": 250,
           "end_silence_ms": 1500, "no_speech_ms": 6000, "min_run": 3 },
  "audio": { "prebuffer_ms": 4000 },
  "pet": { "character": "mixue" }
}
```
VAD 调参：嘈杂环境调大 `min_margin`；想说完更晚自动发就调大 `end_silence_ms`。

### 5.3 编译与刷写
设备连 USB：
```bash
cd kura-chan-firmware
pio run -e kura-chan -t upload                 # 编译并刷固件
pio run -e kura-chan -t uploadfs               # 刷 data/ 到 LittleFS(改 config.json 后用; 不必重编)
pio device monitor -b 115200                   # 串口日志
```
> 改了 `config.json`（wifi/server/VAD 等）只需 `uploadfs`，无需重刷固件。

### 5.4 设备交互
- **拍头**开始录音 → 说话 → 再拍头发送；或说完停顿（`end_silence_ms`）自动发送。
- **触摸屏幕**：切换"角色页 ↔ 状态页"。
- 长时间无交互（默认 2 分钟）进入休眠，拍头唤醒。
- 串口命令：`wifi add/list/clear`、`server <ip>`、`port <n>`、`config`、`reboot` 等。

---

## 6. 工作原理（一次对话）

1. 设备拍头录音 → WebSocket 上行 PCM 帧。
2. server VAD/收尾后做 STT → 得到文本。
3. 取 actor 当前数值，从 PG **每轮重新组装** system_prompt（人格 + 公共规则 + 解锁片段 + 关系状态 + 可用物料）。
4. 调 LLM（DeepSeek / Bedrock，按 `LLM_FORMAT`）流式生成回复；解析其中标记：
   - `[mood:...]` 表情、`[do:wear|bg|blush|glasses=...]` 外观、`[task:...]` 定时、`[bond:±N]` 亲密度。
5. 逐句 TTS 合成 → 下行音频帧；设备流式播放、口型/表情联动。
6. 结算养成（xp/bond/energy），`Sync` 下发最新数值与外观，设备更新 HUD 与立绘。

---

## 7. 运维与安全

- **鉴权**：设备用 `Authorization: Bearer <api_key>`，库里只存 SHA-256 哈希（明文不落库）；admin 用独立 `ADMIN_TOKEN`。
- **时间**：设备连网后 NTP 同步北京时间（时钟显示用）。
- **作服务器的机器别休眠**（macOS：`sudo pmset -a disablesleep 1`），否则设备连不上。
- ⚠️ **安全**：不要把数据库端口（5432/6379/27017）暴露公网；用 DMZ/端口转发时只放行必要端口，数据库绑 `127.0.0.1`，用强密码。admin 页面是高权限接口，务必设强 `ADMIN_TOKEN`、勿公网弱口令。

---

## 8. 文档

**当前**
- [数值驱动内容设计](docs/growth-content-design.md) — level/bond 驱动视觉与人格
- [设备控制机制](docs/skill-device-control.md) — 对话内标记控制硬件 / 感知状态
- [服务端部署](kura-chan-server/deploy/README.md)

**运维 / 交接**
- [工作交接 / 当前状态](docs/HANDOFF.md)
- [换机首次启动 Checklist](docs/CHECKLIST-new-machine.md)

**历史存档**（过时设计稿 + AI 协作产物，仅参考）：[`docs/archive/`](docs/archive/)
