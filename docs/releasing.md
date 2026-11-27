# 发布 Drifting

普通 PR 合入长期开发分支 `next`。只有本仓库的 `next` 可以通过 PR 合入 `main`。
每次 PR 更新都会执行 **Cloud tests**。全部通过后，**Cloud build** 生成 macOS、Windows、
Linux 和 Next 编译产物，供下载验收。合入 `main` 后，GitHub Actions 在合并提交上
重新执行检查和构建。发布任务会核对主线提交确实来自本仓库已合并的 `next → main` PR，
并在签名产物、平台验证和校验清单齐备后自动发布。

如需在开 PR 前复验，推送到 `codex/ci/**` 会运行同一套 **Cloud tests** 和
**Cloud build**，并把预览产物留在该运行的 Artifacts。正式发布仍要求合入
`main` 的 `next → main` PR。

Tauri 正式版使用 `main-<完整 SHA>` 标签，发布说明包含该次主线推送的提交信息。
每次发布都附有 `COMMITS.txt`，记录完整 SHA、作者、提交者、日期和正文；超出
GitHub 发布说明长度时，完整记录仍可从附件下载。相同源码重跑会复用发布标签。
自动更新仍按应用版本比较，修改代码但维持原版本号会生成可下载的 Release，
现有安装需要更高的应用版本才会收到更新。

从 0.1.0-alpha.11 起，App 先试 `releases/latest/download/update.json`，拿不到（404）再读
`latest.json`。`update.json` 只由新版（Rust 客户端 0.2）的 Release 提供，到时候装了
alpha.11 及以后的 0.1 就会被交给新版。所以 **0.1 的 Release 永远不能带 `update.json`**；
新版发布以后不再发 0.1。见 [m42-update.md](rust-rewrite/modules/m42-update.md)「旧版那头的交接」。

## 官方构建的内容

| 项 | 值 |
| --- | --- |
| 平台 | macOS 13+，Apple Silicon（`aarch64-apple-darwin`）；Windows 10/11 x64 与 ARM64（预览，见 [windows.md](windows.md)）；Linux x86_64（见 [linux.md](linux.md)） |
| 构建机 | GitHub `macos-26`，Xcode 26.6（macOS 26 SDK）。用更早的 SDK 编译，新系统会给 App 画旧样式的标题栏，红绿灯比 App 设定的位置偏下 |
| 打包 | `Drifting_<版本号>_macos_arm64.dmg`（首次安装）、`.app.tar.gz` + `.sig`（自动更新）、`latest.json`、`SHA256SUMS.txt`、`sbom.spdx.json`；Windows 为 `Drifting_<版本号>_windows_<x64\|arm64>-setup.exe` + `.sig`；Linux 为 `Drifting_<版本号>_linux_x86_64.AppImage`、`.deb`，以及 AppImage 的 `.tar.gz` + `.sig` 自动更新包 |
| 图标 | macOS 打包前从 Icon Composer 源文件编译 `Assets.car`，支持浅色、深色、Clear 和 Tinted；旧系统保留 `.icns`。构建产物与 DMG 内的 App 都检查图标资产，见 `docs/qa/macos-app-icon.md` |
| 签名 | Developer ID Application 证书；App 和外层的 DMG 都用 App Store Connect API Key 公证并 staple（DMG 那一步在 Tauri 构建之后单独做）。Windows 安装包暂不做 Authenticode 签名，首次运行 SmartScreen 会提示；自动更新仍由同一把更新密钥签名校验 |
| Hosted | 连接 `vars.DRIFTING_HOSTED_ORIGIN`，开放注册；注册页链接 `https://drifting.cc/terms` 与 `https://drifting.cc/privacy` |
| 更新 | 启动时和开着时每 3 小时最多检查一次；有新版本时通知中心出现一条，铃铛留一个更新记号，打开通知中心才消失（重启前没打开过，启动时立刻重查、记号回来）；点开到 设置 › 更新，点一次「安装并重启」就下载、校验签名、保存全部本地修改、安装并重启；下载中和下载好待安装时不做自动检查 |

源码默认仍是纯本地构建（`pnpm dev`、`pnpm tauri:build`），不含 Hosted，也不能检查更新。

## 一次性准备

在仓库 **Settings › Secrets and variables › Actions** 里配置。`release` environment
在 **Settings › Environments › release** 将允许部署的分支限定为 `main`。需要全自动发布时，
该环境应允许主线任务自动进入，避免配置需要人工批准的部署门禁。

在 **Settings › Rules › Rulesets** 导入
[`main.json`](../.github/rulesets/main.json) 和 [`next.json`](../.github/rulesets/next.json)。
启用后要求通过 `PR target`、`Cloud tests` 和 `Cloud build`，以合并提交保留完整历史，
并限制删除和强制推送。保留 GitHub 仓库的合并提交选项，关闭自动删除合并后的分支。
仓库内的规则集文件是管理员导入的配置，GitHub 不会随代码提交自动启用它们。

