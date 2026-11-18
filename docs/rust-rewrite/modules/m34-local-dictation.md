# M34 本地听写

Next 的听写只使用本机麦克风和本机 `whisper-cli`。应用捕获最多两分钟的音频，重采样为 16 kHz 单声道 WAV，交给本地模型，转写完成后把结果放进 Agent 输入框；用户仍需手动发送。云端 ASR、语音服务密钥和远程音频上传不在协议中。

配置 `DRIFTING_WHISPER_MODEL` 指向 `whisper.cpp` 的模型文件，配置 `DRIFTING_WHISPER_CLI` 指向 `whisper-cli`。未配置模型时，按钮给出本地模型缺失提示。录音、停止、取消、切换作品和转写失败都不会把未确认的文字写入对话。

CPAL 负责平台采集，Linux 使用 ALSA，Windows 使用 WASAPI，`dr-platform::dictation` 负责临时 WAV 和进程生命周期。模型和录音不进入书库、同步、崩溃报告或 Agent 日志。

`cargo test -p dr-platform --locked dictation` 覆盖 WAV 采样率和尺寸；真实麦克风验收在 GitHub Linux/Windows runner 之外记录为手工边界。
