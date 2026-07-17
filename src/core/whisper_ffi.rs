//! Привязки FFI к whisper.dll (нативная библиотека whisper.cpp).
//! Используется libloading для динамической загрузки — линковка на этапе компиляции не нужна.

use std::ffi::{c_char, c_float, c_int, CStr, CString};
use std::path::Path;

use libloading::{Library, Symbol};

// Windows API: SetDllDirectoryW — добавляет папку в пути поиска DLL.
#[cfg(windows)]
extern "system" {
    fn SetDllDirectoryW(lpPathName: *const u16) -> i32;
}

#[cfg(windows)]
fn add_dll_directory(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SetDllDirectoryW(wide.as_ptr());
    }
}

#[cfg(not(windows))]
fn add_dll_directory(_path: &Path) {}

/// Заглушка для старых ggml без backend_load.
unsafe extern "C" fn noop_backend_load(_p: *const c_char) {}

/// Непрозрачный контекст whisper.
#[repr(C)]
pub struct WhisperContext {
    _private: [u8; 0],
}

/// Параметры инициализации контекста (whisper_context_params из whisper.h).
///
/// В актуальных сборках whisper.cpp (с ggml-backend) раскладка 32 байта:
///   bool use_gpu;                        // смещение 0
///   bool flash_attn;                     // смещение 1
///   int  gpu_device;                     // смещение 4
///   bool dtw;                            // смещение 8
///   struct ggml_backend_device * device; // смещение 16 (указатель!)
///   struct ggml_backend * backend;       // смещение 24 (указатель!)
#[repr(C)]
pub struct WhisperContextParams {
    pub use_gpu: u8,
    pub flash_attn: u8,
    pub _pad1: [u8; 2],
    pub gpu_device: c_int,
    pub dtw: u8,
    pub _pad2: [u8; 7],
    pub device: *mut u8,
    pub backend: *mut u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum WhisperSamplingStrategy {
    Greedy = 0,
}

#[repr(C)]
pub struct WhisperFullParams {
    pub strategy: c_int,
    pub n_threads: c_int,
    pub n_max_text_ctx: c_int,
    pub offset_ms: c_int,
    pub duration_ms: c_int,
    pub translate: c_int,
    pub no_context: c_int,
    pub no_timestamps: c_int,
    pub single_segment: c_int,
    pub print_special: c_int,
    pub print_progress: c_int,
    pub print_realtime: c_int,
    pub print_timestamps: c_int,
    pub token_timestamps: c_int,
    pub thold_pt: c_float,
    pub thold_ptsum: c_float,
    pub max_len: c_int,
    pub split_on_word: c_int,
    pub max_tokens: c_int,
    pub speed_up: c_int,
    pub debug_mode: c_int,
    pub audio_ctx: c_int,
    pub tdrz_enable: c_int,
    pub initial_prompt: *const c_char,
    pub prompt_tokens: *const c_int,
    pub prompt_n_tokens: c_int,
    pub language: *const c_char,
    pub detect_language: c_int,
    pub suppress_blank: c_int,
    pub suppress_non_speech_tokens: c_int,
    pub temperature: c_float,
    pub max_initial_ts: c_float,
    pub length_penalty: c_float,
    pub temperature_inc: c_float,
    pub entropy_thold: c_float,
    pub logprob_thold: c_float,
    pub no_speech_thold: c_float,
    pub greedy: c_int,
    pub beam_search: c_int,
    pub n_best: c_int,
}

/// Загруженная whisper.dll с указателями на функции.
pub struct WhisperDll {
    _ggml_lib: Library,
    _lib: Library,
    pub context_default_params_by_ref:
        unsafe extern "C" fn(*mut WhisperContextParams),
    pub init_from_file_with_params_no_state:
        unsafe extern "C" fn(*const c_char, *const WhisperContextParams) -> *mut WhisperContext,
    pub free: unsafe extern "C" fn(*mut WhisperContext),
    pub full_default_params: unsafe extern "C" fn(c_int, *mut WhisperFullParams),
    pub full: unsafe extern "C" fn(*mut WhisperContext, WhisperFullParams, *const c_float, c_int) -> c_int,
    pub full_n_segments: unsafe extern "C" fn(*mut WhisperContext) -> c_int,
    pub full_get_segment_text: unsafe extern "C" fn(*mut WhisperContext, c_int) -> *const c_char,
    backend_load_all_from_path: unsafe extern "C" fn(*const c_char),
}

impl WhisperDll {
    pub fn load(dll_dir: &Path) -> Result<Self, String> {
        let dll_path = dll_dir.join("whisper.dll");
        if !dll_path.exists() {
            return Err(format!("whisper.dll not found at {}", dll_path.display()));
        }

        let dll_dir_abs = dll_dir
            .canonicalize()
            .unwrap_or_else(|_| dll_dir.to_path_buf());
        add_dll_directory(&dll_dir_abs);

        // Грузим ggml.dll — именно через неё whisper.dll регистрирует бэкенды.
        // В ggml.dll лежит ggml_backend_load_all_from_path.
        let ggml_path = dll_dir_abs.join("ggml.dll");
        let ggml_lib = unsafe {
            if ggml_path.exists() {
                Library::new(&ggml_path)
                    .map_err(|e| format!("Failed to load ggml.dll: {e}"))?
            } else {
                // Фолбэк для старых сборок: ggml.dll может отсутствовать.
                Library::new(&dll_dir_abs.join("whisper.dll"))
                    .map_err(|e| format!("Failed to load whisper.dll as ggml fallback: {e}"))?
            }
        };

        let dll_path_abs = dll_path
            .canonicalize()
            .unwrap_or_else(|_| dll_path.clone());
        let lib = unsafe {
            Library::new(&dll_path_abs)
                .map_err(|e| format!("Failed to load whisper.dll: {e}"))?
        };

        // ggml_backend_load_all_from_path — опциональный символ.
        let backend_load_all_from_path: unsafe extern "C" fn(*const c_char) = unsafe {
            let sym: Option<Symbol<unsafe extern "C" fn(*const c_char)>> = ggml_lib
                .get(b"ggml_backend_load_all_from_path\0")
                .ok();
            match sym {
                Some(s) => *s,
                None => noop_backend_load,
            }
        };

        let context_default_params_by_ref: unsafe extern "C" fn(*mut WhisperContextParams) = {
            let sym: Symbol<unsafe extern "C" fn(*mut WhisperContextParams)> = unsafe {
                lib.get(b"whisper_context_default_params_by_ref\0")
                    .map_err(|e| format!("Symbol whisper_context_default_params_by_ref: {e}"))?
            };
            *sym
        };

        let init_from_file_with_params_no_state: unsafe extern "C" fn(
            *const c_char,
            *const WhisperContextParams,
        ) -> *mut WhisperContext = {
            let sym: Symbol<
                unsafe extern "C" fn(*const c_char, *const WhisperContextParams) -> *mut WhisperContext,
            > = unsafe {
                lib.get(b"whisper_init_from_file_with_params_no_state\0")
                    .map_err(|e| format!("Symbol whisper_init_from_file_with_params_no_state: {e}"))?
            };
            *sym
        };

        let free: unsafe extern "C" fn(*mut WhisperContext) = {
            let sym: Symbol<unsafe extern "C" fn(*mut WhisperContext)> = unsafe {
                lib.get(b"whisper_free\0")
                    .map_err(|e| format!("Symbol whisper_free: {e}"))?
            };
            *sym
        };

        let full_default_params: unsafe extern "C" fn(c_int, *mut WhisperFullParams) = {
            let sym: Symbol<unsafe extern "C" fn(c_int, *mut WhisperFullParams)> = unsafe {
                lib.get(b"whisper_full_default_params_by_ref\0")
                    .map_err(|e| format!("Symbol whisper_full_default_params_by_ref: {e}"))?
            };
            *sym
        };

        let full: unsafe extern "C" fn(
            *mut WhisperContext,
            WhisperFullParams,
            *const c_float,
            c_int,
        ) -> c_int = {
            let sym: Symbol<
                unsafe extern "C" fn(
                    *mut WhisperContext,
                    WhisperFullParams,
                    *const c_float,
                    c_int,
                ) -> c_int,
            > = unsafe {
                lib.get(b"whisper_full\0")
                    .map_err(|e| format!("Symbol whisper_full: {e}"))?
            };
            *sym
        };

        let full_n_segments: unsafe extern "C" fn(*mut WhisperContext) -> c_int = {
            let sym: Symbol<unsafe extern "C" fn(*mut WhisperContext) -> c_int> = unsafe {
                lib.get(b"whisper_full_n_segments\0")
                    .map_err(|e| format!("Symbol whisper_full_n_segments: {e}"))?
            };
            *sym
        };

        let full_get_segment_text: unsafe extern "C" fn(
            *mut WhisperContext,
            c_int,
        ) -> *const c_char = {
            let sym: Symbol<
                unsafe extern "C" fn(*mut WhisperContext, c_int) -> *const c_char,
            > = unsafe {
                lib.get(b"whisper_full_get_segment_text\0")
                    .map_err(|e| format!("Symbol whisper_full_get_segment_text: {e}"))?
            };
            *sym
        };

        Ok(Self {
            _ggml_lib: ggml_lib,
            _lib: lib,
            context_default_params_by_ref,
            init_from_file_with_params_no_state,
            free,
            full_default_params,
            full,
            full_n_segments,
            full_get_segment_text,
            backend_load_all_from_path,
        })
    }