### Variables

| 名称 | 值 |
| --- | --- |
| `DRIFTING_HOSTED_ORIGIN` | `https://api.drifting.cc` |
| `DRIFTING_UPDATER_PUBLIC_KEY` | 下面生成的更新签名**公钥**（`.pub` 文件的全部内容） |

### Secrets

| 名称 | 来源 |
| --- | --- |
| `APPLE_CERTIFICATE` | Developer ID Application 证书导出的 `.p12`，`base64 -i cert.p12 \| pbcopy` |
| `APPLE_CERTIFICATE_PASSWORD` | 导出 `.p12` 时设的密码 |
| `KEYCHAIN_PASSWORD` | 任意随机字符串，只给 CI 临时钥匙串用 |
| `APPLE_API_ISSUER` | App Store Connect › 用户和访问 › 集成 › Issuer ID |
| `APPLE_API_KEY` | 同页 API Key 的 Key ID |
| `APPLE_API_PRIVATE_KEY` | 下载的 `AuthKey_<KeyID>.p8` 全文 |
| `TAURI_SIGNING_PRIVATE_KEY` | 下面生成的更新签名**私钥**文件全文 |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | 生成私钥时设的密码 |

1. **Developer ID 证书**：Xcode › Settings › Accounts › Manage Certificates › **+** ›
   *Developer ID Application*。在“钥匙串访问”里找到它，连同私钥导出为 `.p12`。
2. **公证 API Key**：App Store Connect › 用户和访问 › 集成 › App Store Connect API，
   新建一个 *Developer* 权限的 Key，下载 `.p8`（只能下载一次）。
3. **更新签名密钥**：在本机运行
   `pnpm tauri signer generate -w ~/.tauri/drifting-updater.key`，按提示设密码。
   私钥和密码只放进 GitHub Secrets 和你的密码管理器。**私钥丢失后，已安装的 App
   再也无法自动更新**，只能让用户手动下载新版。

## 每次发布

1. 如需向已安装客户端推送版本更新，先通过 PR 把 `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`
   的版本改成同一个新版本号（如 `0.1.0-alpha.2`），并写好
   `docs/releases/<版本号>.md`。第一行 `# Drifting <版本号>` 之后的正文会原样显示在
   App 的更新提示和 设置 › 更新 里，写给用户看，保持简短。中英双语（2026-11-05 作者定）：先写中文条目和
   结尾提醒，空一行，再写同样内容的英文。App 按纯文本显示（保留换行，不渲染 Markdown），所以不要用标题或分隔线。
2. README 截图**只在作者说要截时才截**（2026-11-04 作者定），发版本身不截：那时跑
   `pnpm release:screenshots --readme`，截英文和中文浅色的书架、工作区和要素全景各三张，英文写进
   `docs/screenshots/`（README.md 显示），中文写进 `docs/screenshots/zh-CN/`（README.zh-CN.md 显示）；
   加 `--locale en` 或 `--locale zh-CN` 只重截一种语言，和当次改动一起提交。约一分钟，期间会把 App
   窗口调到最前，别动键盘和鼠标；见 [release-screenshots.md](release-screenshots.md)。不带
   `--readme` 则在 `out/screenshots/<版本号>/` 截中英文、浅色深色全部 12 张，不进仓库。
3. 本地运行 `pnpm release:check v<版本号>`，再跑
   `pnpm lint`、`pnpm typecheck`、`pnpm test`。
4. 把改动合入 `next`；发版时提交 `next → main` 的 PR，等待 `Cloud build` 与其他必需检查。
5. 合并后查看 **CI** 的发布任务。Tauri 由签名和公证后的 macOS 构建、Windows x64/ARM64
   安装器、Linux AppImage/DEB 和各平台更新清单一并发布。任一平台失败时，先重试失败的任务。
6. 发布后可下载 DMG 并执行下方人工冒烟检查。重建并部署官网时，首页的下载按钮按文件名下载最新 Release 的
   `Drifting_<版本号>_macos_arm64.dmg` 和 Windows 安装包，版本号取自 `package.json`。
   在 Drifting-Showcase 里运行
   `node <客户端>/scripts/design-assets/landing/build.mjs site`，跑它的测试后推送 main
   （推送就是部署）；见 `scripts/design-assets/landing/README.md`。

发现问题时，通过正常 PR 修复，再合入主线发布。已发布的产物应保持可追溯。

## 冒烟检查

