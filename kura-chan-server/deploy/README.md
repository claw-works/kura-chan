# 在 EC2 / 小电脑上部署 Kura-chan 服务端

把服务端跑在**设备能连到的机器**上。公司 Mac 因 pf + CrowdStrike + FortiDLP
会静默丢弃入站，不能当服务端。

当前部署目标：公网 EC2 `54.187.154.83`（Amazon Linux 2023 / Graviton aarch64），监听 **8866**。
固件已指向 `54.187.154.83:8866`（cfg_rev=2），服务端起来后设备会自动连上。

> 编译方式：**直接在目标机上原生编译**（Graviton 上 `cargo build` 几分钟搞定，
> 比从 macOS 交叉编译省心；交叉编译要处理 ring/aws-lc-sys 的 C 工具链）。

## EC2 (Amazon Linux 2023, aarch64) 部署步骤

1. 拉代码（务必含 `patches/aws-runtime/`，否则编译失败）：
   ```bash
   git clone git@github.com:claw-works/kura-chan.git
   cd kura-chan/kura-chan-server
   ```

2. AWS 凭证：优先用 **EC2 实例角色**（需有调用该 harness 的 Bedrock AgentCore 权限），
   没有则 `aws configure`。确认账号是 320236118172：
   ```bash
   aws sts get-caller-identity
   ```

3. 填环境变量：
   ```bash
   cp deploy/kura-server.env.example deploy/kura-server.env
   # 编辑：填 VOLC_API_KEY 和 HARNESS_ARN；KURA_SERVER_PORT=8866 已预置
   ```

4. 编译并运行：
   ```bash
   bash deploy/setup.sh      # 装 Rust + 编译依赖(dnf) + cargo build --release
   bash deploy/run.sh        # 日志出现 "listening on 0.0.0.0:8866" 即成功
   ```
   开机自启（可选）见 `deploy/kura-chan-server.service`。

## 验证

```bash
curl -i http://54.187.154.83:8866/ws/device   # 返回 400 = 服务端可达（在等 WS 升级）
```
确认 EC2 安全组放开 TCP 8866 入站。通了之后设备（在有外网的 WiFi 上）会自动连上。

> 注意：EC2 在 us-west-2（美国），火山 ASR/TTS 在国内，跨太平洋往返会增加语音延迟，
> 功能可用但偏慢；后续可换离用户更近的主机。
