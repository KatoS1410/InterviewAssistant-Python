//! Автоскачивание whisper.dll и моделей с HuggingFace.
//!
//! Скачивание:
//! - Потоковое (chunk-by-chunk), не грузим весь файл в память.
//! - С поддержкой HTTP Range: если файл `.partial` уже есть, докачка продолжится.
//! - С эмитом прогресс-эвентов через `Sender<WhisperEvent>` (UI и логи).
//!
//! Хранилище: `~/.katos_whisper/` (см. `helpers::whisper_dir`).

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;

use crate::core::helpers::whisper_dir;
use crate::services::whisper::WhisperEvent;

/// Размер буфера чтения чанков из сети: 128 KB.
const CHUNK_SIZE: usize = 128 * 1024;
/// Минимальный интервал между прогресс-эвентами: 250 мс.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Какие модели умеем качать.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelKind {
    Small,
    Medium,
    SmallEn,
    MediumEn,
}

impl ModelKind {
    pub fn url(&self) -> &'static str {
        match self {
            ModelKind::Small => "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
            ModelKind::Medium => "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
            ModelKind::SmallEn => "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin",
            ModelKind::MediumEn => "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.en.bin",
        }
    }

    pub fn file_name(&self) -> &'static str {
        match self {
            ModelKind::Small => "ggml-small.bin",
            ModelKind::Medium => "ggml-medium.bin",
            ModelKind::SmallEn => "ggml-small.en.bin",
            ModelKind::MediumEn => "ggml-medium.en.bin",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ModelKind::Small => "Whisper Small Model (RU) ~ 500 MB",
            ModelKind::Medium => "Whisper Medium Model (RU) ~ 1.5 GB",
            ModelKind::SmallEn => "Whisper Small Model (EN) ~ 500 MB",
            ModelKind::MediumEn => "Whisper Medium Model (EN) ~ 1.5 GB",
        }
    }

    /// Ожидаемый полный размер файла в байтах (используется как fallback,
    /// когда сервер не прислал Content-Length).
    pub fn expected_size(&self) -> u64 {
        match self {
            ModelKind::Small | ModelKind::SmallEn => 500 * 1024 * 1024,
            ModelKind::Medium | ModelKind::MediumEn => 1_500 * 1024 * 1024,
        }
    }
}

/// Возвращает путь к временному файлу `.partial` рядом с финальным.
fn partial_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".partial");
    PathBuf::from(s)
}

/// Форматирует размер в человекочитаемом виде.
pub fn human_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Качает whisper.dll в `whisper_dir()`, если её там ещё нет.
/// В процессе шлёт статус/прогресс-эвенты и продвигается поэтапно.
pub fn ensure_whisper_dll(tx: &Sender<WhisperEvent>) -> Result<(), anyhow::Error> {
    let dir = whisper_dir();
    fs::create_dir_all(&dir)?;
    let final_path = dir.join("whisper.dll");

    if final_path.is_file() {
        let _ = tx.send(WhisperEvent::Status("whisper.dll уже на месте".into()));
        return Ok(());
    }

    let _ = tx.send(WhisperEvent::Status("Скачиваю whisper-bin-x64.zip...".into()));

    let asset_url =
        "https://github.com/ggml-org/whisper.cpp/releases/latest/download/whisper-bin-x64.zip";
    let zip_path = dir.join("whisper-bin-x64.zip");

    // Качаем архив в `.partial` с прогрессом.
    download_file(asset_url, &zip_path, "whisper-bin-x64.zip", tx)?;

    let _ = tx.send(WhisperEvent::Status("Распаковываю whisper.dll из архива...".into()));

    // Распаковываем
    let file = fs::File::open(&zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut extracted = 0usize;
    let total = archive.len();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let outpath = match entry.enclosed_name() {
            Some(p) => dir.join(p.file_name().unwrap_or_default()),
            None => continue,
        };
        if entry.is_dir() {
            continue;
        }
        let mut outfile = fs::File::create(&outpath)?;
        io::copy(&mut entry, &mut outfile)?;
        extracted += 1;
        let _ = tx.send(WhisperEvent::Progress {
            name: "whisper.dll".into(),
            downloaded: extracted as u64,
            total: total as u64,
            stage: "unpack".into(),
        });
    }
    let _ = fs::remove_file(&zip_path);

    if !final_path.is_file() {
        return Err(anyhow::anyhow!(
            "whisper.dll not found after extraction (extracted {} files)",
            extracted
        ));
    }

    let _ = tx.send(WhisperEvent::Progress {
        name: "whisper.dll".into(),
        downloaded: 1,
        total: 1,
        stage: "done".into(),
    });
    let _ = tx.send(WhisperEvent::Status(
        "whisper.dll готов (извлечён из zip)".into(),
    ));
    Ok(())
}

