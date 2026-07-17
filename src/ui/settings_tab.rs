use std::time::Instant;

use crate::app::InterviewApp;
use crate::config::PROVIDERS;
use crate::core::whisper_setup::ModelKind;
use crate::ui::locale::Lang;
use crate::ui::theme::Theme;
use crate::ui::widgets::{
    copyable_text_edit, glass_panel, labeled_combo, pill_button, section_heading,
};
use egui::RichText;

/// Все модели, которые умеем качать и выбирать.
const WHISPER_MODELS: &[(ModelKind, &str)] = &[
    (ModelKind::Small, "small"),
    (ModelKind::Medium, "medium"),
    (ModelKind::SmallEn, "small.en"),
    (ModelKind::MediumEn, "medium.en"),
];

pub fn show(ui: &mut egui::Ui, app: &mut InterviewApp) {
    let available = ui.available_size();
    let col_w = available.x / 2.0 - 12.0;
    ui.columns(2, |cols| {
        // === Колонка 0 (AI, промпт) ===
        cols[0].allocate_ui(egui::vec2(col_w, available.y), |ui| {
            ui.set_clip_rect(ui.max_rect());
            glass_panel(ui, |ui| {
                section_heading(ui, app.t("settings.ai_provider"), "");
                let providers: Vec<String> = PROVIDERS.iter().map(|(p, _, _)| (*p).into()).collect();
                let prev_provider = app.cfg.provider.clone();
                labeled_combo(ui, app.t("settings.provider"), &mut app.cfg.provider, &providers);
                if app.cfg.provider != prev_provider {
                    app.cfg.apply_provider_preset(&app.cfg.provider.clone());
                }

                ui.add_space(4.0);

                match app.cfg.provider.to_lowercase().as_str() {
                    "gigachat" => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(app.t("settings.auth_key")).color(Theme::TEXT_DIM));
                            ui.add(
                                egui::TextEdit::singleline(&mut app.cfg.gigachat_auth_key)
                                    .password(true)
                                    .desired_width(ui.available_width()),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(app.t("settings.scope")).color(Theme::TEXT_DIM));
                            ui.text_edit_singleline(&mut app.cfg.gigachat_scope);
                        });
                    }
                    "local llms" => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(app.t("settings.address")).color(Theme::TEXT_DIM));
                            ui.text_edit_singleline(&mut app.cfg.local_llm_address);
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(app.t("settings.port")).color(Theme::TEXT_DIM));
                            ui.add(egui::DragValue::new(&mut app.cfg.local_llm_port).speed(1.0));
                        });
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(app.t("settings.api_key")).color(Theme::TEXT_DIM));
                            ui.add(
                                egui::TextEdit::singleline(&mut app.cfg.api_key)
                                    .password(true)
                                    .desired_width(ui.available_width()),
                            );
                        });
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(app.t("settings.base_url")).color(Theme::TEXT_DIM));
                            ui.text_edit_singleline(&mut app.cfg.base_url);
                        });
                    }
                }

                ui.add_space(2.0);

                ui.horizontal(|ui| {
                    ui.label(RichText::new(app.t("settings.model")).color(Theme::TEXT_DIM));
                    ui.text_edit_singleline(&mut app.cfg.model);
                    if pill_button(ui, app.t("settings.test"), true, false).clicked() {
                        app.test_ai();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new(app.t("settings.position")).color(Theme::TEXT_DIM));
                    ui.text_edit_singleline(&mut app.cfg.position);
                });
            });

            ui.add_space(6.0);

            glass_panel(ui, |ui| {
                section_heading(ui, app.t("settings.system_prompt"), "");
                copyable_text_edit(ui, "prompt", &mut app.cfg.system_prompt, 12);
            });
        });

        // === Колонка 1 (Whisper, настройки) ===
        cols[1].allocate_ui(egui::vec2(col_w, available.y), |ui| {
            ui.set_clip_rect(ui.max_rect());
            glass_panel(ui, |ui| {
                section_heading(ui, "Whisper", "");

                ui.label(RichText::new(app.t("settings.model")).color(Theme::TEXT_DIM));

                // Радиокнопки для всех 4 моделей
                for (kind, key) in WHISPER_MODELS {
                    let is_selected = app.selected_whisper_model == *key;
                    let radio = egui::RadioButton::new(is_selected, kind.label());
                    if ui.add(radio).clicked() {
                        app.selected_whisper_model = (*key).to_string();
                    }
                }

                ui.add_space(8.0);

                // === Кнопка Apply Changes с прогрессом прямо в надписи ===
                let is_busy = app.apply_progress.active;
                let whisper_ready = app.whisper.is_loaded() && app.whisper_status == app.t("whisper.ready");
                let label = if is_busy {
                    let pct = app.apply_progress.pct();
                    let base = app.t("settings.applying");
                    if pct > 0 {
                        format!("{} {}%", base, pct)
                    } else {
                        base.to_string()
                    }
                } else if whisper_ready {
                    app.t("settings.whisper_ready").to_string()
                } else {
                    app.t("settings.apply_changes").to_string()
                };
                if pill_button(ui, &label, !is_busy && !whisper_ready, is_busy).clicked() {
                    app.apply_whisper_selection();
                }

                // Прогресс-бар под кнопкой во время скачивания
                if is_busy
                    && (app.apply_progress.stage == "dll"
                        || app.apply_progress.stage == "model"
                        || app.apply_progress.stage == "download")
                {
                    ui.add_space(4.0);
                    let pct = app.apply_progress.pct();
                    let progress = if app.apply_progress.total > 0 {
                        app.apply_progress.downloaded as f32 / app.apply_progress.total as f32
                    } else {
                        0.0
                    };
                    let bar = egui::ProgressBar::new(progress)
                        .text(format!(
                            "{}: {}%",
                            app.apply_progress.name,
                            pct
                        ))
                        .desired_width(ui.available_width());
                    ui.add(bar);
                }

                ui.add_space(8.0);

                // chunk_ms и auto_ask
                ui.horizontal(|ui| {
                    ui.label(RichText::new(app.t("settings.chunk_ms")).color(Theme::TEXT_DIM));
                    ui.text_edit_singleline(&mut app.chunk_ms_text);
                    ui.label(RichText::new(app.t("settings.auto_ask_sec")).color(Theme::TEXT_DIM));
                    ui.text_edit_singleline(&mut app.auto_ask_text);
                });

                ui.add_space(4.0);

                // Статус Whisper
                ui.label(
                    RichText::new(format!("Whisper: {}", app.whisper_status))
                        .size(11.0)
                        .color(if app.whisper.is_loaded() { Theme::ACCENT } else { Theme::TEXT_DIM }),
                );
            });

            ui.add_space(6.0);

            // Конфиг
            glass_panel(ui, |ui| {
                ui.horizontal(|ui| {
                    let label = if app.config_edit_mode {
                        app.t("settings.hide_config")
                    } else {
                        app.t("settings.show_config")
                    };
                    if pill_button(ui, label, !app.config_edit_mode, app.config_edit_mode).clicked() {
                        app.config_edit_mode = !app.config_edit_mode;
                    }
                });
                if app.config_edit_mode {
                    ui.add_space(4.0);
                    section_heading(ui, app.t("settings.config_title"), "");
                    ui.horizontal(|ui| {
                        if pill_button(ui, app.t("settings.save"), true, false).clicked() {
                            app.save_config();
                            app.settings_saved_at = Some(Instant::now());
                        }
                        if pill_button(ui, app.t("settings.export"), false, false).clicked() {
                            app.export_config();
                        }
                        if pill_button(ui, app.t("settings.import"), false, false).clicked() {
                            app.import_config();
                        }
                    });
                    copyable_text_edit(ui, "cfg_preview", &mut app.config_preview, 10);
                }
            });

            ui.add_space(6.0);

            // Language selector
            glass_panel(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(app.t("settings.language")).color(Theme::TEXT_DIM));
                    let mut lang_str = app.lang.as_str().to_string();
                    let langs: Vec<String> = vec!["ru".into(), "en".into()];
                    let prev = lang_str.clone();
                    labeled_combo(ui, "", &mut lang_str, &langs);
                    if lang_str != prev {
                        app.lang = Lang::from_str(&lang_str);
                        app.save_config();
                    }
                });
            });
        });
    });

    // Temporary toast: "Settings saved"
    if let Some(t) = app.settings_saved_at {
        if t.elapsed().as_secs_f64() < 2.0 {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(app.t("settings.saved"))
                            .color(Theme::ACCENT)
                            .strong(),
                    );
                });
            });
        } else {
            app.settings_saved_at = None;
        }
    }
}