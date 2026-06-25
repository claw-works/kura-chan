# 部署 Kura-chan 服务端

把服务端跑在**设备能连到的机器**上（公网 EC2、家里带 DDNS/端口转发的机器等）。
注意：装了 pf + 安全软件（如 CrowdStrike / FortiDLP）的公司机器可能静默丢弃入站，不适合当服务端。

> 编译方式：**直接在目标机上原生编译**（`cargo build` 几分钟搞定，比从 macOS 交叉
> 编译省心；交叉编译要处理 ring/aws-lc-sys 的 C 工具链）。

## 部署步骤

1. 拉代码（务必含 `patches/aws-runtime/`，否则编译失败）：
   ```bash
   git clone <仓库地址>
   cd kura-chan/kura-chan-server
   ```

2. AWS 凭证：优先用**实例角色**（需有调用该 harness 的 Bedrock AgentCore 权限），
   没有则 `aws configure`，并确认账号正确：
   ```bash
   aws sts get-caller-identity
   ```

3. PostgreSQL：准备一个库，连接串填到下一步的 `DATABASE_URL`（服务端启动时自动建表 + seed）。

4. 填环境变量：
   ```bash
   cp deploy/kura-server.env.example deploy/kura-server.env
   # 编辑：VOLC_API_KEY / HARNESS_ARN / DATABASE_URL / ADMIN_TOKEN，按需设 KURA_SERVER_PORT
   ```

5. 首次装环境（装 Rust + 系统编译依赖）：
   ```bash
   bash deploy/setup.sh
   ```

6. 启动（后台；自动停旧实例 → 编译 → 起新实例 → 写 server.pid）：
   ```bash
   nohup bash deploy/run.sh >> nohup.log 2>&1 &
   tail -f nohup.log          # 出现 "listening on 0.0.0.0:<port>" 即成功
   kill $(cat server.pid)     # 停止
   ```
   开机自启（可选）见 `deploy/kura-chan-server.service`。

## 验证

```bash
curl http://<host>:<port>/health           # 返回 ok
curl -i http://<host>:<port>/ws/device      # 返回 400 = 服务端可达（在等 WS 升级）
```
公网部署时，确认机器防火墙 / 安全组 / 路由器端口转发放开了 `<port>` 入站。通了之后设备（在可达网络上）会自动连上。

## 注意

- **地理延迟**：服务端离火山 ASR/TTS（国内）越远，语音往返越慢；主机尽量选离用户/火山近的地区。
- **别让机器休眠**（macOS：`sudo pmset -a disablesleep 1`），否则设备连不上。
- ⚠️ **安全**：数据库端口（5432 等）不要暴露公网；用 DMZ/端口转发时只放行必要端口、数据库绑 `127.0.0.1`、用强密码。`/ui/admin` 是高权限接口，`ADMIN_TOKEN` 设强口令、勿公网弱口令。