/// Качает модель `kind` (если её нет). Возвращает полный путь.
/// Шлёт прогресс-эвенты и поддерживает докачку через HTTP Range + `.partial`.
pub fn download_model(
    kind: ModelKind,
    tx: &Sender<WhisperEvent>,
) -> Result<PathBuf, anyhow::Error> {
    let dir = whisper_dir();
    fs::create_dir_all(&dir)?;
    let final_path = dir.join(kind.file_name());

    if final_path.is_file() {
        let size = fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
        let _ = tx.send(WhisperEvent::Status(format!(
            "Модель {} уже скачана ({})",
            kind.file_name(),
            human_size(size)
        )));
        return Ok(final_path);
    }

    let _ = tx.send(WhisperEvent::Status(format!(
        "Скачиваю {} ({})...",
        kind.file_name(),
        human_size(kind.expected_size())
    )));

    download_file(kind.url(), &final_path, kind.file_name(), tx)?;

    let size = fs::metadata(&final_path).map(|m| m.len()).unwrap_or(0);
    let _ = tx.send(WhisperEvent::Progress {
        name: kind.file_name().into(),
        downloaded: size,
        total: size,
        stage: "done".into(),
    });
    let _ = tx.send(WhisperEvent::Status(format!(
        "Скачано: {} ({})",
        kind.file_name(),
        human_size(size)
    )));
    Ok(final_path)
}

/// Потоковое скачивание с поддержкой HTTP Range и прогресса.
///
/// Алгоритм:
/// 1. Проверяем наличие `.partial` рядом с `dest`.
///    Если есть — отправляем `Range: bytes={downloaded}-` для докачки.
/// 2. Открываем `.partial` в append (если resume) или create (если чистый старт).
/// 3. Поточно читаем чанки по 128 KB, дописываем в файл, обновляем счётчик.
/// 4. Раз в `PROGRESS_INTERVAL` (или при смене процента) шлём `WhisperEvent::Progress`.
/// 5. По завершении: rename `.partial` → `dest`.
fn download_file(
    url: &str,
    dest: &Path,
    name: &str,
    tx: &Sender<WhisperEvent>,
) -> Result<(), anyhow::Error> {
    let partial = partial_path(dest);
    let already: u64 = if partial.is_file() {
        fs::metadata(&partial).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };

    let client = reqwest::blocking::Client::builder()
        .user_agent("interview-assistant")
        .timeout(Duration::from_secs(60 * 60)) // 1 час на огромный файл
        .build()?;

    let mut req = client.get(url);
    if already > 0 {
        let pct = 0; // вычислим после того как узнаем total
        req = req.header(reqwest::header::RANGE, format!("bytes={already}-"));
        let _ = tx.send(WhisperEvent::Progress {
            name: name.to_string(),
            downloaded: already,
            total: 0,
            stage: "resume".into(),
        });
        let _ = tx.send(WhisperEvent::Status(format!(
            "Продолжаю {} с {}{}%",
            name,
            if pct > 0 { format!("~") } else { String::new() },
            pct
        )));
    }

    let mut resp = req.send()?;
    let status = resp.status();

    let mut total: u64 = resp.content_length().unwrap_or(0);
    let mut doing_resume = false;

    if status == reqwest::StatusCode::PARTIAL_CONTENT {
        // 206 — сервер поддержал Range. Total = already + Content-Length.
        total = already.saturating_add(total);
        doing_resume = true;
    } else if status.is_success() {
        // 200 — сервер НЕ поддержал resume. Удалим старый `.partial`, начнём заново.
        if already > 0 {
            let _ = fs::remove_file(&partial);
        }
    } else {
        return Err(anyhow::anyhow!("HTTP {} at {}", status, url));
    }

    if already > 0 && doing_resume {
        if total > 0 {
            let pct = (already * 100) / total;
            let _ = tx.send(WhisperEvent::Status(format!(
                "Продолжаю {} с {}% ({} из {})",
                name,
                pct,
                human_size(already),
                human_size(total)
            )));
        } else {
            let _ = tx.send(WhisperEvent::Status(format!(
                "Продолжаю {} с {}",
                name,
                human_size(already)
            )));
        }
    }

    // Открываем файл: append при resume, create при старте с нуля.
    let mut file = if doing_resume && already > 0 {
        fs::OpenOptions::new().append(true).open(&partial)?
    } else {
        fs::File::create(&partial)?
    };

    let mut downloaded: u64 = already;
    let mut last_emit = Instant::now() - PROGRESS_INTERVAL; // чтобы первый chunk прошёл
    let mut last_pct: i64 = -1;
    let mut buf = [0u8; CHUNK_SIZE];

    loop {
        let n = resp.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
        downloaded = downloaded.saturating_add(n as u64);

        let should_emit = last_emit.elapsed() >= PROGRESS_INTERVAL;
        let mut cur_pct: i64 = -1;
        if total > 0 {
            cur_pct = (downloaded * 100 / total) as i64;
        }
        if should_emit || cur_pct != last_pct {
            last_emit = Instant::now();
            last_pct = cur_pct;
            let _ = tx.send(WhisperEvent::Progress {
                name: name.to_string(),
                downloaded,
                total,
                stage: "download".into(),
            });
        }
    }

    file.flush()?;
    file.sync_all()?;
    drop(file);

    // Атомарно переименовываем `.partial` → итоговый файл.
    fs::rename(&partial, dest)?;

    // Финальный эвент прогресса со 100% (если знаем total) — полезно для UI.
    let final_total = if total > 0 {
        total
    } else {
        fs::metadata(dest).map(|m| m.len()).unwrap_or(downloaded)
    };
    let _ = tx.send(WhisperEvent::Progress {
        name: name.to_string(),
        downloaded: final_total,
        total: final_total,
        stage: "done".into(),
    });

    Ok(())
}
