//! Local image backgrounds and frosted surfaces. Original bytes never sync.

use anyhow::{Context as _, Result, bail};
use dr_i18n::keys::{background as copy, common};
use dr_shell::{Settings, t};
use dr_ui::{
    Theme,
    components::{Button, Range},
    glass::{self, Ground},
};
use gpui::{prelude::*, *};
use serde::{Deserialize, Serialize};
use std::{
    io::{Cursor, Read as _},
    path::{Path, PathBuf},
};

const KEY: &str = "appearance.background.v1";
const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Builtin,
    Solid,
    Image,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    mode: Mode,
    original: Option<String>,
    preview: Option<String>,
    blur: u8,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Builtin,
            original: None,
            preview: None,
            blur: 12,
        }
    }
}

fn config(cx: &App) -> Config {
    Settings::device_memory(cx, KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}
fn persist(config: &Config, cx: &mut App) {
    cx.set_global(glass::FrostedSurfaces(config.mode == Mode::Image));
    Settings::set_device_memory(cx, KEY, serde_json::to_value(config).ok());
    Settings::flush(cx);
    cx.refresh_windows();
}
pub fn install(cx: &mut App) {
    let restored = Settings::device_memory(cx, KEY)
        .and_then(|value| serde_json::from_value::<Config>(value).ok())
        .unwrap_or_default();
    let mode = restored.mode;
    cx.set_global(glass::FrostedSurfaces(mode == Mode::Image));
    if mode == Mode::Image && preview_path(cx).is_none() {
        log::warn!(
            "the selected custom background is missing; using the solid background until it is replaced"
        );
        let mut fallback = restored;
        fallback.mode = Mode::Solid;
        Settings::set_device_memory(cx, KEY, serde_json::to_value(fallback).ok());
        Settings::flush(cx);
    }
}

pub fn mode(cx: &App) -> Mode {
    config(cx).mode
}
pub fn is_custom(cx: &App) -> bool {
    mode(cx) == Mode::Image
}

fn directory(cx: &App) -> Result<PathBuf> {
    Settings::read(cx)
        .path()
        .and_then(Path::parent)
        .map(|path| path.join("backgrounds"))
        .or_else(|| {
            dr_platform::AppDirs::for_identity(dr_platform::Identity::current())
                .ok()
                .map(|dirs| dirs.data.join("backgrounds"))
        })
        .context("configuration directory is unavailable")
}

#[derive(Clone)]
struct Store(PathBuf);
impl Store {
    fn path(&self, name: &str) -> Result<PathBuf> {
        if name.is_empty()
            || name.len() > 100
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-.".contains(&c))
            || name.contains("..")
        {
            bail!("invalid background image name");
        }
        Ok(self.0.join(name))
    }

    fn install(&self, source: &Path, blur: u8) -> Result<(String, String)> {
        let file = std::fs::File::open(source)?;
        if file.metadata()?.len() > MAX_BYTES {
            bail!("image is larger than 32 MiB");
        }
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            bail!("image is larger than 32 MiB");
        }
        let format = image::guess_format(&bytes)?;
        let ext = match format {
            image::ImageFormat::Png => "png",
            image::ImageFormat::Jpeg => "jpg",
            image::ImageFormat::WebP => "webp",
            _ => bail!("choose a PNG, JPEG or WebP image"),
        };
        decode(&bytes)?;
        std::fs::create_dir_all(&self.0)?;
        let original = format!("{}.{}", dr_store::digest::sha256_hex(&bytes), ext);
        let path = self.path(&original)?;
        if !path.is_file() {
            dr_store::file_io::atomic_write(&path, &bytes)?;
        }
        let preview = self.preview(&original, blur)?;
        Ok((original, preview))
    }

    fn preview(&self, original: &str, blur: u8) -> Result<String> {
        let path = self.path(original)?;
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_BYTES {
            bail!("background original exceeds limit");
        }
        let hash = dr_store::digest::sha256_hex(&bytes);
        if !original.starts_with(&hash) {
            bail!("background original checksum changed");
        }
        let preview = format!("{hash}-blur-{}.png", blur.min(40));
        let destination = self.path(&preview)?;
        if destination.is_file() {
            return Ok(preview);
        }
        let image = decode(&bytes)?.thumbnail(2560, 2560);
        let image = if blur == 0 {
            image
        } else {
            image.blur(f32::from(blur.min(40)))
        };
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Png)?;
        dr_store::file_io::atomic_write(&destination, &out.into_inner())?;
        Ok(preview)
    }

    fn remove(&self, original: Option<&str>) -> Result<()> {
        let Some(original) = original else {
            return Ok(());
        };
        let Some((hash, _)) = original.split_once('.') else {
            bail!("invalid background filename");
        };
        if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            bail!("invalid background filename");
        }
        for item in std::fs::read_dir(&self.0)? {
            let item = item?;
            let name = item.file_name().to_string_lossy().into_owned();
            if name == original || name.starts_with(&format!("{hash}-blur-")) {
                std::fs::remove_file(self.path(&name)?)?;
            }
        }
        Ok(())
    }
}

