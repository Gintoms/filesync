# Windows 桌面构建

目标为 Windows 10/11 x64，Tauri 2 + Vue 3 桌面壳，包含独立的 `filesync.exe` 后台。使用 NSIS 安装程序，提供简体中文和英文，默认安装给当前用户。已有 WebView2 时直接使用，缺失时通过 Microsoft 引导程序下载安装，因此首次安装可能需要联网。

## 在本机 macOS 交叉构建

当前 Intel Mac 使用 LLVM/LLD 20、makensis 3.12、cargo-xwin 0.23.1 和项目固定的 Rust 1.98.1。工具安装一次即可：

```sh
brew install --force-bottle llvm@20 lld@20 makensis
rustup target add x86_64-pc-windows-msvc --toolchain 1.98.1
rustup component add llvm-tools --toolchain 1.98.1
uv venv .cache/windows-tools
uv pip install --python .cache/windows-tools/bin/python cargo-xwin==0.23.1
```

在项目根目录运行：

```sh
sh scripts/build-windows.sh
```

Apple Silicon / 其他安装位置可用 `FILESYNC_LLVM_BIN`、`FILESYNC_LLD_BIN` 指定对应 LLVM 和 LLD 的 `bin` 目录。本地脚本只在构建进程中设置 PATH 和 Cargo 缓存，不修改全局 Cargo 镜像或默认 Rust 工具链。

构建先生成真正的 Windows 后台二进制，再按 Tauri 要求复制为 `ui/src-tauri/binaries/filesync-x86_64-pc-windows-msvc.exe`，最后生成桌面和安装程序。Rust 与原生依赖使用静态 CRT，避免额外要求用户手动安装 Visual C++ 运行库。

交叉构建 runner 设置 `STATIC_VCRUNTIME=false`，禁用 Tauri 的“静态 VCRuntime + 动态 UCRT”额外链接参数，统一使用 `+crt-static`。否则它会与 cargo-xwin 的静态 UCRT 配置冲突，导致链接时找不到 `round`、`strlen` 等符号。

产物目录：

```text
ui/src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/
```

2026-09-22 已生成 `FileSync_0.1.0_x64-setup.exe`（4,468,773 bytes），将安装包复制到 Windows 电脑后双击安装。首次打开填写中心地址、节点 ID 和节点令牌；菜单随中心配置的上下游关系显示。该包已通过构建与 PE/安装脚本校验，尚未进行 Windows 实机运行验收。

## 在 Windows 构建

安装 Rust MSVC、Node.js 和 Visual Studio C++ 构建工具后：

```powershell
npm --prefix ui ci
npm --prefix ui run build
cargo build --release --locked
node scripts/prepare-desktop.mjs
npm --prefix ui run tauri -- build --bundles nsis
```

GitHub Actions 的 Windows 作业也会构建并保存 NSIS 安装包；工作流文件存在不代表已运行远端 CI。

## 验证边界

交叉编译和 PE 文件校验不能代替 Windows 实机安装、托盘/文件选择器、后台常驻与 20 MB 内存验收。当前开发包没有 Windows 代码签名证书。

使用流程见 [GUI.md](GUI.md)。升级前需要先停止该安装目录启动的后台进程；只关闭桌面窗口或退出界面不会停止后台。

依据：[Tauri Windows 安装包](https://v2.tauri.app/distribute/windows-installer/)、[cargo-xwin](https://github.com/rust-cross/cargo-xwin)。
