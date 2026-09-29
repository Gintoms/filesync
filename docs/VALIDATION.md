# 验证记录

日期：2026-09-22。此记录区分基础项目验证与完整产品验收。

## 图形界面增量验证

- 节点删除：组件测试验证取消不会请求删除、确认后删除并刷新列表；真实 HTTP 中心/agent 端到端测试验证匿名和节点令牌均无删除权限、管理员删除下游会清理关系/回执/在线记录、删除上游会清理中心清单、旧令牌返回 401、重复删除返回 404，以及已接收文件内容保持不变。测试使用临时节点，没有删除用户现有节点。
- 删除功能已部署到本机中心：实际首页加载 `index-DDEgcWaA.js`，包含删除入口与确认文案；节点 API 的 `Allow` 包含 `PATCH,DELETE`；重启前后节点 ID/名称完全一致（当时共 3 个节点），局域网地址健康检查 200。前端 5 个组件测试、后台 9 个测试和 Clippy 均通过。
- HTTP 兼容 macOS 通用 App/PKG/ZIP 与 Windows x64 NSIS 包均已重新生成；下方包摘要对应 HTTP 兼容版本。节点删除是中心 Web/API 功能，本机中心已另行更新，客户端菜单不涉及中心节点管理。

- HTTP 兼容性修复：新增中心 `allow_http` 配置，客户端允许 HTTP/HTTPS；中心仍要求显式开启非回环 HTTP。7 个核心测试、1 个 HTTPS 端到端测试、1 个 HTTP 图形 API 端到端测试全部通过，Clippy `-D warnings` 通过。HTTP 测试实际启动中心子进程监听 `0.0.0.0`，验证认证、发布、认领、下载、后续新增文件和回执。
- 本机 `.local/hub.toml` 已设置 `listen = "0.0.0.0:8787"`、`allow_http = true`；新版中心已启动。通过本机局域网地址检查 `/health` 和 `/` 均为 200，带认证节点列表读取成功、原有 2 个节点保留。这不等于另一台电脑穿过防火墙的实测。

- 中心 Vue 3、节点 Tauri 2 桌面壳均通过编译检查；前端类型检查、生产构建通过。
- 前端 5 个组件测试通过：管理员登录/创建节点/退出、两层关系选择约束、原生目录选择结果转换为前缀认领、上游/下游/未分配关系的菜单隐藏和身份变化后切换页面、删除节点的取消/确认及列表刷新。文件选择器由测试替身模拟，不是系统窗口实测。
- 后台原有 5 个核心测试、HTTPS 端到端测试继续通过。新增 `tests/ui_api.rs` 用真实后台进程验证图形 API 发布、待定不下载、认领、新增文件自动接收、回执、停止发布保留文件、TOML 不被改写及认证隔离。
- 桌面桥接命令白名单测试通过，后台与桌面 Clippy 均以 `-D warnings` 通过。
- 图形改动后重跑 10 个小文件 + 1 MB 文件，四阶段峰值为 5.669 / 4.870 / 6.095 / 5.042 MB；最高 6.095 MB。依然只是单轮基线，不等于常驻后台和三平台完整验收。
- macOS 应用已更新为 Universal `FileSync.app`：`ui/src-tauri/target/universal-apple-darwin/release/bundle/macos/FileSync.app`。桌面和后台程序均通过 `file` 与 `lipo -verify_arch` 的 ARM64/x86_64 检查，Info.plist 校验及 `codesign --verify --deep --strict` 通过；使用 ad-hoc 签名，尚无 Developer ID 签名和公证。中心在 `127.0.0.1:8787` 的首页、JS/CSS 与健康检查此前均返回 200。
- 通用包内后台在本机 Intel Mac 上运行 `--version` 及 HTTPS 四阶段同步检查通过（10 个小文件 + 1 MB 文件），峰值 5.706 / 4.915 / 6.042 / 5.181 MB。报告：[macos-universal-intel-2026-09-22.json](validation/macos-universal-intel-2026-09-22.json)。这是 Intel 上的单轮基线，不能代表 ARM64 实机、常驻后台或完整内存验收。
- 可传输压缩包：与 App 同目录的 `FileSync_0.1.0_macos_universal.zip`，14,095,589 bytes，SHA-256 `99f365f3535cd9d6fa8c4b38218e6e3c666af613120133bad2e097e79c7ad528`。已重新解压检查，两个程序的双架构和签名均通过。
- macOS 向导安装包：`ui/src-tauri/target/universal-apple-darwin/release/bundle/pkg/FileSync_0.1.0_macos_universal.pkg`，14,100,250 bytes，SHA-256 `ac44df8d4b9ba7e6b9d7bc136077914fdb4d7ddc59766864efdd986c541fd29d`。`installer -showChoicesXML` 可正常读取；安装域为本机系统，目标固定 `/Applications/FileSync.app`，组件 `relocatable=false`，包含中文欢迎/完成说明，没有安装前后执行脚本。解包后两个二进制与原 App 字节一致，双架构和 App 签名校验通过。
- `.pkg` 的 `pkgutil --check-signature` 显示未签名；本机 `security find-identity -v -p codesigning` 无有效身份。未执行真实系统安装，也没有 ARM64 实机安装/运行证据；不能将安装器读取与解包检查当作实机验收。正式分发的 Developer ID 签名和公证仍待完成。
- 已在 macOS 交叉构建 Windows x64 桌面及独立后台，生成 `ui/src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/FileSync_0.1.0_x64-setup.exe`（4,468,773 bytes）。`file` / `llvm-readobj` 确认两个程序均为 x86-64 PE，桌面为 GUI 子系统；导入表没有额外的 Visual C++ 运行库 DLL。NSIS 脚本确认同时包含桌面和 `filesync.exe`，简中/英文、当前用户安装、缺失 WebView2 时下载引导程序。安装器自身的 NSIS 启动壳为 32 位，应用载荷为 64 位。
- Windows 安装包 SHA-256：`833109a0b5c968ada68f8ed2825720701253f345630752560820f7ed372bcb95`。包含上下游菜单隐藏修复；macOS 应用也已重新打包，`codesign --verify --deep --strict` 通过。
- 当前会话浏览器/桌面控制连接不可用，未进行真实窗口截图与系统选择器交互验收。macOS ARM64 实机运行、Windows 实机安装/运行、Linux 构建/运行及远端 CI 仍待确认；Windows 包尚未代码签名。macOS CI 已改为构建并检查 Universal 包，配置文件不代表远端已执行。

