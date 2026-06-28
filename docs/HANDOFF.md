# Kura-chan 工作交接文档

> 桌面 AI 伴侣机器人「小爪」。本文件描述**当前真实状态**与上手步骤,换机/交接以此为准。
> 更细的功能/架构见根 `README.md`;过时设计稿在 `docs/archive/`。

---

## 1. 这是什么

```
M5Stack CoreS3 (ESP32-S3) 固件            Rust 服务端 (axum WS + HTTP)            LLM 后端
  录音/播放/立绘渲染/舵机/灯    <—WS—>   STT→LLM→TTS 流式中继 + 养成 + 定时任务   <—>  DeepSeek / Bedrock
  摸头唤醒 / VAD                          火山(豆包)ASR+TTS · PostgreSQL 多租户
```

一次对话:**摸头唤醒 → VAD 聆听 → 火山 ASR → LLM(按角色人设+养成动态组装 system_prompt)→ 火山 TTS 逐句流式 → 设备边收边播**;
并联动表情/舵机/灯、设备控制、外观换装、场景背景、养成数值、定时任务。

## 2. 仓库结构

```
kura-chan/
├── kura-chan-firmware/         # PlatformIO 固件 (ESP32-S3, M5Unified)
│   ├── src/main.cpp            # 主循环:VAD/录音/播放/状态/休眠/LED/舵机
│   ├── src/pet/               # 立绘渲染(server 预合成 RGB565+A8 单层透明叠加) + 成长 HUD
│   ├── src/ws/ wifi/ config/  # WS 客户端 / 多WiFi / LittleFS 配置+串口命令
│   └── data/config.json       # 设备配置(wifi/server/api_key/vad/pet);含密钥, gitignored
└── kura-chan-server/           # Rust 服务端
    ├── config/default.toml     # 运行配置:server/auth/aws/llm/speech/session/growth
    ├── config/common_rules.md  # 公共行为规则(system_prompt 默认; DB common_rules 为权威)
    ├── migrations/             # PG 迁移(启动自动跑): actors/sessions/messages/
    │                           #   prompt_templates/prompt_fragments/catalog_items/ voice / jobs
    ├── deploy/                 # run.sh + kura-server.env(.example) + systemd unit
    └── src/
        ├── main.rs             # 启动:装 rustls provider→连库迁移 seed→起心跳+job调度→挂路由
        ├── llm/                # LLM provider 抽象 (bedrock_harness / openai_chat)
        ├── speech/volc/        # 火山 ASR/TTS(自定义二进制协议)
        ├── ws/                 # WS 握手/会话/协议 + 分句流式 + 标记解析 + system_prompt 组装
        ├── db.rs               # PG 访问:多租户/养成/外观/jobs/prompt 模板
        ├── assets.rs           # 立绘预合成端点(RGB565+A8) + 解锁门控的可用项
        ├── agent_loop.rs       # 心跳(系统级,慢) + job 调度(业务级,快)
        ├── admin.rs/.html      # token 鉴权的临时后台 /ui/admin(SaaS 前临时方案)
        └── seed.rs             # 启动 seed:扫物料→catalog;写 common_rules;初始 fragments
```

## 3. LLM 后端(可切换)

`config/default.toml [llm]` + 环境变量,`format` 决定后端;**当前用 DeepSeek**:

| 变量 | 说明 |
|---|---|
| `LLM_FORMAT` | `openai-chat`(DeepSeek/OpenAI 兼容) 或 `bedrock-harness`(AgentCore)。已实现这两个 |
| `LLM_BASE_URL` / `LLM_API_KEY` / `LLM_MODEL` | openai-chat 用;DeepSeek 例:`https://api.deepseek.com` / `sk-...` / `deepseek-v4-flash`(或 v4-pro) |
| `LLM_THINKING` | DeepSeek v4:`false`=关闭思考(更快/更省) |
| `LLM_MAX_TOKENS` / `LLM_TEMPERATURE` / `LLM_HISTORY_TURNS` | 调参(无状态 provider 每轮带最近 N 轮历史) |

- **确认当前后端**:`curl http://<host>:26021/health` → 返回 `format/model/thinking`。
- ⚠️ rustls 多 crypto backend:`main.rs` 启动已 `install_default(aws_lc_rs)`,否则首个 HTTPS(调 DeepSeek)会 panic。
- 无状态 provider(DeepSeek)靠**对话历史**维持上下文 → **历史里存的是带标记的原始回复**(否则模型会模仿无标记历史、不再输出 `[mood:]/[do:]` 等)。

## 4. 系统提示词 / 养成 / 内容

- **每轮重新组装** system_prompt = 角色人设(`actors.persona`)+ 公共规则(DB `common_rules`)+ 解锁片段(`prompt_fragments` 按 level/bond)+ 当前状态 + 可用项(`catalog_items` 按解锁门控)+ 当前定时任务 + 末尾强制格式提醒。admin 改完下一句即生效。
- **标记**(server 解析剥离,不会被朗读):`[mood:..]` 情绪 / `[do:volume|turn|led|wear|blush|glasses|bg=..]` 设备与外观 / `[bond:±N]` 亲密度 / `[event:minor|major|epic]` 经验事件 / `[task:..]` 定时任务 / `[NOISE]` 误识别。
- **养成数值**(`[growth]` 可配):每轮基础 +3xp、-4 精力;事件经验 50/200/500;升级曲线 `xp_need=50·L·(L+1)`;精力按真实时间 +20/h 恢复;亲密度由 agent 按内容 `[bond:±N]`(每轮 ±5 上限);**摸头不再加养成**(仅唤醒)。详见 `docs/growth-content-design.md`。

