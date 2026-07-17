use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, unbounded};
use eframe::egui;

use crate::config::{self, AppConfig};
use crate::core::{
    list_input_devices, timestamp, to_int, trim_entries, trim_lines, SingleInstanceGuard,
    whisper_setup,
};
use crate::services::{
    ai::{spawn_ai_request, AiEvent, AiSession},
    audio::{AudioMode, AudioRecorder, SAMPLE_RATE},
    hotkeys::{HotkeyAction, HotkeyService},
    whisper::{WhisperEvent, WhisperService},
};
use crate::ui::locale::{self, Lang};
use crate::ui::theme::{apply_theme, draw_header, Theme};
use egui::Color32;

const MAX_LOG_LINES: usize = 5000;
const MAX_HISTORY_ENTRIES: usize = 500;

/// Состояние процесса Apply Changes (видно UI).
#[derive(Clone, Debug, Default)]
pub struct ApplyProgress {
    pub active: bool,
    pub stage: String, // "idle" | "dll" | "model" | "load" | "done" | "error"
    pub name: String,
    pub downloaded: u64,
    pub total: u64,
    pub last_log_pct: i64,
}

impl ApplyProgress {
    /// Текущий процент прогресса (0..=100). 0 если total неизвестен.
    pub fn pct(&self) -> i32 {
        if self.total == 0 {
            0
        } else {
            ((self.downloaded as f64 / self.total as f64) * 100.0) as i32
        }
    }
}

pub struct InterviewApp {
    pub cfg: AppConfig,
    pub tab: usize,

    pub transcript: String,
    pub question: String,
    pub prev_question: String,
    pub answer: String,
    pub prev_answer: String,
    pub last_error: String,
    pub logs: String,
    pub history_questions: String,
    pub history_answers: String,
    pub status: String,
    pub transcript_hint: String,
    pub whisper_status: String,
    pub config_preview: String,
    pub chunk_ms_text: String,
    pub auto_ask_text: String,
    pub lang: Lang,
    pub selected_whisper_model: String,

    pub device_names: Vec<String>,
    pub recording: bool,
    pub record_mode: Option<AudioMode>,
    pub active_hold: Option<HotkeySide>,

    audio: AudioRecorder,
    pub whisper: WhisperService,
    ai: Arc<StdMutex<AiSession>>,
    audio_buffer: Vec<i16>,

    audio_tx: Sender<Vec<i16>>,
    audio_rx: Receiver<Vec<i16>>,
    whisper_tx: Sender<WhisperEvent>,
    whisper_rx: Receiver<WhisperEvent>,
    ai_tx: Sender<AiEvent>,
    ai_rx: Receiver<AiEvent>,
    hotkey_rx: Receiver<HotkeyAction>,

    _hotkeys: Option<HotkeyService>,
    _instance: Option<SingleInstanceGuard>,
    auto_ask_deadline: Option<Instant>,
    awaiting_transcript: bool,
    ai_busy: bool,
    stopping: bool,

    pub config_edit_mode: bool,
    pub ai_request_time: Option<Instant>,
    pub big_status: String,
    pub settings_saved_at: Option<Instant>,