下文为本次图形改动前的基础版测量，二进制摘要仅对应当时的构建。

## 本机环境与构建

- macOS 26.6.1（25G76），Darwin 25.6.0。
- Rust 1.98.1，构建目标 `x86_64-apple-darwin`。
- `target/release/filesync`：Mach-O x86_64，版本 `0.1.0`。
- 二进制 SHA-256：`733f8331b16740fda15fa8439f7e16fec71a0360fd25ba960b65a516a974fa68`。
- 使用项目隔离 Cargo 缓存，未修改本机全局镜像或默认工具链。

## 已通过

```sh
sh scripts/cargo-local.sh fmt --check
sh scripts/cargo-local.sh test --locked
sh scripts/cargo-local.sh clippy --locked --all-targets -- -D warnings
sh scripts/cargo-local.sh build --release --locked
target/release/filesync --version
target/release/filesync --help
python3 scripts/tui_smoke.py
```

5 个文件安全/业务规则测试和 1 个 HTTPS 端到端测试通过。端到端测试实际启动 agent 子进程，并覆盖常驻 agent 的带认证本机控制 API。

| 验证项 | 结果 |
|---|---|
| 自连、第三层关系拒绝；多个上游可指向一个下游 | 通过 |
| 精确/最长前缀匹配及上游隔离 | 通过 |
| 路径穿越、保留名、目标符号链接拒绝 | 已测用例通过；不代表跨平台路径竞态已全面加固 |
| 损坏内容不替换原文件；最近 3 份备份 | 通过 |
| 扫描失败保留原发布队列 | 通过 |
| HTTPS 校验证书与实际上传下载 | 通过 |
| 未认领不下载；通过本机 API 认领后同步 | 通过 |
| 上游多次更新，下游只接收最新内容 | 通过 |
| 前缀认领后新增文件自动接收 | 通过 |
| 截断发布清单不覆盖原清单 | 通过 |
| 非直接下游不能读取清单 | 通过 |
| 多来源目标冲突不互相覆盖 | 通过 |
| 上游撤下后下游文件保留、标记 obsolete | 通过 |
| TUI 通过认证接口显示列表、按 q 退出 | Unix PTY + 控制 API fixture 冒烟通过；原始终端输出在 `.local/tui-smoke.ansi` |

## 内存基线

运行命令：

```sh
python3 scripts/memory_probe.py --files 20000 --large-bytes 99000000 --output .local/memory-report.json
```

测试只使用临时生成的 20,000 个 64-byte 文件与 1 个 99,000,000-byte 文件；中心和 agent 分进程，校验 HTTPS 服务器证书。计量使用系统 `wait4` 的进程峰值 RSS，不计 Python 驱动、中心、TUI 的内存。阈值按 `20,000,000 bytes` 比较。

