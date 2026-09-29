# 中心 Web 与节点桌面

## 使用流程

1. 启动中心，浏览器打开中心地址；本机示例是 `http://127.0.0.1:8787/`。
2. 输入中心的管理员令牌，在“节点管理”中创建上游、下游。节点令牌只显示一次，可下载凭证文件。
3. 在“分发关系”中指定谁向谁发文件。一个上游向全部关联下游提供相同内容，只允许两层。
4. 各电脑打开 FileSync 桌面应用，输入中心地址、节点 ID 和对应令牌；自签 HTTPS 证书需要选择 CA。已有配置也可通过“选择 TOML 文件”连接。

局域网不使用 HTTPS 时，在中心配置中设置 `listen = "0.0.0.0:8787"`、`allow_http = true`，不填写 `tls_cert` / `tls_key`。重启中心后，客户端填写 `http://中心IP:8787`，CA 留空即可。此模式不加密令牌和文件。此前强制远程 HTTPS 的旧版客户端需要更新安装包。
5. 上游进入“发布文件”，通过系统文件选择器选择本机文件或目录，确认发布名称。
6. 下游进入“接收文件”，选择本机目录后才下载。选择“认领目录”会接收此目录当前及未来新增的文件。
7. 在中心或上游的“分发结果”查看最新版回执。“未确认接收”可能是未认领、离线、正在同步或尚未补报，不能直接当作失败。

桌面菜单根据中心关系显示：上游只有“发布文件、分发结果”，下游只有“接收文件、保存位置”；未分配关系时显示“节点状态”。关系变化后刷新会更新菜单并离开不再适用的页面，发布 API 也会校验节点确实有下游。

中心“节点管理”每行提供“重命名、删除”。删除需二次确认，适用于未分配、上游和下游节点；确认后事务性清理节点、分发关系、相关回执和中心文件清单，原令牌立即失效。电脑上的原文件、已接收文件及备份保留，中心共享内容对象不在此操作中清理。重新接入需要创建新节点及凭证。

关闭窗口会隐藏到托盘；托盘可重新打开窗口。“退出界面（继续同步）”只退出桌面，后台仍运行。当前还没有开机自启和系统服务安装器。

## 构建

### 给其他 Mac 安装

分发 `ui/src-tauri/target/universal-apple-darwin/release/bundle/pkg/FileSync_0.1.0_macos_universal.pkg` 即可。对方双击安装包，按系统向导继续并授权，安装到 `/Applications/FileSync.app`，然后从“应用程序”打开。Intel 和 M 系列 Mac 使用同一个包，无需安装 Rust、Node.js 或另一个后台包。

首次打开填写可访问的中心地址、该电脑专用的节点 ID 和令牌；凭证由中心管理员提供，安装包不携带节点凭证。升级前需要退出旧界面并停止旧版后台；只关闭窗口不会停止同步。

当前 `.pkg` 未使用 Developer ID Installer 证书签名，内含 App 为 ad-hoc 签名且未公证。其他 Mac 可能拦截首次安装或打开；只对确认来源可信的包，在系统“隐私与安全性”中按提示允许。正式对外发行仍需 Developer ID Application / Installer 证书和 Apple 公证。本机目前没有可用的开发者签名身份。

已有通用版 App 时，重新生成安装包只需：

```sh
python3 scripts/package-macos.py
```

该脚本固定安装位置、加入中文安装说明，并解包检查两个程序的双架构、App 签名和原始字节一致性。详细构建步骤如下。

开发需要 Node.js、npm、Rust 和平台编译环境；最终用户运行应用不需要 Node.js、Rust 或 Python。

中心及后台：

```sh
npm --prefix ui ci
npm --prefix ui run build
sh scripts/cargo-local.sh build --release --locked
```

标准 Cargo 环境可直接使用 `cargo build --release --locked`。前端资源内嵌在中心可执行文件中，不需要部署 Node 服务。没有构建前端资源时，后台仍可构建，但根路径会明确提示资源缺失。

macOS 完整桌面应用包（Universal，同时包含 ARM64 和 x86_64）：

```sh
sh scripts/build-desktop.sh --bundles app
```

产物：`ui/src-tauri/target/universal-apple-darwin/release/bundle/macos/FileSync.app`。同一个 App 内的桌面程序与后台程序均包含 ARM64 和 x86_64，系统自动使用对应架构，无需分别安装。脚本会安装两种架构的 Rust 标准库，分别编译后台并用 `lipo` 合并，再通过 Tauri 的 Universal 目标构建桌面。使用本机 ad-hoc 签名，尚未进行 Developer ID 签名和公证。可以双击，或：

