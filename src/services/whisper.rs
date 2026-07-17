//! Сервис распознавания речи через Whisper.
//!
//! Аудио копится в буфер, при завершении весь буфер отдаётся Whisper'у за один проход.
//! Модель и DLL подгружаются через whisper_ffi + whisper_setup.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::Mutex;

use crate::core::whisper_ffi::{self, WhisperDll, i16_to_f32};

#[derive(Clone, Debug)]
pub enum WhisperEvent {
    Final { text: String, source: String },
    /// Команда UI-потоку загрузить модель в Whisper.
    LoadRequest { dll_dir: PathBuf, model_path: PathBuf },
    Status(String),
    /// Прогресс скачивания/распаковки.
    Progress {
        /// Имя файла/этапа (например, "ggml-small.bin" или "whisper.dll").
        name: String,
        /// Скачано байт.
        downloaded: u64,
        /// Полный размер в байтах (0, если неизвестен).
        total: u64,
        /// Текст этапа ("download", "unpack", "done", "resume").
        stage: String,
    },
    Error(String),
}

enum WorkerMsg {
    Load { dll_dir: PathBuf, model_path: PathBuf },
    Unload,
    Finalize { samples: Vec<i16>, source: String },
    Shutdown,
}

pub struct WhisperService {
    cmd_tx: Sender<WorkerMsg>,
    event_tx: Arc<Mutex<Sender<WhisperEvent>>>,
    loaded: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl WhisperService {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = unbounded();
        let (event_tx, _) = unbounded();
        let event_tx = Arc::new(Mutex::new(event_tx));
        let loaded = Arc::new(AtomicBool::new(false));

        let worker_tx = Arc::clone(&event_tx);
        let worker_loaded = Arc::clone(&loaded);

        let worker = thread::Builder::new()
            .name("whisper-worker".into())
            .spawn(move || whisper_worker(cmd_rx, worker_tx, worker_loaded))
            .expect("spawn whisper worker");

        Self {
            cmd_tx,
            event_tx,
            loaded,
            worker: Some(worker),
        }
    }

    pub fn is_loaded(&self) -> bool { self.loaded.load(Ordering::SeqCst) }
    pub fn load_async(&self, dll_dir: PathBuf, model_path: PathBuf, event_tx: Sender<WhisperEvent>) {
        *self.event_tx.lock() = event_tx;
        self.loaded.store(false, Ordering::SeqCst);
        let _ = self.cmd_tx.send(WorkerMsg::Load { dll_dir, model_path });
    }

    pub fn unload(&self) {
        self.loaded.store(false, Ordering::SeqCst);
        let _ = self.cmd_tx.send(WorkerMsg::Unload);
    }

    /// Завершить распознавание с накопленным буфером.
    pub fn finalize(&self, samples: Vec<i16>, source: &str) {
        let _ = self.cmd_tx.send(WorkerMsg::Finalize {
            samples,
            source: source.into(),
        });
    }
}

impl Drop for WhisperService {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(WorkerMsg::Shutdown);
        if let Some(worker) = self.worker.take() {
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            thread::spawn(move || {
                let _ = worker.join();
                let _ = done_tx.send(());
            });
            let _ = done_rx.recv_timeout(std::time::Duration::from_secs(3));
        }
    }
}

impl Default for WhisperService {
    fn default() -> Self { Self::new() }
}

// ============================================================================
// Worker thread
// ============================================================================

fn whisper_worker(
    cmd_rx: Receiver<WorkerMsg>,
    event_tx: Arc<Mutex<Sender<WhisperEvent>>>,
    loaded: Arc<AtomicBool>,
) {
    let mut dll: Option<WhisperDll> = None;
    let mut ctx: *mut whisper_ffi::WhisperContext = std::ptr::null_mut();

    let send = |event_tx: &Arc<Mutex<Sender<WhisperEvent>>>, event: WhisperEvent| {
        let tx = event_tx.lock().clone();
        let _ = tx.send(event);
    };

    while let Ok(msg) = cmd_rx.recv() {
        match msg {
            WorkerMsg::Load { dll_dir, model_path } => {
                send(&event_tx, WhisperEvent::Status(
                    format!("Loading Whisper DLL from {}", dll_dir.display())
                ));

                // Проверяем существование модели
                if !model_path.exists() || !model_path.is_file() {
                    send(&event_tx, WhisperEvent::Error(format!(
                        "Whisper model not found: {}", model_path.display()
                    )));
                    loaded.store(false, Ordering::SeqCst);
                    continue;
                }

                // Загружаем DLL
                let dll_obj = match WhisperDll::load(&dll_dir) {
                    Ok(d) => d,
                    Err(e) => {
                        send(&event_tx, WhisperEvent::Error(e));
                        loaded.store(false, Ordering::SeqCst);
                        continue;
                    }
                };

                // Инициализируем контекст
                let model_str = model_path.display().to_string();
                let ctx_ptr = unsafe {
                    match dll_obj.init_context(&model_str) {
                        Ok(c) => c,
                        Err(e) => {
                            send(&event_tx, WhisperEvent::Error(e));
                            loaded.store(false, Ordering::SeqCst);
                            continue;
                        }
                    }
                };

                dll = Some(dll_obj);
                ctx = ctx_ptr;
                loaded.store(true, Ordering::SeqCst);
                send(&event_tx, WhisperEvent::Status("Whisper ready".into()));
            }
            WorkerMsg::Unload => {
                if !ctx.is_null() {
                    if let Some(ref d) = dll {
                        unsafe { d.free_context(ctx); }
                    }
                    ctx = std::ptr::null_mut();
                }
                dll = None;
                loaded.store(false, Ordering::SeqCst);
            }
            WorkerMsg::Finalize { samples, source } => {
                if ctx.is_null() || dll.is_none() {
                    send(&event_tx, WhisperEvent::Error("Whisper not loaded".into()));
                    continue;
                }

                let dll_ref = dll.as_ref().unwrap();

                // Конвертируем i16 в f32
                let f32_samples = i16_to_f32(&samples);

                // Параметры распознавания
                let mut params = unsafe { dll_ref.default_params(whisper_ffi::WhisperSamplingStrategy::Greedy) };

                // Последние сборки whisper.cpp используют C++,
                // whisper_full_default_params_by_ref вернёт корректные defaults.
                // Устанавливаем русский язык.
                let lang_c = std::ffi::CString::new("ru").unwrap();
                params.language = lang_c.as_ptr();
                params.n_threads = 4;
                params.no_timestamps = 1;

                // Запускаем инференс
                match unsafe { dll_ref.run_full(ctx, params, &f32_samples) } {
                    Ok(_) => {
                        let n = unsafe { dll_ref.n_segments(ctx) };
                        let mut text = String::new();
                        for i in 0..n {
                            let seg = unsafe { dll_ref.segment_text(ctx, i) };
                            if !seg.is_empty() {
                                if !text.is_empty() { text.push(' '); }
                                text.push_str(&seg);
                            }
                        }
                        if !text.is_empty() {
                            send(&event_tx, WhisperEvent::Final { text, source });
                        }
                    }
                    Err(e) => {
                        send(&event_tx, WhisperEvent::Error(e));
                    }
                }
            }
            WorkerMsg::Shutdown => {
                if !ctx.is_null() {
                    if let Some(ref d) = dll {
                        unsafe { d.free_context(ctx); }
                    }
                }
                break;
            }
        }
    }
}