| 阶段 | 峰值 RSS（MB） | 用时（秒） | 本轮是否低于 20 MB |
|---|---:|---:|---|
| 扫描 20,001 个文件并上传变化内容 | 8.704 | 5.364 | 是 |
| 接收 20,001 条待定，内容不下载 | 6.832 | 6.997 | 是 |
| 认领并下载 99 MB 文件，2 万小文件仍待定 | 7.471 | 7.459 | 是 |
| 重复扫描，内容未变不重复上传 | 7.401 | 4.208 | 是 |

99 MB 文件落地后的 SHA-256 对比通过。`published` 统计上传内容对象数；2 万小文件内容相同，因此首次上传去重为 2 个对象，不代表清单只有 2 个文件。

原始 JSON：[macos-x86_64-2026-09-22.json](validation/macos-x86_64-2026-09-22.json)。当前本机基线最高 **8.704 MB**，不推断其他系统结果。

另一次认领全部 20,001 个文件的测试在小文件下载过程中主动停止，未得到完整结果，不计为通过；逐文件下载、落盘刷盘和回执的吞吐需要后续优化。

该基线使用 `agent --once`，扫描和传输顺序执行，传输并发数为 1。它不能证明常驻控制服务、并行扫描与传输、持续增长数据库或一小时稳态都符合要求。

## 未验收

- Windows、Linux 节点实机执行及内存；三平台 CI 文件仅准备完毕，未提交/触发远端执行。Linux 中心容器验证见下方。
- 20 MB 完整 NFR 验收。
- 断点续传、系统服务与完整产品界面等 README 列明的待开发功能。

## Linux amd64 中心初始镜像（2026-09-23，已由下方两版保留镜像替代）

- 已构建 Linux amd64 中心镜像（私有仓库地址已隐去），包含 Web 管理界面；镜像大小 82,263,719 字节。
- 本地镜像 ID：`sha256:a2ce98007b49974cc14608ab04ffcce65cd0008cbdcd95143134df9c8580344a`（不是远端仓库 digest）。
- Docker 容器下载依赖发生连接重置，因此使用 `scripts/build-docker-offline.sh` 的流程：本机构建 Web、准备 Cargo vendor，在 Rust 1.98.1 Linux 容器中执行 `cargo build --release --locked --offline`。没有修改 Docker 全局代理或镜像源。
- `scripts/docker-smoke.py` 通过：HTTP 健康检查、首页及两项静态资源、匿名管理员请求拒绝、UID 10001、节点创建/删除、SIGINT 正常停止、重建容器后节点与上传文件保留、文件下载内容一致。验证容器和临时卷已清理。
- `filesync --version` 输出 `filesync 0.1.0`；Compose 配置校验通过。
- 首次推送因 `unauthorized` 被拒绝；随后重新推送成功。远端 manifest 已读取确认，config digest 与上述本地镜像 ID 一致。仓库 digest：`sha256:5ede444c487fda95908044290869e1ee6b685033b6f0c41c0130df3853e83fb7`。
- 这次验证只覆盖中心容器，不代表 Linux 节点桌面、后台内存或全部业务验收。

## 中心保留两版与自动清理（2026-09-23）

- 每个 `(上游, 路径)` 保留最新及上一不同内容版本；相同内容重复发布不轮换，下游仍只接收最新版。更旧无引用内容在发布后清理，定时任务每 60 秒补扫；保留上传租约和在途下载。
- 新增 8 项 Rust 测试，覆盖两版轮换/重复发布/事务回滚、跨路径与跨上游共享、待发布租约、在途下载、错误上传与租约过期、删除节点、重启/目录独占锁、旧库迁移和上传中并发清理。连同原有 9 项测试全部通过，`cargo clippy --locked --all-targets -- -D warnings` 通过。
- macOS release 实际临时中心：连续发布三版后第一版删除；删除节点后约 59.51 秒自动清空无引用内容。原始结果见 `validation/retention-periodic-2026-09-23.json`。
- Linux amd64 新镜像容器验证通过：HTTP/Web/鉴权、非 root 运行、三版仅保留后两版、重复发布不丢上一版、正常停止、重建后两版仍保留且可下载最新版、节点删除。结果见 `validation/docker-retention-amd64-2026-09-23.json`。
- 已更新并推送私有仓库中的 `0.1.0` 镜像；镜像大小 82,306,183 字节。本地 ID：`sha256:6263da1bd836206ff320f0f8fe2cc2e54d48b33e95693f689ef558bc5909631d`，远端 digest：`sha256:9df5efeb6bac391147aab0621b52f0305eef25d34411dfb725f1c353daed483c`。
- 本机 `target/release/filesync` 已重新构建；已有运行中的中心及客户端安装包未替换。Docker 用户需拉取并重建容器，沿用原数据卷/令牌。旧库无法恢复未记录的历史路径关联，完整两版保留从下一次更新开始。
