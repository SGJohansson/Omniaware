//! Text in images, read with the recogniser built into Windows (`Windows.Media.Ocr`).
//!
//! Recognition never runs inside the resident process. Each image is read by a short-lived
//! `omniaware.exe --ocr <image> <out> <language>` at below-normal priority, and the thread that
//! starts those exists only while there is work: with nothing to read, Omniaware's background
//! cost is what it was without OCR. The text is stored per image (blob) in the database, so Ctrl+K
//! finds it and "copy text" puts it on the clipboard.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// Exit code of the helper when the language pack has no OCR component.
const NO_LANGUAGE: i32 = 3;
/// A helper that takes longer than this is stopped.
const TIMEOUT: Duration = Duration::from_secs(60);
/// Pause after each image, so a backlog of old pictures is read slowly.
const GAP: Duration = Duration::from_millis(1500);
/// The worker thread ends after this long without work.
const LINGER: Duration = Duration::from_secs(30);

pub enum Done {
    /// The recognised text, possibly empty ("no text in this picture").
    Text(String),
    /// Windows has no OCR for the requested language (or any profile language).
    NoLanguage(String),
    Failed(String),
}

struct Job {
    blob: String,
    image: PathBuf,
}

/// Images go in one at a time; results come back through `results` and a repaint.
pub struct Queue {
    dir: PathBuf,
    language: String,
    wake: std::sync::Arc<dyn Fn() + Send + Sync>,
    tx: Option<Sender<Job>>,
    rx_done: Receiver<(String, Done)>,
    tx_done: Sender<(String, Done)>,
    busy: Option<String>,
}

impl Queue {
    pub fn new(dir: PathBuf, language: String, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let (tx_done, rx_done) = mpsc::channel();
        Self { dir, language, wake: std::sync::Arc::new(wake), tx: None, rx_done, tx_done, busy: None }
    }

    /// The blob being read right now, if any.
    pub fn busy(&self) -> Option<&str> {
        self.busy.as_deref()
    }

    pub fn send(&mut self, blob: String, image: PathBuf) {
        let mut job = Job { blob: blob.clone(), image };
        if let Some(tx) = &self.tx {
            match tx.send(job) {
                Ok(()) => {
                    self.busy = Some(blob);
                    return;
                }
                Err(mpsc::SendError(j)) => job = j, // the worker has ended: start a new one
            }
        }
        let (tx, rx) = mpsc::channel::<Job>();
        let _ = tx.send(job);
        self.tx = Some(tx);
        self.busy = Some(blob);
        let (dir, language, wake, done) = (self.dir.clone(), self.language.clone(), self.wake.clone(), self.tx_done.clone());
        std::thread::spawn(move || {
            loop {
                let job = match rx.recv_timeout(LINGER) {
                    Ok(j) => j,
                    Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return,
                };
                let result = read(&dir, &job.image, &language);
                if done.send((job.blob, result)).is_err() {
                    return;
                }
                wake();
                std::thread::sleep(GAP);
            }
        });
    }

    pub fn results(&mut self) -> Vec<(String, Done)> {
        let out: Vec<_> = self.rx_done.try_iter().collect();
        if !out.is_empty() {
            self.busy = None;
        }
        out
    }
}

/// Runs the helper process for one image.
fn read(dir: &Path, image: &Path, language: &str) -> Done {
    let spool = dir.join("ocr");
    if let Err(e) = std::fs::create_dir_all(&spool) {
        return Done::Failed(format!("{}: {e}", spool.display()));
    }
    let stem = image.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "image".into());
    let out = spool.join(format!("{stem}.txt"));
    let _ = std::fs::remove_file(&out);
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => return Done::Failed(e.to_string()),
    };
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--ocr").arg(image).arg(&out).arg(if language.trim().is_empty() { "-" } else { language.trim() });
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x4000;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(BELOW_NORMAL_PRIORITY_CLASS | CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Done::Failed(format!("ocr helper: {e}")),
    };
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if started.elapsed() > TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Done::Failed(format!("ocr helper: no answer after {} s", TIMEOUT.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Done::Failed(format!("ocr helper: {e}")),
        }
    };
    let text = std::fs::read_to_string(&out).unwrap_or_default();
    let _ = std::fs::remove_file(&out);
    match status.code() {
        Some(0) => Done::Text(text),
        Some(NO_LANGUAGE) => Done::NoLanguage(text),
        c => Done::Failed(if text.is_empty() { format!("ocr helper: exit {c:?}") } else { text }),
    }
}

