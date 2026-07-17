//! Захват звука через cpal (WASAPI на Windows).
//!
//! Два режима:
//! - Loopback: захват системного вывода (то, что слышно в колонках).
//! - Mic: захват с микрофона.
//!
//! Как устроено:
//! - Запись крутится в отдельном потоке ("audio-capture").
//! - Звук сводится в моно, передискретизируется в 16 кГц и шлётся кусками
//!   через crossbeam-канал в главный поток.
//! - Главный поток копит куски в буфер; при остановке весь буфер
//!   отдаётся VOSK'у за один проход.
//!
//! Неблокирующая остановка:
//! - `stop()` ставит флаг остановки и сразу возвращается.
//! - Поток захвата дропает стрим, сливает остатки и выходит.
//! - Главный поток опрашивает `is_active()`, чтобы понять, что поток
//!   закончил.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use anyhow::{anyhow, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use crossbeam_channel::Sender;
use parking_lot::Mutex;

use crate::core::devices::{is_loopback_name, resolve_device, AudioDeviceInfo};

// ---------------------------------------------------------------------------
// Константы
// ---------------------------------------------------------------------------

/// Целевая частота дискретизации для VOSK (16 кГц моно).
pub const SAMPLE_RATE: u32 = 16000;


// ---------------------------------------------------------------------------
// Режим захвата звука
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioMode {
    Loopback,
    Mic,
}

// ---------------------------------------------------------------------------
// AudioRecorder — управляет захватом звука в фоновом потоке
// ---------------------------------------------------------------------------

/// Управляет захватом звука в фоновом потоке.
///
/// # Неблокирующая остановка
///
/// `stop()` ставит флаг остановки и сразу возвращается. Поток захвата
/// дропает стрим, сливает остатки буфера и выходит.
/// Дёргай `is_active()`, чтобы проверить, закончил ли поток.
pub struct AudioRecorder {
    stop_flag: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// Внутренний буфер накопления, общий с колбэком захвата.
    audio_buffer: Arc<Mutex<Vec<i16>>>,
}

impl AudioRecorder {
    pub fn new() -> Self {
        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            active: Arc::new(AtomicBool::new(false)),
            thread: None,
            audio_buffer: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// true, пока поток захвата крутится.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    /// Запускает захват звука с указанного устройства.
    ///
    /// `chunk_ms` — как часто слать аудио-куски в `tx`.
    /// Если запись уже идёт — сначала останавливает.
    pub fn start(
        &mut self,
        mode: AudioMode,
        device_name: &str,
        chunk_ms: u32,
        tx: Sender<Vec<i16>>,
    ) -> Result<()> {
        if self.is_active() {
            self.stop();
        }

        self.stop_flag.store(false, Ordering::SeqCst);
        let stop_flag = Arc::clone(&self.stop_flag);
        let active = Arc::clone(&self.active);
        let audio_buffer = Arc::clone(&self.audio_buffer);

        let device = pick_device(mode, device_name)?;

        active.store(true, Ordering::SeqCst);

        let handle = thread::Builder::new()
            .name("audio-capture".into())
            .spawn(move || {
                if let Err(err) =
                    capture_loop(device, mode, chunk_ms, stop_flag, tx, audio_buffer)
                {
                    eprintln!("audio capture error: {err}");
                }
                active.store(false, Ordering::SeqCst);
            })
            .context("spawn audio thread")?;

        self.thread = Some(handle);
        Ok(())
    }

    /// Даёт сигнал потоку захвата остановиться. Возвращается сразу.
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        // Отсоединяем ручку потока — он закончит сам.
        // Не делаем join здесь, чтобы не блокировать UI-поток.
        if let Some(handle) = self.thread.take() {
            // Запускаем крошечный поток-уборщик, который просто
            // джойнит старую ручку, чтобы не утекали ресурсы ОС.
            thread::Builder::new()
                .name("audio-reaper".into())
                .spawn(move || {
                    let _ = handle.join();
                })
                .ok();
        }
    }
}

