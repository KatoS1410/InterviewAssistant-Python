// Всякая всячина: устройства, хелперы, сингл-инстанс, Whisper FFI.

pub mod devices;
pub mod helpers;
pub mod single_instance;
pub mod whisper_ffi;
pub mod whisper_setup;

pub use devices::{find_loopback_device, find_mic_device, list_input_devices};
pub use helpers::{timestamp, to_int, trim_entries, trim_lines};
pub use single_instance::{acquire_single_instance, SingleInstanceGuard};