/// `omniaware --ocr <image> <out> <language|->`: recognise, write UTF-8 text to `out`, exit.
pub fn run_helper(args: &[String]) -> i32 {
    let (Some(image), Some(out)) = (args.first(), args.get(1)) else { return 2 };
    let language = args.get(2).map(String::as_str).filter(|l| *l != "-" && !l.is_empty());
    let (code, text) = match imp::recognise(Path::new(image), language) {
        Ok(t) => (0, t),
        Err(Fail::NoLanguage(m)) => (NO_LANGUAGE, m),
        Err(Fail::Other(m)) => (1, m),
    };
    if std::fs::write(out, text).is_err() {
        return 1;
    }
    code
}

#[cfg_attr(not(windows), allow(dead_code))]
enum Fail {
    NoLanguage(String),
    Other(String),
}

#[cfg(windows)]
mod imp {
    use super::Fail;
    use std::path::Path;
    use windows::Globalization::Language;
    use windows::Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapInterpolationMode, BitmapPixelFormat, BitmapTransform, ColorManagementMode,
        ExifOrientationMode,
    };
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::{FileAccessMode, StorageFile};
    use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
    use windows::core::HSTRING;

    fn other(e: windows::core::Error) -> Fail {
        Fail::Other(e.message().to_string())
    }

    pub(super) fn recognise(image: &Path, language: Option<&str>) -> Result<String, Fail> {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        let engine = match language {
            Some(tag) => {
                let lang = Language::CreateLanguage(&HSTRING::from(tag)).map_err(other)?;
                if !OcrEngine::IsLanguageSupported(&lang).map_err(other)? {
                    return Err(Fail::NoLanguage(format!("no OCR for \"{tag}\"")));
                }
                OcrEngine::TryCreateFromLanguage(&lang)
            }
            None => OcrEngine::TryCreateFromUserProfileLanguages(),
        }
        .map_err(|_| Fail::NoLanguage("no OCR language installed".into()))?;

        // Absolute path: the WinRT file API refuses relative ones.
        let path = std::fs::canonicalize(image).map_err(|e| Fail::Other(e.to_string()))?;
        let path = path.to_string_lossy().trim_start_matches(r"\\?\").to_string();
        let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path)).and_then(|a| a.join()).map_err(other)?;
        let stream = file.OpenAsync(FileAccessMode::Read).and_then(|a| a.join()).map_err(other)?;
        let decoder = BitmapDecoder::CreateAsync(&stream).and_then(|a| a.join()).map_err(other)?;
        let (w, h) = (decoder.PixelWidth().map_err(other)?, decoder.PixelHeight().map_err(other)?);
        let max = OcrEngine::MaxImageDimension().map_err(other)?.max(1);
        // Screen text at 100 % scaling is small for the recogniser: twice the size reads better.
        // Anything over the engine's limit is scaled down to fit.
        let longest = w.max(h).max(1);
        let k = if longest * 2 <= max && longest <= 2560 { 2.0 } else if longest > max { max as f64 / longest as f64 } else { 1.0 };
        let transform = BitmapTransform::new().map_err(other)?;
        if k != 1.0 {
            transform.SetScaledWidth(((w as f64 * k) as u32).max(1)).map_err(other)?;
            transform.SetScaledHeight(((h as f64 * k) as u32).max(1)).map_err(other)?;
            transform.SetInterpolationMode(BitmapInterpolationMode::Fant).map_err(other)?;
        }
        let bitmap = decoder
            .GetSoftwareBitmapTransformedAsync(
                BitmapPixelFormat::Bgra8,
                BitmapAlphaMode::Premultiplied,
                &transform,
                ExifOrientationMode::RespectExifOrientation,
                ColorManagementMode::DoNotColorManage,
            )
            .and_then(|a| a.join())
            .map_err(other)?;
        let result = engine.RecognizeAsync(&bitmap).and_then(|a| a.join()).map_err(other)?;
        let mut text = String::new();
        for line in result.Lines().map_err(other)? {
            let l = line.Text().map_err(other)?.to_string();
            if !l.trim().is_empty() {
                text.push_str(l.trim_end());
                text.push('\n');
            }
        }
        Ok(text.trim_end().to_string())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Fail;
    use std::path::Path;
    pub(super) fn recognise(_image: &Path, _language: Option<&str>) -> Result<String, Fail> {
        Err(Fail::Other("text recognition is Windows only".into()))
    }
}
