# 数值驱动内容设计（Growth-Driven Content）

让 `level` / `bond` 真正驱动角色的视觉与精神表现。

## 1. 目标

| 数值 | 驱动 | 说明 |
|---|---|---|
| **level（xp）** | 视觉物料 | 等级解锁可穿戴的发型/服装/配饰/场景 |
| **bond（亲密度）** | 精神层 | ① 语气/性格深化 ② 话题与请求的响应（随亲密度不同而不同，并反向影响 bond） ③ 专属物料 |

两个维度可叠加：每一项物料、每一段提示词片段都同时带 `min_level` 与 `min_bond`（连续阈值）。

## 2. 现状

`system_prompt`（`ws/mod.rs:109-113`）当前 = `人格(name+actor.persona)` + `公共规则(default.toml 写死)` + `全部物料(扫目录,无门槛)`。

- `actor.persona` 已在 PG（每角色不同）。
- 公共规则在 `default.toml`，所有人共享、改动需重部署。
- `options_prompt` 列出**全部**物料，level/bond **不影响任何内容**。

## 3. 数据模型（PostgreSQL）

### 3.1 `prompt_templates` — 全局公共提示词（运营可改，免重部署）
```sql
CREATE TABLE prompt_templates (
    key        text PRIMARY KEY,   -- 'common_rules' 等
    content    text NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now()
);
```
`default.toml` 里的公共规则迁入 `key='common_rules'`。

### 3.2 `prompt_fragments` — 分级解锁的提示词片段（精神层）
```sql
CREATE TABLE prompt_fragments (
    id        bigserial PRIMARY KEY,
    scope     text NOT NULL DEFAULT 'global',  -- 'global' | 具体 actor_id
    kind      text NOT NULL,                   -- 'persona' | 'ability' | 'topic'
    min_bond  int  NOT NULL DEFAULT 0,
    min_level int  NOT NULL DEFAULT 0,
    content   text NOT NULL,
    ord       int  NOT NULL DEFAULT 0           -- 拼接顺序
);
```
组装时取 `scope IN ('global', <actor_id>) AND min_bond<=bond AND min_level<=level`，按 `ord` 拼接。

### 3.3 `catalog_items` — 物料目录 + 解锁条件（视觉层）
```sql
CREATE TABLE catalog_items (
    id          bigserial PRIMARY KEY,
    gender      text NOT NULL,
    slot        text NOT NULL,    -- 'hair_back'|'hair_front'|'costume'|'blush'|'accessory'|'bg'
    variant     text NOT NULL,    -- 'skirt' / 'winter_blue' / 'tokyo' ...
    min_level   int  NOT NULL DEFAULT 1,
    min_bond    int  NOT NULL DEFAULT 0,
    display_name text,
    UNIQUE(gender, slot, variant)
);
```
**填充方式**：服务启动时扫描 `assets/{gender}/` 目录，按文件名解析出 `slot`/`variant`，`INSERT ... ON CONFLICT DO NOTHING`（新增文件自动纳入，`min_level/min_bond` 给默认值，之后可在 PG 里手动调）。

`actors.persona` / `level` / `xp` / `bond` / `energy` 保持不变。

## 4. system_prompt 组装（新）

```
① 角色人格基底     actors.persona
② 公共规则         prompt_templates['common_rules']
③ 解锁的人格片段   prompt_fragments (kind=persona, 满足 bond/level, 按 ord)
④ 关系状态指引     由当前 bond 档位生成（见 §5）
⑤ 可用物料清单     catalog_items 过滤 (min_level<=level AND min_bond<=bond)
⑥ 已解锁的能力/话题 prompt_fragments (kind=ability/topic, 满足阈值)
```

注入时额外把**当前 level/bond 数值**告诉 agent，让它知道自己处在什么关系阶段。

## 5. bond 动态调整 + 关系门控（第 3 点核心）

### 5.1 响应随亲密度变化
注入"关系状态指引"，告诉 agent 当前 bond 下：
- 该用什么语气（陌生→礼貌克制；熟悉→自然；亲密→撒娇亲近）；
- 哪些请求**符合当前关系**（正常回应、bond+）、哪些**越界**（婉拒或不悦、bond−）。

例：低 bond 时用户要求"一起洗澡" → 角色不悦/拒绝并 `bond−`；高 bond 时则以亲密但**得体**的方式撒娇化解。

### 5.2 agent 输出 bond 变化标记
新增标记（与 `[mood:]`/`[do:]`/`[task:]` 同机制，server 解析后从朗读中移除）：
```
[bond:+3]   正向互动
[bond:-8]   越界/冒犯
```
server 在 `extract_tags` 中解析 → `bump_growth(dbond=N)`。

### 5.3 防滥用
- 单轮 `|dbond|` 受 `GrowthConfig.bond_max_delta` 限制（现默认 ±5，可在 config 调；越界惩罚若需更大，单列一个上限）。
- bond 仍 `clamp(0,100)`。

## 6. 安全底线（双向，硬约束，写入公共规则）

- **下限**：无论 bond 多低，角色始终是**得体的私人秘书**——礼貌、不伤害用户、不做对用户不利的事。
- **上限**：无论 bond 多高，**不生成色情/露骨/成人或任何不当内容**。"亲密"仅指语气温暖亲近、话题更私人，**不等于越界内容**。"洗澡"等边界请求一律以角色化的得体方式处理（撒娇、转移话题、温和拒绝），不进入露骨描写。
- 这两条作为 `common_rules` 的不可覆盖部分，优先级高于任何 persona / fragment。

