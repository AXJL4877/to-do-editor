//! AI settings glue: save the dialog, ask the production host for the model
//! list or a connection test, and turn its answers into the dialog's view.
use super::*;
use crate::adapters::production::host::{class_message, HostReport};
use todo_interaction::{AvailableModelView, ModelCapability};
use todo_ui::AiSettingsAction;

impl App {
    pub(super) fn ai_settings_action(&mut self, action: AiSettingsAction) {
        match action {
            AiSettingsAction::None => {}
            AiSettingsAction::Save => {
                if self.save_ai_settings() {
                    self.list_models();
                }
            }
            AiSettingsAction::ListModels => self.list_models(),
            AiSettingsAction::TestConnection => {
                if !self.save_ai_settings() {
                    return;
                }
                let result = match self.production.as_mut() {
                    Some(host) => host.test_connection(),
                    None => Err("AI 服务没有启动".into()),
                };
                match result {
                    Ok(()) => {
                        self.ai_settings.huniu_testing = true;
                        self.ai_settings.huniu_connection = None;
                    }
                    Err(message) => self.ai_settings.huniu_connection = Some((false, message)),
                }
            }
        }
    }

    fn list_models(&mut self) {
        let result = match self.production.as_mut() {
            Some(host) => host.list_models(),
            None => Err("AI 服务没有启动".into()),
        };
        self.ai_settings.huniu_model_status = Some(match result {
            Ok(()) => "正在读取可用模型…".into(),
            Err(message) => message,
        });
    }

    /// Host answers that belong to the settings dialog; others are returned.
    pub(super) fn ai_report(&mut self, report: HostReport) -> Option<HostReport> {
        match report {
            HostReport::Models { provider, models } => {
                if provider != todo_production::Provider::Huniu {
                    return None;
                }
                let view = &mut self.ai_settings;
                view.huniu_models = models
                    .iter()
                    .map(|model| AvailableModelView {
                        id: model.id.clone(),
                        capabilities: model
                            .capabilities
                            .iter()
                            .map(|capability| match capability {
                                todo_ai::ModelCapability::Language => ModelCapability::Language,
                                todo_ai::ModelCapability::Image => ModelCapability::Image,
                                todo_ai::ModelCapability::Video => ModelCapability::Video,
                                todo_ai::ModelCapability::Multimodal => ModelCapability::Multimodal,
                                todo_ai::ModelCapability::Audio => ModelCapability::Audio,
                            })
                            .collect(),
                    })
                    .collect();
                view.huniu_model_status =
                    Some(format!("已读取 {} 个可用模型", view.huniu_models.len()));
                // A configured model the gateway no longer offers falls back
                // to the first one with the same capability.
                for (current, capability) in [
                    (&mut view.huniu_language_model, ModelCapability::Language),
                    (&mut view.huniu_image_model, ModelCapability::Image),
                    (&mut view.huniu_video_model, ModelCapability::Video),
                    (
                        &mut view.huniu_multimodal_model,
                        ModelCapability::Multimodal,
                    ),
                ] {
                    let offered = |m: &&AvailableModelView| m.capabilities.contains(&capability);
                    if !view
                        .huniu_models
                        .iter()
                        .filter(offered)
                        .any(|m| &m.id == current)
                    {
                        if let Some(model) = view.huniu_models.iter().find(offered) {
                            *current = model.id.clone();
                        }
                    }
                }
                None
            }
            HostReport::ModelsFailed {
                provider,
                class,
                message,
            } => {
                if provider == todo_production::Provider::Huniu {
                    self.ai_settings.huniu_model_status = Some(format!(
                        "读取模型列表失败：{}",
                        class_message(&class, &message)
                    ));
                }
                None
            }
            HostReport::Connection {
                provider,
                ok,
                message,
            } => {
                if provider == todo_production::Provider::Huniu {
                    self.ai_settings.huniu_testing = false;
                    self.ai_settings.huniu_connection = Some((ok, message));
                }
                None
            }
            other => Some(other),
        }
    }