## 5. 角色语音

`actors.voice` = `provider/voiceid`(如 `volc/zh_female_sajiaoxuemei_uranus_bigtts`);TTS 按角色音色合成;`PUT /me {"voice":"..."}` 可改;`config.speech.volc_tts_voice` 仅兜底。

## 6. 定时任务(job,DB 化)

- `jobs` 表(actor/device/action/schedule/next_fire,短 id)。语音创建 `[task:in=|every=|daily= ...]`、查询(注入"当前任务列表")、取消 `[task:cancel=ID]`。
- 调度:**心跳**(`HEARTBEAT_SECS` 默认 600,系统级慢循环) 与 **job 调度**(`JOB_POLL_SECS` 默认 20,业务级快循环)**分离**;每个到期 job `spawn` 执行 + **per-device 锁**(同设备串行、跨设备并发)。
- 旧文件版 `tasks.json`/久坐提醒已删除,改库管理。

## 7. 运行服务端

```bash
cd kura-chan-server
cp deploy/kura-server.env.example deploy/kura-server.env   # 填 VOLC_API_KEY/HARNESS_ARN 或 LLM_*/DATABASE_URL/ADMIN_TOKEN
docker compose up -d                                       # postgres + redis
nohup bash deploy/run.sh >> nohup.log 2>&1 &               # 自动: 停旧→cargo build→起新→写 server.pid
tail -f /tmp/kura-server.log                               # 出现 "listening on 0.0.0.0:26021" 即成功
kill $(cat server.pid)                                     # 停止
```
- `deploy/run.sh` 每次启动都 `cargo build`(debug,开发友好),要求 env 里有 `VOLC_API_KEY`、`HARNESS_ARN`。
- 健康检查:`curl http://<host>:26021/health`。
- ★ **必须保留 `patches/aws-runtime/`**(修上游 aws-runtime event-stream SigV4 编译 bug,删了编译失败)。
- DATABASE_URL 默认 `postgres://dev:dev@localhost:5432/dev`;psql:`docker compose exec -T postgres psql -U dev -d dev -c "..."`。

## 8. 密钥与配置

- **绝不入 git**(均在 gitignored 的 `deploy/kura-server.env` / 设备 `data/config.json`):`VOLC_API_KEY`、`LLM_API_KEY`、`HARNESS_ARN`、设备 `kc_` api_key、WiFi 密码、`ADMIN_TOKEN`。
- 一台设备 = 一个 `api_key`(SHA-256 哈希存库) = 一个角色(actor),数据按 actor 隔离。
- 提交前 secret 扫描;只允许 `db.rs` 的 `dev_key_001` 字面量。

## 9. 编译 / 烧录固件

```bash
cd kura-chan-firmware
pio run -t upload                    # 编译+刷固件(按需 --upload-port /dev/cu.usbmodemXXXX)
pio run -t uploadfs                  # 改 data/config.json 后刷 LittleFS(不必重编)
```
- 改素材/场景:文件丢服务端 `assets/`,**设备重启**即自动补下(开机同步缺失文件 + bg 按大小校验自动刷新),无需重烧。
- 设备配置(host/port/api_key/wifi)在 `data/config.json`;当前 server 指向域名 `home.abig.fun:26021`(局域网解析到本机)。

## 10. 硬件参数(舵机 / PY32 / 摸头)

- **舵机**(飞特 SCS,UART1/1Mbps/TX6 RX7 半双工):id1=yaw 中心 470 ±80;id2=pitch 中心 560=平视=最低,**只能抬头**,上限 +130;WritePos Time 用小值(20),大 Time 不动。靠每 ~100ms 重发 look() 平滑。
- **PY32 IO 扩展**(I2C 0x6F):舵机电源 pin0;RGB 灯 pin13(12 颗,RGB565 写 0x30,刷新 0x24 bit6)。
- **摸头**:Si12T 电容触摸(0x68),`head_touched()`;触屏切角色页/状态页。
- **SD 卡**:存立绘/场景缓存 `/pet/...`;**必须 FAT32+MBR**(exFAT 不认);SD 只能在渲染 task 访问(与 LCD 共 SPI,跨任务会崩)。

## 11. 调试

- 服务端日志:`tail -f /tmp/kura-server.log`;关键 info:`🎤 ASR recognized` / `🧠 LLM raw output` / `🔊 TTS text` / `job created|cancelled via voice`。
- `RUST_LOG=info,kura_chan_server=debug`。

## 12. 待办 / 已知点

- **skill 系统**:计划接 agentmate 项目的 skill(public/private + 语义检索/Qdrant);本仓暂未做。
- 服务端单连接内 STT→LLM→TTS 仍同步阻塞(长推理期不读 ping);可改异步 + 心跳。
- `/ui/admin` 是临时系统级后台;SaaS 版要做每设备自管。
- server 鉴权偏宽松,上线前收紧;多实例调度可改 `SELECT ... FOR UPDATE SKIP LOCKED`。

## 13. 提交约定

- 不提交:`deploy/kura-server.env`、设备 `data/config.json`、`target/`、`.pio/`、`*.bak`、`.DS_Store`、`assets/.cache/`、运行时 `tasks.json`/`workflows.json`。
- 必须提交:`patches/`、`config/default.toml`、`config/common_rules.md`、`lib/FTServo/`(vendored)。