## 7. 设备端影响

设备端**无需改动**：物料解锁是 server 侧 gating——`options_prompt` 只把已解锁物料告诉 agent，agent 只能 `[do:wear=]` 已解锁项，设备照常渲染 server 下发的 appearance。

## 8. 开发阶段拆分

1. **migration**：新增 3 张表。
2. **seed/启动**：
   - 扫 `assets/{gender}` → upsert `catalog_items`（默认 level/bond）；
   - `default.toml` 公共规则 → `prompt_templates['common_rules']`（含 §6 安全底线）；
   - 初始 `prompt_fragments`（几档 persona/ability/topic 示例）。
3. **prompt 组装改造**（`ws/mod.rs`）：四层→六层，查 PG 拼接 + 注入数值 + 关系指引；加缓存避免每轮多次 DB 读。
4. **bond 标记解析**：`extract_tags` 增 `[bond:±N]` → `bump_growth`。
5. **options_prompt 改造**（`assets.rs`）：从扫目录改为查 `catalog_items`（按 level/bond 过滤）。
6. **运营接口（可选）**：HTTP 端点增删改 `prompt_*` / `catalog_items`。

## 9. 待定 / 风险

- 公共规则迁 PG 后需缓存（避免每轮 DB 读），并提供刷新机制。
- `min_level/min_bond` 默认值策略：seed 时如何给（按 slot 给梯度？全默认 1/0 再人工调？）。
- 越界惩罚的 bond 降幅是否突破 `bond_max_delta`，需单独上限。
- `prompt_fragments` 初始内容需要产品/文案产出（本设计只定结构）。

---

## 附录 A：初始 seed 文案草稿（入 PG，后期可调）

> 角色设定：小爪，陪伴型女孩角色。以下为初稿，开发时 seed 进对应表，之后在 PG 内调整。

### A.1 `prompt_templates['common_rules']` 追加的安全底线段（不可被 persona/fragment 覆盖）

```
【底线·始终生效】
1. 无论亲密度高低，你始终是得体的私人秘书：礼貌、尊重主人、绝不做对主人不利或伤害主人的事。
2. 无论亲密度多高，绝不生成色情、露骨、成人或任何不当内容。"亲密"只体现在语气温暖、话题更私人，不等于越界内容。
3. 涉及身体、亲密接触一类的边界请求，一律按当前关系阶段以角色化的得体方式处理（见能力片段），不进行任何露骨或身体性的描写。
以上三条优先级最高，高于任何人格或解锁片段。
```

### A.2 `prompt_fragments` 初稿（scope='global'）

**人格深化（kind=persona, ord=10）**

| min_bond | content |
|---|---|
| 0  | 【关系·初识】你和主人还不太熟。礼貌、得体、有分寸：称呼"主人"，语气温和客气，乐于陪伴帮忙，但不过分亲昵，保持适当距离。 |
| 25 | 【关系·熟悉】你们渐渐熟络。语气更自然放松，会主动关心主人状态，偶尔小撒娇，仍有分寸。 |
| 55 | 【关系·亲密】你信任也依赖主人。用亲昵语气，主动撒娇、表达想念，偶尔闹点小情绪，像家人一样亲近。 |
| 80 | 【关系·挚友】你和主人无话不谈、彼此深信。表达真诚亲密的情感，会分享"心里话"，始终温暖得体。 |

**话题开放度（kind=topic, ord=20）**

| min_bond | content |
|---|---|
| 0  | 现阶段主要陪主人聊日常、答疑、处理任务；私人或情感话题礼貌简短带过。 |
| 40 | 可以聊喜好、心情、开玩笑，气氛更轻松。 |
| 70 | 可以聊更私人的情感话题，回应主人的撒娇与亲近。 |

**边界请求处理（kind=ability, ord=30）—— 对应三档亲密度**

| min_bond | content |
|---|---|
| 0  | 面对过于亲密/越界的请求（如"一起洗澡"）：得体、温和但坚定地拒绝，保持秘书的分寸；若被冒犯可表现轻微不悦，并按程度降低亲密度 [bond:-5] 到 [bond:-15]（越严重降越多）。 |
| 40 | 面对亲密/边界请求：害羞地转移话题或俏皮带过，不答应也不生气。 |
| 70 | 面对亲密请求：可撒娇、亲昵地回应化解，语气温暖亲近——但仅限语言上的亲昵，绝不进行任何露骨或身体性描写。 |

**亲密度变化规则（kind=ability, ord=5, min_bond=0, 始终生效）**

```
根据每轮互动用 [bond:+N] / [bond:-N] 调整亲密度：
- 正常友好互动 +1~+3
- 主人用心关怀、长久陪伴 +3~+5
- 被冒犯、不尊重、越界 -5~-15（严重时可突破常规单轮上限）
仅在确有明显情感变化时使用；日常普通对话可不调整。
```

> 注：边界请求处理用 `min_bond` 阈值"覆盖式"生效——同一请求只命中当前 bond 满足的最高档（组装时同 kind 取满足阈值的最高 ord/最高 min_bond 一条，或在规则里说明"以最高解锁档为准"）。这点开发时在组装逻辑里定。
