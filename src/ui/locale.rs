//! Строки локализации UI

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Lang {
    #[serde(rename = "ru")]
    Ru,
    #[serde(rename = "en")]
    En,
}

impl Lang {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "en" | "english" => Lang::En,
            _ => Lang::Ru,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Lang::Ru => "ru",
            Lang::En => "en",
        }
    }

}

/// Получить локализованную строку
pub fn t(lang: Lang, key: &str) -> &'static str {
    let entry = LOCALE.get(key).unwrap_or(&("???", "???"));
    match lang {
        Lang::Ru => entry.0,
        Lang::En => entry.1,
    }
}

/// Все строки: ключ → (русский, английский)
static LOCALE: phf::Map<&'static str, (&'static str, &'static str)> = phf::phf_map! {
    // Вкладки
    "tab.main" => ("Основное", "Main"),
    "tab.history" => ("История", "History"),
    "tab.settings" => ("Настройки", "Settings"),
    "tab.logs" => ("Логи", "Logs"),

    // Заголовок
    "header.title" => ("Interview Assistant", "Interview Assistant"),
    "header.hint" => ("[<-] loopback  [->] mic", "[<-] loopback  [->] mic"),

    // Основная вкладка
    "main.transcript" => ("Транскрипт", "Transcript"),
    "main.answer" => ("Ответ AI", "AI Answer"),
    "main.ask" => ("Спросить AI", "Ask AI"),
    "main.clear" => ("Очистить", "Clear"),
    "main.copy_answer" => ("Копировать ответ", "Copy Answer"),
    "main.copy_question" => ("Копировать вопрос", "Copy Question"),
    "main.prev_question" => ("Предыдущий вопрос", "Previous Question"),
    "main.prev_answer" => ("Предыдущий ответ", "Previous Answer"),
    "main.thinking" => ("Думаю...", "Thinking..."),
    "main.no_answer" => ("Нет ответа от ИИ, проверьте доступность модели", "No AI response, check model availability"),
    "main.whisper_not_loaded" => ("Whisper не подключен! Распознавание невозможно.", "Whisper not loaded! Recognition unavailable."),
    "main.recording" => ("Запись...", "Recording..."),
    "main.stopping" => ("Остановка записи...", "Stopping..."),
    "main.stopped" => ("Запись остановлена, распознавание...", "Recording stopped, recognizing..."),
    "main.source_loopback" => ("Источник: LOOPBACK", "Source: LOOPBACK"),
    "main.source_mic" => ("Источник: MIC", "Source: MIC"),
    "main.live_speech" => ("Live speech stream", "Live speech stream"),
    "main.ai_error" => ("Ошибка AI", "AI Error"),
    "main.ai_timeout" => ("Таймаут AI", "AI Timeout"),
    "main.ai_response" => ("Ответ получен", "Response received"),
    "main.ai_request" => ("Запрос к AI...", "AI request..."),
    "main.no_text" => ("Нет текста для AI", "No text for AI"),

    // История
    "history.title" => ("История вопросов и ответов", "Question & Answer History"),
    "history.questions" => ("Вопросы", "Questions"),
    "history.answers" => ("Ответы", "Answers"),
    "history.clear" => ("Очистить историю", "Clear History"),
    "history.empty" => ("История пуста", "History is empty"),

    // Настройки
    "settings.ai_provider" => ("AI / Provider", "AI / Provider"),
    "settings.provider" => ("Provider", "Provider"),
    "settings.auth_key" => ("Authorization Key", "Authorization Key"),
    "settings.scope" => ("Scope", "Scope"),
    "settings.address" => ("Address", "Address"),
    "settings.port" => ("Port", "Port"),
    "settings.api_key" => ("API Key", "API Key"),
    "settings.base_url" => ("Base URL", "Base URL"),
    "settings.model" => ("Model", "Model"),
    "settings.test" => ("Проверить", "Test"),
    "settings.position" => ("Должность", "Position"),
    "settings.system_prompt" => ("System Prompt", "System Prompt"),
    "settings.whisper" => ("Whisper / Аудио", "Whisper / Audio"),
    "settings.model_path" => ("Model path", "Model path"),
    "settings.chunk_ms" => ("Chunk ms", "Chunk ms"),
    "settings.auto_ask_sec" => ("Auto ask sec", "Auto ask sec"),
    "settings.load_whisper" => ("Загрузить Whisper", "Load Whisper"),
    "settings.reload_whisper" => ("Перезагрузить", "Reload"),
    "settings.setup_whisper" => ("Скачать модель Whisper", "Download Whisper model"),
    "settings.download_hint" => ("Скачать модель можно здесь:", "Download model here:"),
    "settings.unpack_hint" => ("Положите ggml-*.bin файл в папку модели.", "Place ggml-*.bin file in the model folder."),
    "settings.dll_hint" => ("whisper.dll качается автоматически.", "whisper.dll downloads automatically."),
    "settings.whisper_status" => ("Whisper:", "Whisper:"),
    "settings.show_config" => ("Показать конфиг для редактирования", "Show config for editing"),
    "settings.hide_config" => ("Скрыть конфиг", "Hide config"),
    "settings.config_title" => ("Конфиг", "Config"),
    "settings.save" => ("Сохранить", "Save"),
    "settings.export" => ("Экспорт", "Export"),
    "settings.import" => ("Импорт", "Import"),
    "settings.language" => ("Язык", "Language"),
    "settings.saved" => ("Настройки сохранены", "Settings saved"),

    // Логи
    "logs.title" => ("Логи приложения", "Application Logs"),
    "logs.clear" => ("Очистить логи", "Clear Logs"),

    // Строка состояния
    "status.devices" => ("Устройств:", "Devices:"),
    "status.devices_count" => ("Устройств: {}", "Devices: {}"),
    "status.recording_blocked" => ("Ошибка: Whisper не загружен", "Error: Whisper not loaded"),
    "status.ai" => ("AI:", "AI:"),
    "status.whisper" => ("Whisper:", "Whisper:"),
    "status.hz" => ("Hz", "Hz"),

    // Статус Whisper
    "whisper.not_loaded" => ("not loaded", "not loaded"),
    "whisper.loading" => ("loading...", "loading..."),
    "whisper.ready" => ("ready", "ready"),
    "whisper.error" => ("error", "error"),
    "whisper.no_path" => ("no model path", "no model path"),
    "whisper.downloading" => ("downloading...", "downloading..."),

    // Apply Changes (кнопка + статусы)
    "settings.apply_changes" => ("Применить изменения", "Apply Changes"),
    "settings.whisper_ready" => ("Whisper готов", "Whisper ready"),
    "settings.applying" => ("Применение...", "Applying..."),
    "settings.dl_dll" => ("Скачиваю whisper.dll...", "Downloading whisper.dll..."),
    "settings.dl_dll_exists" => ("whisper.dll уже скачан, пропускаю", "whisper.dll already present, skipping"),
    "settings.unpacking_dll" => ("Распаковываю whisper.dll...", "Unpacking whisper.dll..."),
    "settings.dll_done" => ("whisper.dll готов", "whisper.dll ready"),
    "settings.dl_model" => ("Скачиваю {name} (~{size})...", "Downloading {name} (~{size})..."),
    "settings.dl_model_exists" => ("Модель {name} уже скачана", "Model {name} already downloaded"),
    "settings.dl_resume" => ("Продолжаю загрузку {name} с {pct}%...", "Resuming {name} from {pct}%..."),
    "settings.dl_progress" => ("{name}: {pct}% ({done}/{total})", "{name}: {pct}% ({done}/{total})"),
    "settings.dl_done" => ("Скачано: {name} ({size})", "Downloaded: {name} ({size})"),
    "settings.load_start" => ("Загружаю {name} в Whisper...", "Loading {name} into Whisper..."),
    "settings.apply_done" => ("Готово", "Done"),
    "settings.dl_failed" => ("Ошибка скачивания {name}: {err}", "Download failed for {name}: {err}"),

    // Misc полезное
    "misc.mb_size" => ("{n} MB", "{n} MB"),
    "misc.gb_size" => ("{n} GB", "{n} GB"),

    // Прочее
    "misc.browse" => ("...", "..."),
    "misc.loopback_found" => ("Loopback найден", "Loopback found"),
    "misc.loopback_not_found" => ("Loopback не найден", "Loopback not found"),
    "misc.mic_found" => ("Микрофон найден", "Microphone found"),
    "misc.mic_not_found" => ("Микрофон не найден", "Microphone not found"),
    "misc.app_initialized" => ("App initialized", "App initialized"),
    "misc.config_saved" => ("Config saved", "Config saved"),
    "misc.recording_blocked" => ("Recording blocked: Whisper not loaded", "Recording blocked: Whisper not loaded"),
};