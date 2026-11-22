# Next Linux

Next 使用 GPUI 的 Linux 平台实现。Linux 凭据使用 Secret Service；没有桌面凭据服务时报告失败，不保存明文。日常 GitHub CI 增加 Ubuntu 24.04 x86_64 lane，安装 ALSA、Fontconfig、Wayland/X11 和 Vulkan 构建依赖，运行和 Windows 相同的 Rust 格式、Clippy、测试和构建检查。

窗口导航沿用书内历史模型。返回优先离开铺满视图，然后返回上一页，历史用尽回到书架；关闭窗口先存布局和正在编辑的模板，再让系统关闭窗口。