    pub apply_progress: ApplyProgress,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HotkeySide {
    Left,
    Right,
}

impl InterviewApp {
    pub fn new(instance: SingleInstanceGuard) -> Self {
        let cfg = config::load();
        let chunk_ms_text = cfg.chunk_ms.to_string();
        let auto_ask_text = cfg.auto_ask_sec.to_string();
        let (audio_tx, audio_rx) = unbounded();
        let (whisper_tx, whisper_rx) = unbounded();
        let (ai_tx, ai_rx) = unbounded();
        let (hotkey_tx, hotkey_rx) = unbounded();

        let ai = Arc::new(StdMutex::new(AiSession::new(cfg.clone())));
        let whisper = WhisperService::new();
        let hotkeys = HotkeyService::install(hotkey_tx).ok();

        let lang = Lang::from_str(&cfg.lang);

        let mut app = Self {
            cfg: cfg.clone(),
            tab: 0,
            transcript: String::new(),
            question: String::new(),
            prev_question: String::new(),
            answer: String::new(),
            prev_answer: String::new(),
            last_error: String::new(),
            logs: String::new(),
            history_questions: String::new(),
            history_answers: String::new(),
            status: locale::t(lang, "header.hint").to_string(),
            transcript_hint: locale::t(lang, "main.live_speech").to_string(),
            whisper_status: locale::t(lang, "whisper.not_loaded").to_string(),
            config_preview: serde_json::to_string(&cfg).unwrap_or_default(),
            chunk_ms_text,
            auto_ask_text,
            lang,
            selected_whisper_model: cfg.whisper_model.clone(),
            device_names: Vec::new(),
            recording: false,
            record_mode: None,
            active_hold: None,

            audio: AudioRecorder::new(),
            whisper,
            ai,
            audio_buffer: Vec::new(),
            audio_tx,
            audio_rx,
            whisper_tx,
            whisper_rx,
            ai_tx,
            ai_rx,
            hotkey_rx,
            _hotkeys: hotkeys,
            _instance: Some(instance),
            auto_ask_deadline: None,
            awaiting_transcript: false,
            ai_busy: false,
            stopping: false,
            config_edit_mode: false,
            ai_request_time: None,
            big_status: String::new(),
            settings_saved_at: None,
            apply_progress: ApplyProgress::default(),
        };

        app.refresh_devices();
        app.log("App initialized");
        app.auto_load_whisper();
        app
    }

    pub fn t(&self, key: &str) -> &'static str {
        locale::t(self.lang, key)
    }

    pub fn log(&mut self, msg: &str) {
        let line = format!("[{}] {msg}\n", timestamp());
        self.logs.push_str(&line);
        trim_lines(&mut self.logs, MAX_LOG_LINES);
    }

    pub fn set_status(&mut self, msg: &str) {
        self.status = msg.to_string();
    }

    pub fn refresh_devices(&mut self) {
        self.device_names = list_input_devices()
            .into_iter()
            .map(|d| d.name)
            .collect();
        if self.cfg.loopback_device.is_empty() {
            if let Some(dev) = crate::core::find_loopback_device() {
                self.cfg.loopback_device = dev.name;
            }
        }
        if self.cfg.mic_device.is_empty() {
            if let Some(dev) = crate::core::find_mic_device() {
                self.cfg.mic_device = dev.name;
            }
        }
        self.set_status(&format!("{} {}", self.t("status.devices"), self.device_names.len()));
        self.log(&format!("Devices refreshed: {}", self.device_names.len()));
        self.refresh_config_preview();
    }

    pub fn detect_loopback(&mut self) {
        if let Some(dev) = crate::core::find_loopback_device() {
            self.cfg.loopback_device = dev.name.clone();
            self.set_status(self.t("misc.loopback_found"));
            self.log(&format!("Loopback: {}", dev.name));
        } else {
            self.set_status(self.t("misc.loopback_not_found"));
            self.log("Loopback not found");
        }
    }

    pub fn detect_mic(&mut self) {
        if let Some(dev) = crate::core::find_mic_device() {
            self.cfg.mic_device = dev.name.clone();
            self.set_status(self.t("misc.mic_found"));
            self.log(&format!("Mic: {}", dev.name));
        } else {
            self.set_status(self.t("misc.mic_not_found"));
            self.log("Mic not found");
        }
    }

    pub fn save_config(&mut self) {
        self.sync_numeric_fields();
        self.cfg.lang = self.lang.as_str().to_string();
        self.cfg.whisper_model = self.selected_whisper_model.clone();
        self.auto_ask_deadline = None;
        if let Err(err) = config::save(&self.cfg) {
            self.log(&format!("Save error: {err}"));
        } else {
            self.log("Config saved");
        }
        self.refresh_config_preview();
    }

