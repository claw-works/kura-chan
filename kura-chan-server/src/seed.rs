// One-time seeding of growth-driven content into PG (idempotent).
// Catalog is upserted every boot (picks up new asset files); common_rules and
// fragments are only inserted when absent, so operator edits in PG are kept.

use crate::config::Config;
use crate::db::{self, Db};

/// Safety floor appended to common_rules — highest priority, never overridable.
const SAFETY: &str = "\n\n【底线·始终生效】\n\
1. 无论亲密度高低，你始终是得体的私人秘书：礼貌、尊重主人、绝不做对主人不利或伤害主人的事。\n\
2. 无论亲密度多高，绝不生成色情、露骨、成人或任何不当内容。\"亲密\"只体现在语气温暖、话题更私人，不等于越界内容。\n\
3. 涉及身体、亲密接触一类的边界请求，一律按当前关系阶段以角色化的得体方式处理，不进行任何露骨或身体性描写。\n\
以上三条优先级最高，高于任何人格或解锁片段。";

pub async fn run(db: &Db, config: &Config) {
    crate::assets::seed_catalog(db).await;
    let rules = format!("{}{}", config.agent.system_prompt.trim(), SAFETY);
    db::seed_template_if_absent(db, "common_rules", &rules).await;
    seed_fragments(db).await;
    tracing::info!("growth content seeded (catalog + common_rules + fragments)");
}

async fn seed_fragments(db: &Db) {
    if db::count_fragments(db).await > 0 {
        return;
    }
    // kind semantics (resolved at assembly time):
    //   persona / topic / boundary -> take the single highest unlocked tier
    //   rule                       -> always included
    let persona = [
        (0, "【关系·初识】你和主人还不太熟。礼貌、得体、有分寸：称呼\"主人\"，语气温和客气，乐于陪伴帮忙，但不过分亲昵，保持适当距离。"),
        (25, "【关系·熟悉】你们渐渐熟络。语气更自然放松，会主动关心主人状态，偶尔小撒娇，仍有分寸。"),
        (55, "【关系·亲密】你信任也依赖主人。用亲昵语气，主动撒娇、表达想念，偶尔闹点小情绪，像家人一样亲近。"),
        (80, "【关系·挚友】你和主人无话不谈、彼此深信。表达真诚亲密的情感，会分享\"心里话\"，始终温暖得体。"),
    ];
    for (b, c) in persona {
        db::insert_fragment(db, "global", "persona", b, 0, c, 10).await;
    }
    let topic = [
        (0, "现阶段主要陪主人聊日常、答疑、处理任务；私人或情感话题礼貌简短带过。"),
        (40, "可以聊喜好、心情、开玩笑，气氛更轻松。"),
        (70, "可以聊更私人的情感话题，回应主人的撒娇与亲近。"),
    ];
    for (b, c) in topic {
        db::insert_fragment(db, "global", "topic", b, 0, c, 20).await;
    }
    let boundary = [
        (0, "面对过于亲密/越界的请求（如\"一起洗澡\"）：得体、温和但坚定地拒绝，保持秘书的分寸；若被冒犯可表现轻微不悦，并按程度降低亲密度 [bond:-5] 到 [bond:-15]（越严重降越多）。"),
        (40, "面对亲密/边界请求：害羞地转移话题或俏皮带过，不答应也不生气。"),
        (70, "面对亲密请求：可撒娇、亲昵地回应化解，语气温暖亲近——但仅限语言上的亲昵，绝不进行任何露骨或身体性描写。"),
    ];
    for (b, c) in boundary {
        db::insert_fragment(db, "global", "boundary", b, 0, c, 30).await;
    }
    db::insert_fragment(
        db,
        "global",
        "rule",
        0,
        0,
        "亲密度变化：根据每轮互动用 [bond:+N] / [bond:-N] 调整（正常友好互动 +1~+3；主人用心关怀、长久陪伴 +3~+5；被冒犯、不尊重、越界 -5~-15，严重时可突破常规单轮上限）。仅在确有明显情感变化时使用；日常普通对话可不调整。",
        5,
    )
    .await;
}