```sh
open ui/src-tauri/target/universal-apple-darwin/release/bundle/macos/FileSync.app
```

本次构建另提供同目录的 `FileSync_0.1.0_macos_universal.zip`，解压后仍是一个 `FileSync.app`，可复制到 Intel 或 M 系列 Mac 使用。旧的 `target/release/bundle/macos` 目录可能仍保留此前的 Intel 单架构包，请使用上述 `universal-apple-darwin` 路径。当前已完成双架构与签名检查和 Intel 后台同步检查，ARM64 实机运行仍待验证。

使用指定的现有配置（已有桌面实例时，先从托盘退出界面）：

```sh
ui/src-tauri/target/universal-apple-darwin/release/bundle/macos/FileSync.app/Contents/MacOS/filesync-desktop --config /absolute/agent.toml
```

Windows x64 安装包与构建步骤见 [WINDOWS.md](WINDOWS.md)，本机可用 `sh scripts/build-windows.sh` 交叉构建。Windows/Linux 标准构建环境先执行前端构建与 `cargo build --release --locked`，再运行 `node scripts/prepare-desktop.mjs`，最后 `npm --prefix ui run tauri -- build`。Linux 需要 WebKitGTK 4.1、GTK/AppIndicator 等开发包，见 [Tauri 平台前置要求](https://v2.tauri.app/start/prerequisites/)。CI 已加入三平台编译检查，但本机运行不代表远端 CI 或三平台验收通过。

## 配置与凭证

- 首次桌面连接会验证节点 ID 和令牌一致，生成应用自己的 `agent.toml`，凭证单独保存在数据目录 `node.token`。Unix 文件权限为 `0600`；Windows 文件存于当前用户应用数据目录，继承该目录 ACL。
- 环境变量令牌优先于凭证文件。用户导入的 TOML 不会被界面改写。
- 桌面发布源保存到 `sources` 表；认领规则保存到 `claims` 表。相同名称/映射键由 TOML 优先，界面显示来源并禁止直接删除配置项。
- 单个文件映射优先于目录映射，目录映射中最长前缀优先。更改目录映射不会覆盖此前对单个文件单独指定的位置。
- 中心管理员令牌只留在页面内存，刷新或退出需重新输入；不会写入浏览器本地存储。
- 桌面前端只通过限定的 Tauri 命令调用后台，不获得节点令牌，不启用通用 shell 或任意文件读写权限。
- 桌面启动独立后台，后台日志位于数据目录 `agent.log`。若已有旧版后台，需先停止旧进程，再由桌面连接新版本。
- 20 MB 指标只统计后台同步进程；桌面、WebView 和中心另计。新增接口后的完整三平台常驻内存验收仍待完成。

## 本次范围

已接通：中心节点创建/重命名、关系建立/删除、最近联系时间、分页分发结果；桌面首次连接/导入配置、发布/停止发布、文件搜索分页、单文件/目录认领、冲突提示、更改保存位置、取消映射、手动触发同步、托盘开关。

仍未完成：一次性注册与账号密码登录、节点凭证吊销、系统通知、完整多选/过滤与发布预览、传输进度/暂停/续传、系统服务与自动启动、Windows/Linux 安装包实机验证及 macOS 签名公证。桌面设置凭证当前使用权限受限文件，尚未接入系统钥匙串。

当前桌面首次创建的配置会自动记住；通过文件选择器临时导入其他 TOML 仅用于本次桌面会话，重启后可重新选择，或使用 `--config` 固定路径。

## 验证

```sh
npm --prefix ui test
npm --prefix ui run build
sh scripts/cargo-local.sh test --locked
sh scripts/cargo-local.sh clippy --locked --all-targets -- -D warnings
sh scripts/cargo-desktop.sh test --locked
sh scripts/cargo-desktop.sh clippy --locked --all-targets -- -D warnings
```

`tests/ui_api.rs` 使用临时中心与真实后台进程验证权限、发布、待定不下载、目录认领、未来新增文件、回执、停止发布后保留本机文件及 TOML 不被修改；前端测试覆盖管理员登录/退出、创建节点、关系选择约束和系统目录选择后提交认领。组件测试模拟原生文件选择器，不能替代三平台原生窗口人工验收。