    pub fn export_config(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .set_file_name("config.json")
            .save_file()
        {
            self.sync_numeric_fields();
            match config::export_to(&path, &self.cfg) {
                Ok(_) => self.log(&format!("Exported: {}", path.display())),
                Err(err) => self.log(&format!("Export error: {err}")),
            }
        }
    }

    pub fn import_config(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            match config::import_from(&path) {
                Ok(cfg) => {
                    self.lang = Lang::from_str(&cfg.lang);
                    self.cfg = cfg;
                    self.chunk_ms_text = self.cfg.chunk_ms.to_string();
                    self.auto_ask_text = self.cfg.auto_ask_sec.to_string();
                    self.selected_whisper_model = self.cfg.whisper_model.clone();
                    self.refresh_config_preview();
                    self.log(&format!("Imported: {}", path.display()));
                }
                Err(err) => self.log(&format!("Import error: {err}")),
            }
        }
    }

    pub fn refresh_config_preview(&mut self) {
        self.config_preview = serde_json::to_string(&self.cfg).unwrap_or_default();
    }

    fn sync_numeric_fields(&mut self) {
        self.cfg.chunk_ms = to_int(&self.chunk_ms_text, 250).max(20) as u32;
        self.cfg.auto_ask_sec = to_int(&self.auto_ask_text, 0).max(0) as u32;
    }

    fn model_path(&self) -> PathBuf {
        let dir = crate::core::helpers::whisper_dir();
        match self.selected_whisper_model.as_str() {
            "medium" => dir.join("ggml-medium.bin"),
            "small.en" => dir.join("ggml-small.en.bin"),
            "medium.en" => dir.join("ggml-medium.en.bin"),
            _ => dir.join("ggml-small.bin"),
        }
    }

    /// Автозагрузка при старте: если модель скачана — грузим
    fn auto_load_whisper(&mut self) {
        let model_path = self.model_path();
        let dll_dir = crate::core::helpers::whisper_dir();

        if !model_path.exists() {
            self.whisper_status = format!(
                "{} (выберите модель в настройках)",
                model_path.file_name().unwrap_or_default().to_string_lossy()
            );
            self.log(&format!("Whisper model not found: {}", model_path.display()));
            return;
        }

        let tx = self.whisper_tx.clone();
        self.whisper_status = self.t("whisper.loading").to_string();
        self.whisper.load_async(dll_dir, model_path, tx);
        self.log("Whisper: auto-load started");
    }

    /// ЕДИНСТВЕННАЯ кнопка Apply Changes.
    /// Скачивает DLL+модель (если их нет) и загружает в Whisper.
    /// Все шаги логируются и шлют прогресс в UI.
    pub fn apply_whisper_selection(&mut self) {
        self.save_config();
        self.whisper.unload();
        self.apply_progress = ApplyProgress {
            active: true,
            stage: "start".into(),
            ..Default::default()
        };
        self.whisper_status = self.t("settings.applying").to_string();
        self.log("Apply Changes: started");

        let model_name = self.selected_whisper_model.clone();
        let tx = self.whisper_tx.clone();
        let dll_dir = crate::core::helpers::whisper_dir();

        std::thread::spawn(move || {
            // === Шаг 1: whisper.dll ===
            let _ = tx.send(WhisperEvent::Status("Шаг 1/3: проверка whisper.dll".into()));
            match whisper_setup::ensure_whisper_dll(&tx) {
                Ok(_) => {
                    let _ = tx.send(WhisperEvent::Status(
                        "Шаг 1/3: whisper.dll готов".into(),
                    ));
                }
                Err(e) => {
                    let _ = tx.send(WhisperEvent::Error(format!(
                        "whisper.dll: ошибка скачивания/распаковки: {e}"
                    )));
                    return;
                }
            }

            // === Шаг 2: модель ===
            let kind = match model_name.as_str() {
                "medium" => whisper_setup::ModelKind::Medium,
                "small.en" => whisper_setup::ModelKind::SmallEn,
                "medium.en" => whisper_setup::ModelKind::MediumEn,
                _ => whisper_setup::ModelKind::Small,
            };
            let _ = tx.send(WhisperEvent::Status(format!(
                "Шаг 2/3: модель {} (ожидаемый размер: {})",
                kind.file_name(),
                whisper_setup::human_size(kind.expected_size())
            )));
            match whisper_setup::download_model(kind, &tx) {
                Ok(_) => {
                    let _ = tx.send(WhisperEvent::Status(format!(
                        "Шаг 2/3: {} скачана",
                        kind.file_name()
                    )));
                }
                Err(e) => {
                    let _ = tx.send(WhisperEvent::Error(format!(
                        "Модель {}: ошибка скачивания: {e}",
                        kind.file_name()
                    )));
                    return;
                }
            }

            // === Шаг 3: загрузка в Whisper ===
            let _ = tx.send(WhisperEvent::Status(
                "Шаг 3/3: загрузка модели в Whisper...".into(),
            ));
            let model_path = match model_name.as_str() {
                "medium" => dll_dir.join("ggml-medium.bin"),
                "small.en" => dll_dir.join("ggml-small.en.bin"),
                "medium.en" => dll_dir.join("ggml-medium.en.bin"),
                _ => dll_dir.join("ggml-small.bin"),
            };
            let _ = tx.send(WhisperEvent::LoadRequest {
                dll_dir: dll_dir.clone(),
                model_path,
            });
        });
    }

    pub fn test_ai(&mut self) {
        self.save_config();
        let result = self.ai.lock().unwrap().test_connection();
        self.log(&format!(
            "AI test: {} — {}",
            if result.ok { "OK" } else { "FAIL" },
            result.message
        ));
        self.set_status(&result.message);
    }

    pub fn ask_ai(&mut self) {
        if self.ai_busy {
            return;
        }
        let q = self.question.trim().to_string();
        if q.is_empty() {
            self.set_status(self.t("main.no_text"));
            return;
        }
        if self.prev_question != q {
            self.prev_question = self.question.clone();
        }
        if !self.answer.is_empty() && self.answer != self.t("main.thinking") {
            self.prev_answer = self.answer.clone();
        }
        self.save_config();
        self.answer = self.t("main.thinking").to_string();
        self.ai_busy = true;
        self.ai_request_time = Some(Instant::now());
        spawn_ai_request(
            self.cfg.clone(),
            Arc::clone(&self.ai),
            q,
            self.ai_tx.clone(),
        );
        self.set_status(self.t("main.ai_request"));
        self.log("AI request started");
    }

    fn start_recording(&mut self, mode: AudioMode) {
        if self.recording { return; }
        if !self.whisper.is_loaded() {
            self.big_status = self.t("main.whisper_not_loaded").to_string();
            self.set_status(self.t("status.recording_blocked"));
            self.log("Recording blocked: Whisper not loaded");
            return;
        }
        self.save_config();
        self.sync_numeric_fields();
        self.transcript.clear();
        self.audio_buffer.clear();

        let device = match mode {
            AudioMode::Loopback => self.cfg.loopback_device.clone(),
            AudioMode::Mic => self.cfg.mic_device.clone(),
        };

        match self.audio.start(mode, &device, self.cfg.chunk_ms, self.audio_tx.clone()) {
            Ok(_) => {
                self.recording = true;
                self.record_mode = Some(mode);
                self.transcript_hint = match mode {
                    AudioMode::Loopback => self.t("main.source_loopback").to_string(),
                    AudioMode::Mic => self.t("main.source_mic").to_string(),
                };
                self.set_status(self.t("main.recording"));
                self.log(&format!("Recording started: {mode:?} ({device})"));
            }
            Err(err) => {
                self.log(&format!("Audio error: {err}"));
                self.set_status(&err.to_string());
            }
        }
    }

    fn stop_recording(&mut self) {
        if !self.recording { return; }
        self.audio.stop();
        self.recording = false;
        self.stopping = true;
        self.set_status(self.t("main.stopping"));
        self.log("Recording stop signalled");
    }

    fn apply_whisper_event(&mut self, event: WhisperEvent) {
        match event {
            WhisperEvent::Final { text, source } => {
                let line = format!("[{source}] {text}");
                if !self.transcript.is_empty() && !self.transcript.ends_with('\n') {
                    self.transcript.push('\n');
                }
                self.transcript.push_str(&line);
                self.big_status.clear();
                if self.awaiting_transcript {
                    self.awaiting_transcript = false;
                    self.question = self.transcript.clone();
                    self.ask_ai();
                }
            }
            WhisperEvent::LoadRequest { dll_dir, model_path } => {
                let tx = self.whisper_tx.clone();
                let _ = tx.send(WhisperEvent::Status(format!(
                    "Loading model {} into Whisper...",
                    model_path.file_name().unwrap_or_default().to_string_lossy()
                )));
                self.whisper.load_async(dll_dir, model_path, tx);
            }
            WhisperEvent::Status(msg) => {
                self.whisper_status = msg.clone();
                self.log(&msg);
                // Подсказка apply_progress о текущем этапе по строке статуса
                if self.apply_progress.active {
                    if msg.starts_with("Шаг 1") {
                        self.apply_progress.stage = "dll".into();
                    } else if msg.starts_with("Шаг 2") {
                        self.apply_progress.stage = "model".into();
                    } else if msg.starts_with("Шаг 3") {
                        self.apply_progress.stage = "load".into();
                    }
                }
            }
            WhisperEvent::Progress {
                name,
                downloaded,
                total,
                stage,
            } => {
                self.apply_progress.name = name.clone();
                self.apply_progress.downloaded = downloaded;
                self.apply_progress.total = total;
                self.apply_progress.stage = stage.clone();

                let pct = if total > 0 {
                    ((downloaded as f64 / total as f64) * 100.0) as i64
                } else {
                    -1
                };

                // Обновляем whisper_status для отображения в строке состояния
                let label = if pct >= 0 {
                    format!("{}: {}%", name, pct)
                } else if total > 0 {
                    format!("{}: {}", name, whisper_setup::human_size(downloaded))
                } else {
                    format!("{}: {}", name, whisper_setup::human_size(downloaded))
                };
                self.whisper_status = label.clone();

                // Логируем только при смене процента (каждые 5%) или на ключевых этапах
                let should_log = stage == "resume"
                    || stage == "done"
                    || (pct >= 0 && (pct - self.apply_progress.last_log_pct).abs() >= 5);
                if should_log {
                    if stage == "resume" {
                        self.log(&format!(
                            "Resume: {} at {} ({})",
                            name,
                            whisper_setup::human_size(downloaded),
                            if total > 0 {
                                format!("{}/{}", pct, 100)
                            } else {
                                "?%".to_string()
                            }
                        ));
                    } else if stage == "done" {
                        self.log(&format!("Done: {} ({})", name, whisper_setup::human_size(downloaded)));
                    } else if pct >= 0 {
                        self.log(&format!(
                            "{}: {}% ({} / {})",
                            name,
                            pct,
                            whisper_setup::human_size(downloaded),
                            whisper_setup::human_size(total)
                        ));
                    }
                    self.apply_progress.last_log_pct = pct;
                }

                // Финальный этап скачивания — закрываем apply_progress
                if stage == "done" && name.contains(".bin") {
                    self.apply_progress.stage = "load".into();
                }
            }
            WhisperEvent::Error(msg) => {
                self.whisper_status = self.t("whisper.error").to_string();
                self.big_status.clear();
                self.log(&format!("ERROR: {}", msg));
                self.apply_progress.active = false;
                self.apply_progress.stage = "error".into();
            }
        }
    }

    fn poll_channels(&mut self) {
        if self.stopping && !self.audio.is_active() {
            self.stopping = false;

            let source = match self.record_mode {
                Some(AudioMode::Loopback) => "LOOPBACK",
                Some(AudioMode::Mic) => "MIC",
                None => "AUDIO",
            };

            while let Ok(chunk) = self.audio_rx.try_recv() {
                self.audio_buffer.extend_from_slice(&chunk);
            }

            let samples = self.audio_buffer.clone();
            self.audio_buffer.clear();

            if !samples.is_empty() {
                self.whisper.finalize(samples, source);
            }

            self.record_mode = None;
            self.transcript_hint = self.t("main.live_speech").to_string();
            self.big_status = self.t("main.stopped").to_string();
            self.set_status(self.t("main.stopped"));
            self.log("Recording stopped");

            self.awaiting_transcript = true;
        }

        while let Ok(chunk) = self.audio_rx.try_recv() {
            self.audio_buffer.extend_from_slice(&chunk);
        }

        while let Ok(event) = self.whisper_rx.try_recv() {
            self.apply_whisper_event(event);
        }

        if self.whisper.is_loaded() && self.whisper_status != self.t("whisper.ready") {
            // Если только что загрузилось (после apply) — отмечаем done
            if self.apply_progress.active {
                self.apply_progress.active = false;
                self.apply_progress.stage = "done".into();
                self.apply_progress.last_log_pct = 100;
            }
            self.whisper_status = self.t("whisper.ready").to_string();
        }

        while let Ok(event) = self.ai_rx.try_recv() {
            match event {
                AiEvent::Answer(text) => {
                    if !self.question.is_empty() {
                        if !self.history_questions.is_empty() {
                            self.history_questions.push(crate::core::helpers::ENTRY_SEP);
                        }
                        self.history_questions.push_str(&self.question);
                    }
                    if !text.is_empty() {
                        if !self.history_answers.is_empty() {
                            self.history_answers.push(crate::core::helpers::ENTRY_SEP);
                        }
                        self.history_answers.push_str(&text);
                    }
                    trim_entries(&mut self.history_questions, MAX_HISTORY_ENTRIES);
                    trim_entries(&mut self.history_answers, MAX_HISTORY_ENTRIES);
                    self.answer = text;
                    self.big_status.clear();
                    self.set_status(self.t("main.ai_response"));
                    self.log("AI request completed");
                }
                AiEvent::Error(err) => {
                    if !self.question.is_empty() {
                        if !self.history_questions.is_empty() {
                            self.history_questions.push(crate::core::helpers::ENTRY_SEP);
                        }
                        self.history_questions.push_str(&self.question);
                    }
                    if !self.history_answers.is_empty() {
                        self.history_answers.push(crate::core::helpers::ENTRY_SEP);
                    }
                    self.history_answers.push_str(&format!("[ОШИБКА] {err}"));
                    trim_entries(&mut self.history_questions, MAX_HISTORY_ENTRIES);
                    trim_entries(&mut self.history_answers, MAX_HISTORY_ENTRIES);
                    self.last_error = format!("{}: {err}", self.t("main.ai_error"));
                    self.big_status.clear();
                    self.set_status(self.t("main.ai_error"));
                    self.log(&format!("AI error: {err}"));
                }
            }
            self.ai_busy = false;
            self.ai_request_time = None;
        }

        if self.ai_busy {
            if let Some(start) = self.ai_request_time {
                if start.elapsed() >= Duration::from_secs(5) {
                    self.big_status = self.t("main.no_answer").to_string();
                    self.set_status(self.t("main.ai_timeout"));
                }
            }
        }

        while let Ok(action) = self.hotkey_rx.try_recv() {
            match action {
                HotkeyAction::LoopbackPress if self.active_hold.is_none() => {
                    self.active_hold = Some(HotkeySide::Left);
                    self.start_recording(AudioMode::Loopback);
                }
                HotkeyAction::LoopbackRelease if self.active_hold == Some(HotkeySide::Left) => {
                    self.active_hold = None;
                    if self.record_mode == Some(AudioMode::Loopback) {
                        self.stop_recording();
                    }
                }
                HotkeyAction::MicPress if self.active_hold.is_none() => {
                    self.active_hold = Some(HotkeySide::Right);
                    self.start_recording(AudioMode::Mic);
                }
                HotkeyAction::MicRelease if self.active_hold == Some(HotkeySide::Right) => {
                    self.active_hold = None;
                    if self.record_mode == Some(AudioMode::Mic) {
                        self.stop_recording();
                    }
                }
                _ => {}
            }
        }
    }
}

