//! Bounded microphone capture and local Whisper transcription. The WAV and
//! transcript exist only in a private temporary directory for one request.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const RATE: u32 = 16_000;
const MAX_SECONDS: usize = 120;
const MAX_TEXT: u64 = 128 * 1024;

pub struct Recording {
    stream: cpal::Stream,
    samples: Arc<Mutex<Vec<i16>>>,
    sample_rate: u32,
    failed: Arc<AtomicBool>,
}

impl Recording {
    pub fn start() -> Result<Self> {
        let device = cpal::default_host()
            .default_input_device()
            .context("No microphone is available")?;
        let format = device
            .default_input_config()
            .context("No supported microphone format")?;
        let config = format.config();
        let channels = usize::from(config.channels);
        let sample_rate = config.sample_rate.0;
        if channels == 0 || !(8_000..=384_000).contains(&sample_rate) {
            bail!("Unsupported microphone configuration");
        }
        let maximum = sample_rate as usize * MAX_SECONDS;
        let samples = Arc::new(Mutex::new(Vec::new()));
        let failed = Arc::new(AtomicBool::new(false));
        let target = |failed: Arc<AtomicBool>| move |_| failed.store(true, Ordering::Relaxed);
        macro_rules! input {
            ($ty:ty, $convert:expr) => {{
                let buffer = samples.clone();
                device.build_input_stream(
                    &config,
                    move |input: &[$ty], _| {
                        if let Ok(mut output) = buffer.lock() {
                            let remaining = maximum.saturating_sub(output.len());
                            output.extend(input.chunks_exact(channels).take(remaining).map(
                                |frame| {
                                    let convert: fn($ty) -> f32 = $convert;
                                    let mono =
                                        frame.iter().map(|sample| convert(*sample)).sum::<f32>()
                                            / channels as f32;
                                    (mono.clamp(-1., 1.) * f32::from(i16::MAX)) as i16
                                },
                            ));
                        }
                    },
                    target(failed.clone()),
                    None,
                )?
            }};
        }
        let stream = match format.sample_format() {
            cpal::SampleFormat::F32 => input!(f32, |v| v),
            cpal::SampleFormat::I16 => input!(i16, |v| f32::from(v) / 32768.),
            cpal::SampleFormat::U16 => input!(u16, |v| (f32::from(v) - 32768.) / 32768.),
            _ => bail!("Unsupported microphone sample format"),
        };
        stream.play()?;
        Ok(Self {
            stream,
            samples,
            sample_rate,
            failed,
        })
    }

    pub fn stop(self) -> Result<Audio> {
        drop(self.stream);
        if self.failed.load(Ordering::Relaxed) {
            bail!("Microphone capture was interrupted");
        }
        let samples = std::mem::take(
            &mut *self
                .samples
                .lock()
                .map_err(|_| anyhow::anyhow!("Microphone buffer unavailable"))?,
        );
        if samples.is_empty() {
            bail!("No audio was captured");
        }
        Ok(Audio {
            samples,
            sample_rate: self.sample_rate,
        })
    }
}

pub struct Audio {
    samples: Vec<i16>,
    sample_rate: u32,
}

impl Audio {
    fn write_wav(&self, mut out: impl Write) -> std::io::Result<()> {
        let count = self.samples.len() as u64 * u64::from(RATE) / u64::from(self.sample_rate);
        let bytes = count as u32 * 2;
        out.write_all(b"RIFF")?;
        out.write_all(&(36 + bytes).to_le_bytes())?;
        out.write_all(b"WAVEfmt \x10\0\0\0\x01\0\x01\0")?;
        out.write_all(&RATE.to_le_bytes())?;
        out.write_all(&(RATE * 2).to_le_bytes())?;
        out.write_all(b"\x02\0\x10\0data")?;
        out.write_all(&bytes.to_le_bytes())?;
        for i in 0..count {
            let offset = i * u64::from(self.sample_rate);
            let index = (offset / u64::from(RATE)) as usize;
            let fraction = (offset % u64::from(RATE)) as i64;
            let left = i64::from(self.samples[index]);
            let right = i64::from(*self.samples.get(index + 1).unwrap_or(&self.samples[index]));
            let sample = (left + (right - left) * fraction / i64::from(RATE)) as i16;
            out.write_all(&sample.to_le_bytes())?;
        }
        Ok(())
    }
}

pub fn model_path() -> Result<PathBuf> {
    Ok(crate::AppDirs::for_identity(crate::Identity::current())?
        .data
        .join("dictation/ggml-model.bin"))
}

pub fn install_model(source: &Path) -> Result<PathBuf> {
    let size = source.metadata()?.len();
    if !(1_000_000..=2_000_000_000).contains(&size) {
        bail!("Select a whisper.cpp GGML model between 1 MB and 2 GB");
    }
    let mut input = std::fs::File::open(source)?;
    let mut magic = [0; 4];
    input.read_exact(&mut magic)?;
    if ![b"lmgg".as_slice(), b"ggml".as_slice()].contains(&magic.as_slice()) {
        bail!("This is not a whisper.cpp GGML model");
    }
    let dest = model_path()?;
    std::fs::create_dir_all(dest.parent().context("model directory missing")?)?;
    let mut temp = tempfile::NamedTempFile::new_in(dest.parent().unwrap())?;
    use std::io::{Seek as _, SeekFrom};
    input.seek(SeekFrom::Start(0))?;
    let copied = std::io::copy(&mut input.take(2_000_000_001), &mut temp)?;
    if copied != size {
        bail!("Model file changed while being copied");
    }
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(&dest).map_err(|error| error.error)?;
    Ok(dest)
}

pub fn configured_model() -> Option<PathBuf> {
    std::env::var_os("DRIFTING_WHISPER_MODEL")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| model_path().ok().filter(|path| path.is_file()))
}

pub fn transcribe(audio: Audio, model: &Path, cancelled: &AtomicBool) -> Result<String> {
    if cancelled.load(Ordering::Relaxed) {
        bail!("Dictation cancelled");
    }
    let dir = tempfile::tempdir()?;
    let wav = dir.path().join("voice.wav");
    let output = dir.path().join("result");
    audio.write_wav(std::io::BufWriter::new(std::fs::File::create(&wav)?))?;
    let program = std::env::var_os("DRIFTING_WHISPER_CLI").unwrap_or_else(|| "whisper-cli".into());
    let mut cmd = Command::new(program);
    cmd.args(["--no-gpu", "-l", "auto", "-otxt", "-m"])
        .arg(model)
        .arg("-f")
        .arg(wav)
        .arg("-of")
        .arg(&output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd
        .spawn()
        .context("Install whisper.cpp and add whisper-cli to PATH")?;
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        if cancelled.load(Ordering::Relaxed) || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Dictation stopped");
        }
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("Whisper could not transcribe the recording");
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut text = String::new();
    std::fs::File::open(output.with_extension("txt"))?
        .take(MAX_TEXT + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_TEXT {
        bail!("Transcript is too large");
    }
    if cancelled.load(Ordering::Relaxed) {
        bail!("Dictation cancelled");
    }
    Ok(text.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_contains_the_expected_sample_count_and_rate_after_resampling() {
        let audio = Audio {
            samples: (0..48_000).map(|n| (n % 20_000) as i16).collect(),
            sample_rate: 48_000,
        };
        let mut wav = Vec::new();
        audio.write_wav(&mut wav).unwrap();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 16_000);
        assert_eq!(wav.len(), 44 + 32_000);
        assert_eq!(i16::from_le_bytes(wav[46..48].try_into().unwrap()), 3);
    }
}