fn decode(bytes: &[u8]) -> Result<image::DynamicImage> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?;
    if u64::from(image.width()) * u64::from(image.height()) > 32_000_000 {
        bail!("background dimensions exceed limit");
    }
    Ok(image)
}

fn preview_path(cx: &App) -> Option<PathBuf> {
    let config = config(cx);
    let path = Store(directory(cx).ok()?)
        .path(config.preview.as_deref()?)
        .ok()?;
    path.is_file().then_some(path)
}

pub fn draw(window: &mut Window, cx: &mut App) -> AnyElement {
    if config(cx).mode == Mode::Image && preview_path(cx).is_none() {
        let mut fallback = config(cx);
        fallback.mode = Mode::Solid;
        persist(&fallback, cx);
    }
    let p = Theme::of(window, cx).palette;
    let color = p.desk.solid();
    div()
        .absolute()
        .inset_0()
        .bg(color)
        .when_some(
            (mode(cx) == Mode::Image)
                .then(|| preview_path(cx))
                .flatten(),
            |el, path| {
                el.child(img(path).size_full().object_fit(ObjectFit::Cover))
                    .child(div().absolute().inset_0().bg(p.desk.alpha(0.18)))
            },
        )
        .into_any_element()
}

pub fn solid(window: &mut Window, cx: &mut App) -> AnyElement {
    div()
        .absolute()
        .inset_0()
        .bg(Theme::of(window, cx).palette.desk.solid())
        .into_any_element()
}

/// Renders only the surface at reduced opacity; text and controls stay opaque.
#[derive(IntoElement)]
pub struct Frosted {
    pub id: SharedString,
    pub element: AnyElement,
}
impl RenderOnce for Frosted {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let p = Theme::of(window, cx).palette;
        let background = div()
            .relative()
            .flex_none()
            .overflow_hidden()
            .rounded(px(5.0))
            .when(self.id.as_ref() == "sea-bottom", |el| el.w_full())
            .when(self.id.as_ref() != "sea-bottom", |el| el.h_full())
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(p.desk.alpha(if p.dark { 0.56 } else { 0.34 })),
            )
            .child(self.element);
        glass::surface(Ground::Uneven, background)
    }
}

pub fn settings(window: &mut Window, cx: &mut App) -> AnyElement {
    let panel = window.use_keyed_state("background-settings", cx, |_, cx| Panel {
        generation: 0,
        busy: false,
        failed: false,
        _settings: cx.observe_global::<Settings>(|_, cx| cx.notify()),
    });
    panel.into_any_element()
}