impl eframe::App for InterviewApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(egui::Visuals::dark());
        self.poll_channels();
        ctx.request_repaint_after(Duration::from_millis(50));

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(Theme::BG))
            .show(ctx, |ui| {
                draw_header(ui, "Interview Assistant", "[<-] loopback  [->] mic", self.recording);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let tab_names = [
                        self.t("tab.main"),
                        self.t("tab.history"),
                        self.t("tab.settings"),
                        self.t("tab.logs"),
                    ];
                    for (idx, name) in tab_names.iter().enumerate() {
                        let selected = self.tab == idx;
                        let (fill, text_color, stroke_color) = if selected {
                            (Theme::ACCENT_SOFT, Color32::WHITE, Theme::ACCENT)
                        } else {
                            (Theme::TAB_INACTIVE, Theme::TEXT, Theme::BORDER)
                        };
                        let button = egui::Button::new(
                            egui::RichText::new(*name).color(text_color).strong(),
                        )
                        .fill(fill)
                        .stroke(egui::Stroke::new(1.5, stroke_color))
                        .rounding(egui::Rounding::same(10.0));
                        if ui.add(button).clicked() {
                            self.tab = idx;
                        }
                    }
                });
                ui.add_space(8.0);
                match self.tab {
                    0 => crate::ui::main_tab::show(ui, self),
                    1 => crate::ui::history_tab::show(ui, self),
                    2 => crate::ui::settings_tab::show(ui, self),
                    _ => crate::ui::logs_tab::show(ui, self),
                }
                ui.add_space(8.0);

                // В строке состояния: если идёт скачивание, показываем процент.
                let whisper_info = if self.apply_progress.active
                    && (self.apply_progress.stage == "dll"
                        || self.apply_progress.stage == "model")
                {
                    let pct = self.apply_progress.pct();
                    format!(
                        "{}: {}% | {} Hz",
                        self.apply_progress.name,
                        pct,
                        SAMPLE_RATE
                    )
                } else {
                    format!("Whisper: {} | {} Hz", self.whisper_status, SAMPLE_RATE)
                };

                crate::ui::status_bar(
                    ui,
                    &self.status,
                    &format!("AI: {}", self.cfg.model),
                    &whisper_info,
                );
            });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.whisper.unload();
        self.audio.stop();
    }
}

pub fn launch() -> eframe::Result<()> {
    let instance = match crate::core::acquire_single_instance() {
        Some(guard) => guard,
        None => {
            eprintln!("Interview Assistant is already running.");
            std::process::exit(0);
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1450.0, 880.0])
            .with_min_inner_size([1200.0, 720.0])
            .with_title("Interview Assistant"),
        ..Default::default()
    };

    eframe::run_native(
        "Interview Assistant",
        options,
        Box::new(|cc| {
            apply_theme(&cc.egui_ctx);
            Ok(Box::new(InterviewApp::new(instance)))
        }),
    )
}