# M34 本地听写

正文和 Agent 输入框共用设备本地听写控件：开始、停止转写、取消、修改草稿和插入。转写结果只在作者按「插入」后回填输入框，发送仍由作者触发。取消、切书或销毁输入框会取消未确认结果。云端 ASR、语音服务密钥和远程音频上传不在协议中。

## 安装

未安装模型时，控件提供 Tiny（78 MB）、Base（148 MB）、Small（488 MB）多语言模型下载，也可导入本机 Whisper GGML 模型。下载来自 whisper.cpp 维护者的公开模型仓库，固定 revision、字节数和 SHA-256；失败、取消、截断、超长或哈希不符都不替换已有模型，临时下载自动清除。模型只进入当前 Next 身份和 profile 的设备数据目录，不进入作品、同步或崩溃报告。

Linux 和 Windows Next Alpha 安装包包含 CPU-only `whisper-cli`，由 `script/build-whisper.py` 从 whisper.cpp v1.8.2 的固定源码提交构建，先核对源码归档 SHA-256，并保留 MIT 授权说明。模型按需安装，不随程序打包。开发运行可设置 `DRIFTING_WHISPER_CLI` 和 `DRIFTING_WHISPER_MODEL`；未设置时优先使用程序旁 `dictation/` 下的推理程序和当前 profile 已安装的模型，再回退到 PATH。

CPAL 使用 Linux ALSA、Windows WASAPI。录音最多两分钟，转为 16 kHz 单声道 WAV，在私有临时目录中推理；取消会终止推理进程，退出后删除音频和转写文件。模板编辑区的听写栏与正文以有高度约束的列布局排列，单元格键盘操作保持可用。

## 验收

- `dr-platform::dictation` 的 WAV 测试核对重采样率、样本数和采样内容。
- `dictation::models` 的合成字节测试核对原子替换，以及取消、损坏、截断和超长下载保留原模型。
- `python script/build-whisper.py` 构建真实 CPU 推理程序；`python script/check-dictation.py` 下载并校验固定 Tiny，识别 Testkit 的合成语音，再运行应用实际转写路径的显式验收测试。测试检查六个关键词、16 kHz 转换和预先取消；普通离线测试不下载模型。合成语音由 eSpeak NG 生成，来源、生成脚本和 CC0 权利记录在 `dr-testkit/fixtures/README.md`。
- 日常 CI 的 Linux x64、Windows x64 与手动 Release 验证任务运行同一合成识别流程。

真实中文麦克风、设备权限、音质和两个平台安装包内的听写交互仍须分别记录；合成音频不能代替这些结果。
