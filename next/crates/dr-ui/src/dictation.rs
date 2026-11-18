//! Shared local dictation controls. Transcription is a draft; only Insert
//! emits text. Dropping this view or changing its owner cancels pending work.
use crate::{Theme, components::Button, input::TextArea};
use dr_platform::dictation::{self, Recording};
use gpui::{prelude::*, *};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InsertTranscript(pub String);

pub struct Dictation {
    recording: Option<Recording>,
    cancelled: Arc<AtomicBool>,
    busy: bool,
    draft: Entity<TextArea>,
    error: Option<String>,
    generation: u64,
}
impl EventEmitter<InsertTranscript> for Dictation {}
impl Drop for Dictation {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}
impl Dictation {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            recording: None,
            cancelled: Arc::new(AtomicBool::new(false)),
            busy: false,
            draft: cx.new(|cx| TextArea::new("dictation-draft", window, cx).framed(3.0)),
            error: None,
            generation: 0,
        }
    }
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.cancelled.store(true, Ordering::Relaxed);
        self.recording = None;
        self.busy = false;
        self.draft.update(cx, |draft, cx| draft.set_text("", cx));
        self.error = None;
        cx.notify();
    }
    pub fn is_active(&self) -> bool {
        self.recording.is_some() || self.busy
    }
    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            self.reset(cx);
            return;
        }
        if let Some(recording) = self.recording.take() {
            let Some(model) = dictation::configured_model() else {
                self.error =
                    Some("Choose a local Whisper model first / 请先安装本地语音模型".into());
                cx.notify();
                return;
            };
            let audio = match recording.stop() {
                Ok(audio) => audio,
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            };
            self.cancelled = Arc::new(AtomicBool::new(false));
            let cancel = self.cancelled.clone();
            let generation = self.generation;
            self.busy = true;
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move { dictation::transcribe(audio, &model, &cancel) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.generation != generation {
                        return;
                    }
                    this.busy = false;
                    match result {
                        Ok(text) => this.draft.update(cx, |draft, cx| draft.set_text(text, cx)),
                        Err(error) => this.error = Some(error.to_string()),
                    }
                    cx.notify();
                });
            })
            .detach();
        } else if dictation::configured_model().is_none() {
            self.error = Some("Install whisper.cpp, then choose a GGML model / 请安装 whisper.cpp 并选择 GGML 模型".into());
        } else {
            match Recording::start() {
                Ok(recording) => {
                    self.recording = Some(recording);
                    self.error = None;
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        cx.notify();
    }
    fn install(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Whisper GGML model / 离线语音模型".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move { dictation::install_model(&path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.error = result.err().map(|error| error.to_string());
                cx.notify();
            });
        })
        .detach();
    }
}
impl Render for Dictation {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let draft = self.draft.read(cx).text();
        let active = self.is_active();
        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.0))
                    .child(
                        Button::new(
                            "dictation-toggle",
                            if self.busy {
                                "Cancel / 取消"
                            } else if self.recording.is_some() {
                                "Transcribe / 转写"
                            } else {
                                "Dictate / 听写"
                            },
                        )
                        .small()
                        .on_click(cx.listener(|this, _, _, cx| this.toggle(cx))),
                    )
                    .when(active || !draft.is_empty(), |row| {
                        row.child(
                            Button::new("dictation-cancel", "Discard / 丢弃")
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| this.reset(cx))),
                        )
                    })
                    .when(!active, |row| {
                        row.child(
                            Button::new("dictation-model", "Model… / 模型…")
                                .small()
                                .on_click(cx.listener(|this, _, _, cx| this.install(cx))),
                        )
                    }),
            )
            .when(!draft.is_empty(), |el| {
                el.child(self.draft.clone()).child(
                    Button::new("dictation-insert", "Insert / 插入")
                        .small()
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            let text = this.draft.read(cx).text();
                            if !text.trim().is_empty() {
                                cx.emit(InsertTranscript(text));
                                this.reset(cx);
                            }
                        })),
                )
            })
            .when_some(self.error.clone(), |el, error| {
                el.child(
                    div()
                        .text_color(Theme::of(window, cx).palette.ink(2).solid())
                        .child(error),
                )
            })
    }
}