impl Default for AudioRecorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AudioRecorder {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Выбор устройства
// ---------------------------------------------------------------------------

/// Выбирает аудиоустройство: по имени если задано, иначе автоопределение по режиму.
fn pick_device(mode: AudioMode, device_name: &str) -> Result<AudioDeviceInfo> {
    if !device_name.trim().is_empty() {
        if let Some(dev) = resolve_device(device_name) {
            return Ok(dev);
        }
    }

    match mode {
        AudioMode::Loopback => crate::core::find_loopback_device()
            .ok_or_else(|| anyhow!("Loopback device not found")),
        AudioMode::Mic => {
            let host = cpal::default_host();
            host.default_input_device()
                .and_then(|d| d.name().ok())
                .and_then(|name| resolve_device(&name))
                .ok_or_else(|| anyhow!("Microphone not found"))
        }
    }
}

// ---------------------------------------------------------------------------
// Цикл захвата (крутится в потоке audio-capture)
// ---------------------------------------------------------------------------

/// Главный цикл захвата. Открывает устройство, запускает поток, ждёт флага
/// остановки, дропает стрим и сливает остатки буфера.
fn capture_loop(
    device_info: AudioDeviceInfo,
    mode: AudioMode,
    chunk_ms: u32,
    stop_flag: Arc<AtomicBool>,
    tx: Sender<Vec<i16>>,
    buffer: Arc<Mutex<Vec<i16>>>,
) -> Result<()> {
    let host = cpal::default_host();
    let device = open_cpal_device(&host, &device_info, mode)?;
    let config = pick_stream_config(&device, mode)?;

    let sample_rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let block_size = ((sample_rate as u64 * chunk_ms as u64) / 1000).max(256) as usize;

    let err_fn = |err| eprintln!("audio stream error: {err}");

    let stream = match config.sample_format() {
        SampleFormat::I16 => build_i16_stream(
            &device, &config, channels, sample_rate, block_size,
            tx.clone(), buffer.clone(), err_fn,
        )?,
        SampleFormat::F32 => build_f32_stream(
            &device, &config, channels, sample_rate, block_size,
            tx.clone(), buffer.clone(), err_fn,
        )?,
        SampleFormat::U16 => build_u16_stream(
            &device, &config, channels, sample_rate, block_size,
            tx.clone(), buffer.clone(), err_fn,
        )?,
        other => return Err(anyhow!("Unsupported sample format: {other:?}")),
    };

    stream.play()?;

    // Ждём, пока главный поток не даст сигнал остановки.
    while !stop_flag.load(Ordering::SeqCst) {
        thread::sleep(std::time::Duration::from_millis(20));
    }

    // Даём WASAPI время доставить последние буферы (200 мс).
    // Без этой паузы последние 100-200 мс аудио теряются внутри драйвера.
    thread::sleep(std::time::Duration::from_millis(200));

    // Дропаем стрим — последние данные уже в наших буферах.
    drop(stream);

    // Сливаем остатки из буфера накопления (несколько блоков по 10 мс).
    {
        let mut buf = buffer.lock();
        if !buf.is_empty() {
            let _ = tx.send(buf.clone());
            buf.clear();
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Вспомогательные функции для cpal (устройства / потоки)
// ---------------------------------------------------------------------------

fn open_cpal_device(
    host: &cpal::Host,
    info: &AudioDeviceInfo,
    mode: AudioMode,
) -> Result<cpal::Device> {
    // Loopback-устройства на Windows открываются как устройства вывода.
    if info.is_output || matches!(mode, AudioMode::Loopback) || is_loopback_name(&info.name) {
        #[cfg(windows)]
        {
            if let Ok(outputs) = host.output_devices() {
                for device in outputs {
                    if let Ok(name) = device.name() {
                        if info.name.starts_with(&name) || name == info.name {
                            return Ok(device);
                        }
                    }
                }
            }
        }
    }

    // Запасной вариант: ищем среди устройств ввода.
    if let Ok(inputs) = host.input_devices() {
        for device in inputs {
            if device.name().ok().as_deref() == Some(info.name.as_str()) {
                return Ok(device);
            }
        }
    }

    host.default_input_device()
        .ok_or_else(|| anyhow!("Could not open device: {}", info.name))
}

fn pick_stream_config(
    device: &cpal::Device,
    mode: AudioMode,
) -> Result<cpal::SupportedStreamConfig> {
    if matches!(mode, AudioMode::Loopback) || device.default_output_config().is_ok() {
        if let Ok(out) = device.default_output_config() {
            return Ok(out);
        }
    }
    device
        .default_input_config()
        .map_err(|e| anyhow!("Stream config error: {e}"))
}

// ---------------------------------------------------------------------------
// Обработка звука: downmix → resample → chunk
// ---------------------------------------------------------------------------

/// Сводит многоканальный i16 в моно усреднением каналов.
fn downmix_to_mono_i16(samples: &[i16], channels: usize) -> Vec<i16> {
    if channels <= 1 {
        return samples.to_vec();
    }
    samples
        .chunks(channels)
        .map(|frame| {
            let sum: i32 = frame.iter().map(|&s| s as i32).sum();
            (sum / channels as i32) as i16
        })
        .collect()
}

/// Линейная передискретизация из `from_rate` в `to_rate`.
fn resample_linear(input: &[i16], from_rate: u32, to_rate: u32) -> Vec<i16> {
    if from_rate == to_rate || input.is_empty() {
        return input.to_vec();
    }
    let out_len = ((input.len() as u64 * to_rate as u64) / from_rate as u64) as usize;
    let mut out = Vec::with_capacity(out_len.max(1));
    for i in 0..out_len.max(1) {
        let src_pos = (i as f64 * from_rate as f64) / to_rate as f64;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;
        let a = input.get(idx).copied().unwrap_or(0) as f32;
        let b = input.get(idx + 1).copied().unwrap_or(a as i16) as f32;
        out.push((a + (b - a) * frac) as i16);
    }
    out
}

/// Обрабатывает сырой звук из колбэка потока:
/// downmix → resample → накопление → выдача кусков при достижении block_size.
fn emit_chunk(
    raw: Vec<i16>,
    channels: usize,
    sample_rate: u32,
    block_size: usize,
    tx: &Sender<Vec<i16>>,
    buffer: &Mutex<Vec<i16>>,
) {
    let mono = downmix_to_mono_i16(&raw, channels);
    let at_target = resample_linear(&mono, sample_rate, SAMPLE_RATE);

    let mut buf = buffer.lock();
    buf.extend(at_target);
    while buf.len() >= block_size {
        let chunk: Vec<i16> = buf.drain(0..block_size).collect();
        let _ = tx.try_send(chunk);
    }
}

// ---------------------------------------------------------------------------
// Сборщики потоков (по одному на формат сэмплов)
// ---------------------------------------------------------------------------

fn build_i16_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    channels: usize,
    sample_rate: u32,
    block_size: usize,
    tx: Sender<Vec<i16>>,
    buffer: Arc<Mutex<Vec<i16>>>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream> {
    let cfg = config.clone().into();
    device
        .build_input_stream(
            &cfg,
            move |data: &[i16], _| {
                emit_chunk(data.to_vec(), channels, sample_rate, block_size, &tx, &buffer);
            },
            err_fn,
            None,
        )
        .map_err(|e| anyhow!("build i16 stream: {e}"))
}

fn build_f32_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    channels: usize,
    sample_rate: u32,
    block_size: usize,
    tx: Sender<Vec<i16>>,
    buffer: Arc<Mutex<Vec<i16>>>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream> {
    let cfg = config.clone().into();
    device
        .build_input_stream(
            &cfg,
            move |data: &[f32], _| {
                let pcm: Vec<i16> = data
                    .iter()
                    .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                    .collect();
                emit_chunk(pcm, channels, sample_rate, block_size, &tx, &buffer);
            },
            err_fn,
            None,
        )
        .map_err(|e| anyhow!("build f32 stream: {e}"))
}

fn build_u16_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    channels: usize,
    sample_rate: u32,
    block_size: usize,
    tx: Sender<Vec<i16>>,
    buffer: Arc<Mutex<Vec<i16>>>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream> {
    let cfg = config.clone().into();
    device
        .build_input_stream(
            &cfg,
            move |data: &[u16], _| {
                let pcm: Vec<i16> = data
                    .iter()
                    .map(|&s| (s as i32 - 32768) as i16)
                    .collect();
                emit_chunk(pcm, channels, sample_rate, block_size, &tx, &buffer);
            },
            err_fn,
            None,
        )
        .map_err(|e| anyhow!("build u16 stream: {e}"))
}