    /// Создаёт контекст whisper из файла модели с параметрами только-CPU.
    pub unsafe fn init_context(&self, model_path: &str) -> Result<*mut WhisperContext, String> {
        let c_path = CString::new(model_path)
            .map_err(|e| format!("Invalid model path: {e}"))?;

        // === ВАЖНО: подгружаем CPU/GPU-бэкенды до создания контекста. ===
        let model_parent = std::path::Path::new(model_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_default();
        if let Some(dir_str) = model_parent.to_str() {
            if let Ok(c_dir) = CString::new(dir_str) {
                (self.backend_load_all_from_path)(c_dir.as_ptr());
            }
        }

        let mut ctx_params: WhisperContextParams = std::mem::zeroed();
        (self.context_default_params_by_ref)(&mut ctx_params);
        ctx_params.use_gpu = 0;

        let ctx = (self.init_from_file_with_params_no_state)(c_path.as_ptr(), &ctx_params);
        if ctx.is_null() {
            return Err(format!(
                "whisper_init_from_file_with_params_no_state failed for {}",
                model_path
            ));
        }
        Ok(ctx)
    }

    pub unsafe fn free_context(&self, ctx: *mut WhisperContext) {
        if !ctx.is_null() {
            (self.free)(ctx);
        }
    }

    pub unsafe fn default_params(&self, strategy: WhisperSamplingStrategy) -> WhisperFullParams {
        let mut params: WhisperFullParams = std::mem::zeroed();
        (self.full_default_params)(strategy as c_int, &mut params);
        params
    }

    pub unsafe fn run_full(
        &self,
        ctx: *mut WhisperContext,
        params: WhisperFullParams,
        samples: &[f32],
    ) -> Result<i32, String> {
        let ret = (self.full)(ctx, params, samples.as_ptr(), samples.len() as c_int);
        if ret != 0 {
            return Err(format!("whisper_full returned error code {ret}"));
        }
        Ok(ret)
    }

    pub unsafe fn n_segments(&self, ctx: *mut WhisperContext) -> i32 {
        (self.full_n_segments)(ctx) as i32
    }

    pub unsafe fn segment_text(&self, ctx: *mut WhisperContext, index: i32) -> String {
        let ptr = (self.full_get_segment_text)(ctx, index as c_int);
        if ptr.is_null() {
            return String::new();
        }
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

/// Преобразует i16 образцы звука в f32 (нужно для whisper).
pub fn i16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples
        .iter()
        .map(|&s| s as f32 / 32768.0)
        .collect()
}