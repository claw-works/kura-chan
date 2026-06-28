# 换机器首次启动 Checklist

> 出差/换机时照此一步步走。详细背景见 `HANDOFF.md`。

## 一、随身携带(不在 git 里)
- [ ] `VOLC_API_KEY`(火山 API key)
- [ ] `HARNESS_ARN`(AgentCore harness ARN,如 `arn:aws:bedrock-agentcore:us-west-2:<acct>:harness/...`)
- [ ] AWS 凭证(access key / SSO 配置,region us-west-2,有权限调该 harness)

## 二、新机器装环境
- [ ] Rust(stable)+ `cargo`
- [ ] PlatformIO(`pio`),USB 串口驱动
- [ ] `aws configure`(或导出 AWS_ACCESS_KEY_ID / AWS_SECRET_ACCESS_KEY / AWS_REGION)

## 三、拉代码
```bash
git clone git@github.com:claw-works/kura-chan.git
cd kura-chan
```
- [ ] 确认 `kura-chan-server/patches/aws-runtime/` 存在(★ 缺了编译失败)

## 四、跑服务端
```bash
cd kura-chan-server
cargo build                       # 确认 Finished,无 error
VOLC_API_KEY=<key> HARNESS_ARN=<arn> RUST_LOG=info,kura_chan_server=debug \
  nohup ./target/debug/kura-chan-server > /tmp/kura-server.log 2>&1 &
tail -f /tmp/kura-server.log      # 看到 "listening on 0.0.0.0:26021"
```
- [ ] 启动日志无 "harness_arn is empty" 警告
- [ ] 记下新机器的局域网 IP:`ipconfig getifaddr en0`(或 `ifconfig`)

## 五、编译/烧录固件
```bash
cd ../kura-chan-firmware
pio run                                              # 编译通过
pio run -t upload --upload-port /dev/cu.usbmodemXXXX # 串口按实际
```

## 六、把设备指向新服务端 IP(关键!)
设备 NVS 存的是旧 IP,必须重指向。串口(115200)发:
```
server <新机器IP>     # 例 server 192.168.1.50
port 26021
config                # 核对
reboot
```
- [ ] 若换了 WiFi:`wifi add <ssid> <password>` 后 `reboot`
- [ ] 串口不通时:改 `src/config/config_store.cpp` 默认 `srv_host` → 重新烧录(需先工厂复位让默认生效)

## 七、联调验证
- [ ] 设备开机:头平滑回中、屏幕显示新 IP、WS:OK
- [ ] 摸头 → 蓝灯抬头(聆听)→ 说一句 → 思考 → 小爪语音回答
- [ ] "把灯调成蓝色" / "点点头" / "向左转头" / "音量调到 80%" / "电量还有多少"
- [ ] 服务端日志能看到 STT 文本、harness 调用、TTS 帧

## 八、UDP 遥测(可选,若要调 VAD/状态)
- [ ] 把固件 `dbg()` 目标 IP 改成新机器 IP(`src/main.cpp` 里 `192.168.31.249`),重烧
- [ ] 新机器监听 8089

## 排错速查
| 现象 | 可能原因 |
|------|----------|
| cargo build 报 aws-runtime 编译错 | `patches/` 丢失或 Cargo.toml patch 段被删 |
| 启动 harness_arn is empty | 没设 `HARNESS_ARN` 环境变量 |
| 设备 WS:off 连不上 | 设备 NVS 还指旧 IP(走第六步)/ 服务端没起 / 不同网段 |
| 调用 harness 403/无响应 | AWS 凭证/权限/region 不对 |
| 舵机不动 | 见 HANDOFF §8:WritePos Time 必须小值;pitch 不能低头 |
