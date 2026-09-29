# filesync

两层团队文件分发工具。上游发布相同内容给所有下游；下游先认领本机位置，再自动接收最新版。

**当前是可运行的开发基础版，不是全部需求已经交付的正式版本。** Rust + Tokio + Axum + Reqwest + SQLite + Ratatui；中心新增 Vue 3 Web 管理，节点新增 Tauri 2 桌面应用，后台仍独立运行。

图形使用流程与桌面构建见 [图形界面说明](docs/GUI.md)。

中心容器构建与启动见 [Docker 部署说明](docs/DOCKER.md)。

## 构建与测试

需要 Rust 工具链和本机 C 编译工具（用于随程序编译 SQLite）；用户运行构建产物不需安装 Rust 或数据库。

```sh
npm --prefix ui ci
npm --prefix ui run build
cargo build --release --locked
cargo test --locked
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
```

产物：macOS/Linux 为 `target/release/filesync`，Windows 为 `target/release/filesync.exe`。`rust-toolchain.toml` 固定工具链，`Cargo.lock` 固定依赖。

如果本机全局 Cargo 配置引用了不可用的旧镜像，macOS/Linux 可使用 `sh scripts/cargo-local.sh build --release --locked`（`test` / `fmt` / `clippy` 同理）。该脚本从隔离目录调用官方 registry，缓存仅放在项目 `.cache/cargo`，不修改全局配置或默认 Rust 工具链。

## 当前功能

- 中心 Web 登录、节点创建/重命名/删除、两层关系建立/删除、最近联系时间与分页接收结果；拒绝第三层和自连。删除节点会使原令牌失效，并保留电脑上已有文件。
- Tauri 桌面首次连接、原生文件/目录选择、发布与认领、保存位置管理、托盘；界面退出后后台继续运行。
- 定时扫描、SQLite 扫描缓存、SHA-256 校验、HTTP/HTTPS 流式上传/下载。
- 完整清单事务发布；读取失败/缺少结束标记时不误判删除。
- 下游待定时不下载，精确映射和前缀映射认领，未来同前缀文件自动接收。
- 更新先备份，保留最近 3 份备份；源文件撤下后本地文件保留。
- 中心按上游和路径保留最新及上一版，自动清理更旧的无引用内容；保护待发布上传和在途下载，下游仍只接收最新版。
- 多来源目标冲突检测；回执持久化与补报。
- 基础 TUI 列表和文本路径认领；本机带认证的控制 API；TUI 退出不结束 agent。
- JSON status/pending、配置校验、单次同步、后台数据目录单实例锁。

## 本机启动

中心启动后可在浏览器打开 `http://127.0.0.1:8787/` 管理节点和关系；以下 CLI 操作仍保留。桌面入口见 [GUI.md](docs/GUI.md)。

1. 将 `config/hub.example.toml` 复制成自己的配置，修改绝对路径。管理员令牌从 `FILESYNC_ADMIN_TOKEN` 环境变量读取，至少 32 字符，使用随机值。
2. `filesync --config /absolute/hub.toml hub`。局域网 HTTP 使用 `listen = "0.0.0.0:8787"` 和 `allow_http = true`；客户端填写 `http://中心IP:8787`，无需 CA。HTTP 不加密令牌和文件。未显式允许 HTTP 时，非回环监听仍要求配置 HTTPS 证书和私钥。
3. 创建节点：

```sh
filesync admin --hub http://127.0.0.1:8787 new-node upstream-A
filesync admin --hub http://127.0.0.1:8787 new-node downstream-B
filesync admin --hub http://127.0.0.1:8787 add-edge <A_ID> <B_ID>
```

创建节点返回 `{id, token}`，请将 token 保存在该节点进程环境变量中。HTTPS 自签 CA 可通过 `admin --ca-cert /absolute/ca.pem` 指定。

4. 分别参考 `config/agent.example.toml` 创建 A/B 的配置；使用各自节点 ID、令牌环境变量和数据目录。A 配置 `[[source]]`，B 开始时不配置映射。
5. 分别运行：

```sh
filesync --config /absolute/a.toml agent
filesync --config /absolute/b.toml agent
```

6. 在 B 打开另一个终端，使用相同的节点令牌环境变量：

```sh
filesync --config /absolute/b.toml
filesync --config /absolute/b.toml status
filesync --config /absolute/b.toml pending
filesync --config /absolute/b.toml claim --upstream <A_ID> --from 'quotes/' --to /absolute/received
```

`filesync` 无子命令进入 TUI；`m` 输入单文件绝对目标路径，`M` 输入当前文件父目录前缀的目标目录；`q` 只退出界面。当前尚未提供文件浏览器。`agent --once` 用于脚本和验证，不启动常驻控制 API。

## 内存验证

macOS/Linux 可运行以下基线（Python 3.11+、OpenSSL）：

```sh
python3 scripts/memory_probe.py --files 20000 --large-bytes 99000000
```

脚本只生成临时测试文件，启动本机 HTTPS 中心和真实 agent 子进程，以系统 `wait4` 报告的进程峰值 RSS 计量，生成 `.local/memory-report.json`。测试凭证与文件在临时目录，结束后清理，不读取用户文件。仅认领并下载其中的 99 MB 文件，2 万小文件仍保持待定。

**这个基线不等于完整 20 MB 验收**：它测量顺序扫描/传输的 `agent --once`，尚不覆盖常驻控制服务、并发扫描与传输、长期运行或三平台全部实测。性能报告见 `docs/VALIDATION.md`。

本机 macOS x86_64 已完成上述基线，最高峰值 **8.704 MB**；完整报告和限制见 [验证记录](docs/VALIDATION.md)。

TUI 的 Unix PTY 冒烟可运行 `python3 scripts/tui_smoke.py`，验证界面显示、认证查询和按 `q` 退出。

## 尚未完成

- 断点续传、传输暂停和限速；目前失败后重试整文件。
- 完整账号与一次性注册、独立的令牌轮换/吊销、准确在线状态和审计；现已支持删除节点并使其令牌失效、删边及最近联系时间。
- 完整目录浏览/发布 TUI、通配过滤、忽略、分页搜索、分发矩阵、中英文、系统通知。
- 大量小文件的批量传输、落盘与回执吞吐优化。
- 配置数秒热重载；目前每轮同步之间重新读取，错误配置沿用上一份有效配置。
- OS 服务注册、安装包、Windows/macOS 路径竞态与 Unicode 边界加固。
- 三平台完整业务和内存验收。CI 矩阵已经提供，但本地建好工作流不代表远端 CI 已运行。

这些缺口使当前版本适合本地开发验证，尚不适合正式分发业务文件。

## 目录

```text
src/main.rs       单程序 CLI 入口
src/hub.rs        中心 API / HTTPS / 授权 / 清单事务
src/agent.rs      同步循环 / 待定 / 认领 / 本机控制 API
src/scan.rs       流式扫描和增量缓存
src/transfer.rs   路径检查 / 哈希 / 原子替换 / 最近 3 份备份
src/store.rs      SQLite 数据模型
src/config.rs     TOML 与环境变量凭证
src/model.rs      共享协议结构
src/tui.rs        Ratatui 界面
tests/            文件安全与 HTTPS 端到端测试
scripts/          内存基线验证
ui/               Vue 3 界面、Tauri 2 桌面壳与组件测试
config/           配置模板
docs/             需求、技术设计、交互和验证记录
```