    fn save_ai_settings(&mut self) -> bool {
        let root = paths::repo_root();
        let mut file = todo_ai::config::AiConfig::load_file(&root).unwrap_or_default();
        let before = AiConfig::load(&root).ok();
        file.huniu.base_url = Some(self.ai_settings.huniu_base_url.clone());
        file.huniu.strategy = (!self.ai_settings.huniu_strategy.trim().is_empty())
            .then(|| self.ai_settings.huniu_strategy.trim().to_owned());
        file.huniu.language_model = Some(self.ai_settings.huniu_language_model.clone());
        file.huniu.image_model = Some(self.ai_settings.huniu_image_model.clone());
        file.huniu.video_model = Some(self.ai_settings.huniu_video_model.clone());
        file.huniu.multimodal_model = Some(self.ai_settings.huniu_multimodal_model.clone());
        file.volc.base_url = Some(self.ai_settings.volc_base_url.clone());
        file.volc.audio_model = Some(self.ai_settings.volc_audio_model.clone());
        file.volc.speaker = Some(self.ai_settings.volc_speaker.clone());
        file.proxy = Some(match self.ai_settings.proxy {
            todo_interaction::ProxyChoice::Auto => todo_ai::ProxyMode::Auto,
            todo_interaction::ProxyChoice::System => todo_ai::ProxyMode::System,
            todo_interaction::ProxyChoice::Direct => todo_ai::ProxyMode::Direct,
        });
        let result = AiConfig::save(&root, &file)
            .map_err(|e| e.to_string())
            .and_then(|_| {
                let store = todo_ai::secrets::default_store();
                let secrets = Secrets {
                    store: store.as_ref(),
                    environment: &todo_ai::secrets::process_environment,
                };
                if self.ai_settings.huniu_key_source != todo_interaction::KeySource::Environment {
                    if self.ai_settings.huniu_clear_key {
                        secrets
                            .save(todo_production::Provider::Huniu, None)
                            .map_err(|e| e.to_string())?;
                        self.ai_settings.huniu_key = None;
                        self.ai_settings.huniu_key_source = todo_interaction::KeySource::Missing;
                        self.ai_settings.huniu_clear_key = false;
                        if let Some(host) = self.production.as_mut() {
                            host.set_key(todo_production::Provider::Huniu, None);
                        }
                    }
                    if let Some(value) = Secret::new(self.ai_settings.huniu_key_entry.clone()) {
                        secrets
                            .save(todo_production::Provider::Huniu, Some(&value))
                            .map_err(|e| e.to_string())?;
                        self.ai_settings.huniu_key = Some(value.masked());
                        self.ai_settings.huniu_key_source = todo_interaction::KeySource::Keychain;
                        self.ai_settings.huniu_key_entry.clear();
                        if let Some(host) = self.production.as_mut() {
                            host.set_key(todo_production::Provider::Huniu, Some(value));
                        }
                    }
                }
                if self.ai_settings.volc_key_source != todo_interaction::KeySource::Environment {
                    if self.ai_settings.volc_clear_key {
                        secrets
                            .save(todo_production::Provider::Volc, None)
                            .map_err(|e| e.to_string())?;
                        self.ai_settings.volc_key = None;
                        self.ai_settings.volc_key_source = todo_interaction::KeySource::Missing;
                        self.ai_settings.volc_clear_key = false;
                        if let Some(host) = self.production.as_mut() {
                            host.set_key(todo_production::Provider::Volc, None);
                        }
                    }
                    if let Some(value) = Secret::new(self.ai_settings.volc_key_entry.clone()) {
                        secrets
                            .save(todo_production::Provider::Volc, Some(&value))
                            .map_err(|e| e.to_string())?;
                        self.ai_settings.volc_key = Some(value.masked());
                        self.ai_settings.volc_key_source = todo_interaction::KeySource::Keychain;
                        self.ai_settings.volc_key_entry.clear();
                        if let Some(host) = self.production.as_mut() {
                            host.set_key(todo_production::Provider::Volc, Some(value));
                        }
                    }
                }
                Ok(())
            });
        // Models are read per request; addresses and the proxy need a new
        // client. Restarting interrupts requests in flight, so only then.
        let after = AiConfig::load(&root).ok();
        let endpoint = |c: &Option<AiConfig>| {
            c.as_ref().map(|c| {
                (
                    c.huniu.base_url.clone(),
                    c.volc.base_url.clone(),
                    format!("{:?}", c.proxy),
                    c.huniu.strategy.clone(),
                    c.huniu.language_model.clone(),
                    c.huniu.image_model.clone(),
                    c.huniu.video_model.clone(),
                    c.huniu.multimodal_model.clone(),
                    c.volc.audio_model.clone(),
                )
            })
        };
        let result = result.and_then(|()| {
            if endpoint(&before) != endpoint(&after) {
                if let Some(host) = self.production.as_mut() {
                    host.reconfigure()?;
                }
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                self.notice("AI 设置已保存".into());
                true
            }
            Err(error) => {
                self.notice(format!("AI 设置保存失败：{error}"));
                false
            }
        }
    }
}
