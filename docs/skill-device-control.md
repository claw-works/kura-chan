# 小爪设备控制 Skill

让小爪(桌面伴侣机器人)能在对话中控制自身硬件、并感知自身状态。
机制:大模型在回复文本里内联**标记**,服务端解析后剥离标记、把指令下发设备执行,设备状态则由设备上报、服务端注入到每轮上下文。

不依赖 harness 的 function-calling,纯文本流即可工作。可把下面「提示词片段」配置到 server 的 `[agent].system_prompt`(当前做法)或远程 harness 的 system prompt。

---

## 1. 提示词片段(粘贴到 system prompt)

```
情绪标记:在每句回复的最前面,根据心情加一个情绪标记,格式 [mood:xxx],xxx 取值之一:
happy / sad / angry / surprised / love / confused / neutral。
例:[mood:happy]嘿嘿,小爪在呢!

设备控制:当用户要你调节自身部件、或你想用动作表达情绪时,在回复里插入控制标记
(标记会被系统执行并从朗读中去掉),你仍要用一句话自然回应:
- 调音量:[do:volume=N]   (N 为 0-100 百分比)
- 头部动作:[do:turn=left] 左转 / [do:turn=right] 右转 / [do:turn=up] 抬头 /
           [do:turn=center] 回正 / [do:turn=nod] 点头(嗯/好) / [do:turn=shake] 摇头(不/不要) /
           [do:turn=auto] 恢复自动姿态
           注意:头部结构【只能抬头,不能低头】,用户让低头时用点头/抬头代替并俏皮说明。
- 改灯色:[do:led=COLOR]  (COLOR: red/green/blue/white/yellow/orange/purple/pink/cyan/off;
          恢复自动用 [do:led=auto])
例:用户"把灯调成蓝色" -> [mood:happy]好嘞,小爪变蓝色啦![do:led=blue]
例:用户"点点头"       -> [mood:happy]嗯嗯![do:turn=nod]

设备状态:每轮用户消息前可能出现以 "[设备状态]" 开头的一行系统信息(如电量、音量),
仅供回答设备相关问题时参考,不要主动复述,也不要当成用户说的话。
用户问"电量还有多少""音量多大"时,用其中数据自然回答。
```

---

## 2. 标记语法(模型 → 服务端)

| 标记 | 含义 | 示例 |
|------|------|------|
| `[mood:xxx]` | 情绪,驱动表情+灯色+姿态 | `[mood:love]` |
| `[do:volume=N]` | 扬声器音量,N=0~100 | `[do:volume=80]` |
| `[do:turn=DIR]` | 头部动作 left/right/up/center/nod/shake/auto | `[do:turn=nod]` |
| `[do:led=COLOR]` | RGB 灯颜色或 auto | `[do:led=blue]` |
| `[NOISE]` | 无意义输入,触发"没听清"兜底 | `[NOISE]` |

- 标记可出现在文本任意位置;服务端按出现顺序解析,**剥离后再 TTS**,所以不会被读出来。
- 一条回复可包含多个标记。`[NOISE]` 单独使用,不加情绪标记。
- 头部 pitch 只能抬头,不能低头(机械限位);`down` 会被当作回正/平视处理。

## 3. 控制消息(服务端 → 设备,WebSocket JSON)

```json
{ "type": "control", "action": "volume", "value": 80 }
{ "type": "control", "action": "led", "color": "blue" }   // 或 "auto"/"off"
{ "type": "control", "action": "turn", "dir": "nod" }      // left/right/up/down/center/nod/shake/auto
```

设备侧执行:
- volume → `M5.Speaker.setVolume(value*255/100)`,并记录当前音量上报。
- led → 手动颜色覆盖(休眠时仍灭;`auto` 恢复随状态自动变色)。
- turn → left≈-20°/right≈+20°(yaw),up≈+25°(pitch,抬头),center 回正平视,
  nod/shake 为一次性手势(约1.3秒),auto 释放覆盖。down 因底座限位按平视处理。

## 4. 状态消息(设备 → 服务端,WebSocket JSON)

设备在连接后、之后每 30 秒上报:

```json
{ "type": "status", "battery": 85, "charging": true, "volume": 78 }
```

服务端缓存最新值,在每轮发给 agent 的用户消息前注入一行:

```
[设备状态] 电量85%(充电中) 音量78%
<用户实际说的话>
```

## 5. 能力清单(hello.capabilities)

`servo` · `led` · `face` · `battery`

## 6. 待办 / 后续

- 摄像头:`你现在看到了什么?` —— 需 harness 支持图像输入,当前未实现。
  规划:设备抓帧 → 服务端 → 多模态 harness;或定义为 function-calling 工具。
- 若后续 harness 支持 function-calling,可把上述 `[do:...]` 升级为正式工具调用,
  服务端解析 tool_call 事件下发设备并回 tool_result。