struct Panel {
    generation: u64,
    busy: bool,
    failed: bool,
    _settings: Subscription,
}
impl Panel {
    fn choose(&mut self, cx: &mut Context<Self>) {
        let prompt = t(cx, copy::PICK_IMAGE);
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(prompt),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(source) = paths.into_iter().next() else {
                return;
            };
            this.update(cx, |panel, cx| panel.prepare(Some(source), None, cx))
                .ok();
        })
        .detach();
    }

    fn prepare(&mut self, source: Option<PathBuf>, blur: Option<u8>, cx: &mut Context<Self>) {
        let Ok(root) = directory(cx) else {
            self.failed = true;
            cx.notify();
            return;
        };
        self.generation += 1;
        let generation = self.generation;
        self.busy = true;
        self.failed = false;
        let before = config(cx);
        let blur = blur.unwrap_or(before.blur);
        let original = before.original.clone();
        cx.spawn(async move |this, cx| {
            // Slider motion is coalesced before CPU image processing.
            cx.background_executor()
                .timer(std::time::Duration::from_millis(120))
                .await;
            if !this
                .read_with(cx, |panel, _| panel.generation == generation)
                .unwrap_or(false)
            {
                return;
            }
            let installed = cx
                .background_spawn(async move {
                    let store = Store(root);
                    if let Some(source) = source {
                        store.install(&source, blur)
                    } else {
                        let original = original.context("no image selected")?;
                        let preview = store.preview(&original, blur)?;
                        Ok((original, preview))
                    }
                })
                .await;
            this.update(cx, |panel, cx| {
                if panel.generation != generation {
                    return;
                }
                panel.busy = false;
                match installed {
                    Ok((original, preview)) => {
                        if config(cx) != before {
                            return;
                        }
                        persist(
                            &Config {
                                mode: Mode::Image,
                                original: Some(original),
                                preview: Some(preview),
                                blur,
                            },
                            cx,
                        );
                    }
                    Err(error) => {
                        log::warn!("background import: {error}");
                        panel.failed = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn remove(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        self.busy = true;
        self.failed = false;
        let before = config(cx);
        let root = directory(cx);
        cx.spawn(async move |this, cx| {
            let old = before.clone();
            let result = cx
                .background_spawn(async move { Store(root?).remove(old.original.as_deref()) })
                .await;
            this.update(cx, |panel, cx| {
                panel.busy = false;
                if result.is_ok() && config(cx) == before {
                    persist(
                        &Config {
                            mode: Mode::Solid,
                            ..Config::default()
                        },
                        cx,
                    );
                } else if let Err(error) = result {
                    log::warn!("removing background: {error}");
                    panel.failed = true;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for Panel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = config(cx);
        let selected = settings.mode;
        let me = cx.entity();
        let set_mode = |id, label: dr_i18n::Key, mode| {
            Button::new(id, t(cx, label))
                .small()
                .disabled(selected == mode)
                .on_click(move |_, _, cx| {
                    let mut value = config(cx);
                    value.mode = mode;
                    persist(&value, cx);
                })
        };
        let mut view = div().w_full().flex().flex_col().gap_2().child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(set_mode("background-builtin", copy::BUILTIN, Mode::Builtin))
                .child(set_mode("background-solid", copy::SOLID, Mode::Solid))
                .child(
                    Button::new(
                        "background-custom",
                        t(
                            cx,
                            if settings.original.is_some() {
                                copy::REPLACE
                            } else {
                                copy::CUSTOM
                            },
                        ),
                    )
                    .icon(dr_ui::sand::Icon::ImagePlus)
                    .small()
                    .disabled(self.busy)
                    .on_click({
                        let me = me.clone();
                        move |_, _, cx| me.update(cx, |panel, cx| panel.choose(cx))
                    }),
                ),
        );
        if settings.original.is_some() && selected == Mode::Image {
            view = view
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(set_mode(
                            "background-use-image",
                            copy::USE_IMAGE,
                            Mode::Image,
                        ))
                        .child(
                            Button::new("background-remove", t(cx, common::DELETE))
                                .small()
                                .disabled(self.busy)
                                .on_click({
                                    let me = me.clone();
                                    move |_, _, cx| me.update(cx, |panel, cx| panel.remove(cx))
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(t(cx, copy::BLUR))
                        .child(
                            Range::new("background-blur", f64::from(settings.blur), 0., 40.)
                                .label(t(cx, copy::BLUR))
                                .on_change({
                                    let me = me.clone();
                                    move |value, _, cx| {
                                        me.update(cx, |panel, cx| {
                                            panel.prepare(None, Some(value.round() as u8), cx)
                                        })
                                    }
                                }),
                        ),
                );
        }
        if let Some(path) = preview_path(cx) {
            view = view.child(
                div()
                    .w_full()
                    .h(px(120.))
                    .overflow_hidden()
                    .rounded(px(8.))
                    .child(img(path).size_full().object_fit(ObjectFit::Cover)),
            );
        }
        let p = Theme::of(window, cx).palette;
        view.when(self.busy, |el| el.child(t(cx, copy::PREPARING)))
            .when(self.failed, |el| {
                el.child(
                    div()
                        .text_color(p.danger.solid())
                        .child(t(cx, copy::FAILED)),
                )
            })
            .when(
                settings.original.is_some() && preview_path(cx).is_none(),
                |el| el.child(t(cx, copy::MISSING)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn source_copy_preview_blur_and_removal_stay_in_the_config_directory() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("picked.png");
        let mut image = image::RgbaImage::new(32, 32);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgba([if (x + y) % 2 == 0 { 0 } else { 255 }, 20, 80, 255]);
        }
        image.save(&source).unwrap();
        let store = Store(dir.path().join("config/backgrounds"));
        let (original, preview) = store.install(&source, 12).unwrap();
        assert_eq!(
            std::fs::read(&source).unwrap(),
            std::fs::read(store.path(&original).unwrap()).unwrap()
        );
        assert!(store.path(&preview).unwrap().is_file());
        let sharp = store.preview(&original, 0).unwrap();
        assert_ne!(
            std::fs::read(store.path(&sharp).unwrap()).unwrap(),
            std::fs::read(store.path(&preview).unwrap()).unwrap()
        );
        assert!(store.path("../private.png").is_err());
        store.remove(Some(&original)).unwrap();
        assert!(source.is_file());
        assert!(!store.path(&original).unwrap().exists());
        assert!(!store.path(&preview).unwrap().exists());
    }

    #[test]
    fn unsupported_corrupt_or_changed_images_are_rejected_without_replacing_source() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("invalid.png");
        std::fs::write(&source, b"not an image").unwrap();
        let store = Store(dir.path().join("backgrounds"));
        assert!(store.install(&source, 12).is_err());
        let valid = dir.path().join("valid.png");
        image::RgbaImage::new(4, 4).save(&valid).unwrap();
        let (original, _) = store.install(&valid, 0).unwrap();
        std::fs::write(store.path(&original).unwrap(), b"corrupt").unwrap();
        assert!(store.preview(&original, 15).is_err());
    }
}