- DMG 能打开，拖进“应用程序”后首次启动没有 Gatekeeper 拦截。
- 切换系统的浅色、深色、Clear 和 Tinted 图标外观，程序坞与 Finder 中的冰山图标跟随变化。
- 首次启动不需要账号，书架上能新建项目、写字，退出重开内容还在。
- 注册新账号能收到验证码邮件，登录后本地项目上传、云端项目下载正常。
- 设置 › 隐私、设置 › 关于 里的链接能打开 drifting.cc 和 GitHub 页面。
- 从上一个已发布版本升级：通知中心出现新版本提示，下载、安装、自动重启后出现
  “已更新到”通知，项目、素材、账号和模型密钥都还在。
- Windows：在 Windows 11（ARM 虚拟机也可以）装对应的 `-setup.exe`，不需要管理员权限；
  按 [windows.md](windows.md) 的手动验收清单走一遍，至少包括微软拼音输入、登录、退出重开。
- Linux：在 Ubuntu 24.04 或兼容发行版上运行 AppImage，确认首次启动、写入、退出重开和
  Secret Service 密钥存储；再用 DEB 包安装一次。Linux 不能导入 HEIC、HEIF 或 AVIF。

## 检查脚本

`scripts/check-release.mjs`（`pnpm release:check`）核对：三处版本号一致且是 SemVer、
tag 等于 `v<版本号>`、发布说明存在、README 截图（`docs/screenshots/manifest.json`）的文件都在、都在 README 里（不要求是这个版本截的）、打包仍生成更新签名产物、App 轮询的更新地址正确、
构建脚本和过程宏不剥离、所有 GitHub Action 都钉在完整 commit SHA 上。
`pnpm ci:contract:check` 核对 CI 与发布工作流的结构。

最后一项对应 `src-tauri/Cargo.toml` 里的 `[profile.release.build-override] strip = "none"`。
release 默认会剥离调试信息，而 Xcode 27 的工具链剥离后的过程宏 dylib 加载不了
（报 `mis-aligned LINKEDIT string pool` 或 `can't find crate`），正式构建在编译 App
之前就会失败。构建脚本和过程宏只在构建机上运行，不剥离它们不影响发出去的 App，App
本身仍会剥离。

## Next Alpha 自动预发布（独立通道）

Next 使用独立的 **Next Release** 工作流（`.github/workflows/release-next.yml`），
由 `main` 推送的 CI 调用，使用独立的安装包和版本号。准备任务串行预留 `0.2.0-alpha.N`：首次为 `alpha.1`；
重试同一源码复用 `next-release/<版本>` 分支与发布提交，已发布的同一源码
不再分配新号。预留分支保留用于审计，不要改写或删除。

准备任务在源提交之上生成一个只修改 Next 对外版本包及 `next/Cargo.lock`
的发布提交。Linux 和 Windows 都检出这一提交，运行格式、Clippy、工作区测试、
`cargo xtask check`，再以 `next-alpha` 特性构建。原有 macOS 和 Windows ARM
日常回归检查仍由 `next.yml` 保留。

安装身份是 `cc.drifting.next.alpha`，产品名 **Drifting Next Alpha**，
数据、系统凭据和协议 `drifting-next-alpha:` 与 Tauri 及开发构建分离。
默认开发构建仍是 `cc.drifting.next.dev`。Alpha 不生成 Tauri 的 `latest.json`，
也不使用其正式自动更新通道。

Linux 任务产生 x86_64 AppImage 和 DEB。DEB 独立安装到
`/opt/drifting-next-alpha`，命令是 `drifting-next-alpha`，不会占用旧客户端
的可执行路径。当前运行库基线是 Ubuntu 24.04。AppImage 用校验 SHA-256 后的
linuxdeploy 打包。两种包都要在临时 X11 会话里出现可见窗口后才能上传。
Windows x64 任务产生 NSIS 安装包，检查独立身份、随包字体、窗口启动、关窗和卸载。

所有验证和平台打包任务成功后，工作流自动发布 **prerelease**。三个安装包、校验清单
和完整提交记录 `COMMITS.txt` 会一起上传。发布说明保留按功能汇总、比较范围、源码提交和发布提交，
并补充本次主线推送的完整提交信息。Next Alpha 不占用 Tauri 正式版的最新发布入口。

本地验收：`python3 -m unittest discover -s next/script/tests -v` 验证编号、
锁文件、说明以及本地 Git 远端预留／重试演练；actionlint 验证工作流结构，
Alpha 身份有 Rust 定向测试。Linux/Windows 云端构建、实际安装与 Wayland、
中文麦克风和已登录外部 Agent 的验证属于独立验收，不能用脚本检查代替。

Next 的 Linux CI 使用无调试符号的开发／测试构建并关闭增量缓存，避免完整 GPUI 工作区测试在 GitHub runner 上耗尽磁盘。2026-11-19 的运行 44461951803 中，Linux 格式和 Clippy 通过，测试阶段 runner 因磁盘满退出；这不是测试全通过的证据。
