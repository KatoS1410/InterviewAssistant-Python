use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const SYSTEM_PROMPT: &str = r#"Ты — {position}, проходишь техническое собеседование. Отвечаешь устно, как живой кандидат на экзамене: уверенно, по делу, без воды.

ВАЖНО про распознавание речи. Вопросы приходят из системы распознавания речи, которая часто искажает технические термины. Восстанавливай правильный смысл по контексту профессии. Примеры искажений: «мокер» или «докир» = Docker; «кубер»/«кубернетис» = Kubernetes; «джанго» = Django; «редис» = Redis; «постгрес»/«постгря» = PostgreSQL; «гит»/«гет» = Git; «реакт»/«риакт» = React; «эс кью эль» = SQL; «апи» = API; «джейсон» = JSON. Если термин явно искажён — мысленно исправь его и отвечай по сути, не переспрашивай и не комментируй искажение.

КОНТЕКСТ ДИАЛОГА. Помни предыдущие вопросы и ответы. Если новый вопрос продолжает тему, отвечай именно про предмет прошлого вопроса, а не абстрактно.

ФОРМАТ ОТВЕТА. Краткость — сестра таланта. Отвечай сжато: суть в 2–5 предложениях, при необходимости короткий список. Без вступлений вроде «отличный вопрос». Не упоминай, что ты ИИ или языковая модель. Говори от первого лица, как специалист. Если вопрос требует кода — дай минимальный пример."#;

pub const PROVIDERS: &[(&str, &str, &str)] = &[
    ("OpenAI", "https://api.openai.com/v1", "gpt-4o-mini"),
    ("DeepSeek", "https://api.deepseek.com/v1", "deepseek-chat"),
    (
        "OpenRouter",
        "https://openrouter.ai/api/v1",
        "deepseek/deepseek-chat",
    ),
    ("Local LLMs", "http://127.0.0.1:8000", "llama3"),
    ("GigaChat", "", "GigaChat"),
    ("Свой", "", ""),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppConfig {
    pub provider: String,
    pub api_key: String,
    pub model: String,
    pub base_url: String,
    pub whisper_model: String,
    pub chunk_ms: u32,
    pub auto_ask_sec: u32,
    pub loopback_device: String,
    pub mic_device: String,
    pub position: String,
    pub system_prompt: String,
    pub gigachat_scope: String,
    pub gigachat_auth_key: String,
    pub local_llm_address: String,
    pub local_llm_port: u16,
    pub lang: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            provider: "OpenAI".into(),
            api_key: String::new(),
            model: "gpt-4o-mini".into(),
            base_url: "https://api.openai.com/v1".into(),
            whisper_model: "small".into(),
            chunk_ms: 500,
            auto_ask_sec: 0,
            loopback_device: String::new(),
            mic_device: String::new(),
            position: String::new(),
            system_prompt: SYSTEM_PROMPT.into(),
            gigachat_scope: "GIGACHAT_API_PERS".into(),
            gigachat_auth_key: String::new(),
            local_llm_address: "127.0.0.1".into(),
            local_llm_port: 8000,
            lang: "ru".into(),
        }
    }
}

impl AppConfig {
    pub fn resolved_system_prompt(&self) -> String {
        self.system_prompt.replace(
            "{position}",
            if self.position.trim().is_empty() {
                "специалист"
            } else {
                self.position.trim()
            },
        )
    }

    pub fn apply_provider_preset(&mut self, provider: &str) {
        if let Some((_, url, model)) = PROVIDERS.iter().find(|(p, _, _)| *p == provider) {
            if !url.is_empty() {
                self.base_url = (*url).into();
            }
            if !model.is_empty() && (self.model.is_empty() || self.model == "gpt-4o-mini") {
                self.model = (*model).into();
            }
        }
        self.provider = provider.into();
    }
}

pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".katos_interview_assistant")
}

pub fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

pub fn load() -> AppConfig {
    let path = config_path();
    if !path.exists() {
        let cfg = AppConfig::default();
        let _ = save(&cfg);
        return cfg;
    }

    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => AppConfig::default(),
    }
}

pub fn save(cfg: &AppConfig) -> anyhow::Result<()> {
    fs::create_dir_all(config_dir())?;
    let tmp = config_path().with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_string_pretty(cfg)?)?;
    fs::rename(tmp, config_path())?;
    Ok(())
}

pub fn export_to(path: &Path, cfg: &AppConfig) -> anyhow::Result<()> {
    fs::write(path, serde_json::to_string_pretty(cfg)?)?;
    Ok(())
}

pub fn import_from(path: &Path) -> anyhow::Result<AppConfig> {
    let raw = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

