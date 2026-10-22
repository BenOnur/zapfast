//! Local extension host. Policy and composition live in external scripts.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rhai::{Dynamic, Map};
use serde::{Deserialize, Serialize};

use crate::model::{Content, Message, PickerTab};
use crate::plugin_runtime::{ScriptPlugins, event};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginConfig {
    pub enabled: bool,
}

pub struct Pending {
    pub account: String,
    pub chat: String,
    pub plugin: String,
    pub text: String,
    pub due: Instant,
}

#[derive(Default)]
pub struct Host {
    runtime: RefCell<ScriptPlugins>,
    pub enabled: HashMap<String, bool>,
    pub root: PathBuf,
    pending: VecDeque<Pending>,
    recent: HashSet<(String, String, String)>,
    order: VecDeque<(String, String, String)>,
    pub edits: HashMap<String, String>,
    pub account: String,
    pub overrides: HashMap<String, serde_json::Value>,
    pub toasts: Vec<String>,
}

impl Host {
    pub fn load(root: &Path, saved: &HashMap<String, PluginConfig>) -> Self {
        let mut host = Self {
            root: root.into(),
            enabled: saved
                .iter()
                .map(|(id, c)| (id.clone(), c.enabled))
                .collect(),
            ..Self::default()
        };
        if ScriptPlugins::initialize(root).is_err() {
            host.runtime
                .get_mut()
                .errors
                .push("Eklenti klasörü hazırlanamadı.".into());
        }
        host.reload();
        host
    }
    pub fn reload(&mut self) {
        self.pending.clear();
        self.edits.clear();
        self.runtime.get_mut().reload(&self.root, &[]);
        for info in self.runtime.get_mut().infos() {
            self.enabled.entry(info.id).or_insert(info.default_enabled);
        }
    }
    pub fn saved(&self) -> HashMap<String, PluginConfig> {
        self.enabled
            .iter()
            .map(|(id, &enabled)| (id.clone(), PluginConfig { enabled }))
            .collect()
    }
    pub fn infos(&self) -> Vec<crate::plugin_runtime::PluginInfo> {
        self.runtime.borrow().infos()
    }
    pub fn errors(&self) -> Vec<String> {
        self.runtime.borrow().errors.clone()
    }
    pub fn is_enabled(&self, id: &str) -> bool {
        self.infos().iter().any(|i| i.id == id && i.error.is_none())
            && self.enabled.get(id).copied().unwrap_or(false)
    }
    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        self.enabled.insert(id.into(), enabled);
        self.pending.retain(|p| p.plugin != id);
    }
    pub fn call(&self, hook: &str, mut event: Map) -> Vec<(String, Map)> {
        event
            .entry("account".into())
            .or_insert_with(|| self.account.clone().into());
        event.insert(
            "configs".into(),
            rhai::serde::to_dynamic(&self.overrides).unwrap_or_default(),
        );
        self.runtime.borrow_mut().values(hook, event, &self.enabled)
    }
    /// Optional local rendering policies; no paths or message contents are exposed.
    pub fn media_poster_first(&self, kind: &str) -> bool {
        let mut event = Map::new();
        event.insert("kind".into(), kind.into());
        self.call("on_media_policy", event)
            .into_iter()
            .any(|(_, mut output)| {
                output
                    .remove("poster_first")
                    .and_then(|v| v.try_cast::<bool>())
                    .unwrap_or(false)
            })
    }
    /// Bounded layout policy; plugins cannot enlarge the parent allocation.
    pub fn layout_width(&self, region: &str, available: f32, fallback: f32) -> f32 {
        let available = available.max(0.0);
        let mut event = Map::new();
        event.insert("region".into(), region.into());
        event.insert("available_width".into(), (available as f64).into());
        event.insert("default_width".into(), (fallback as f64).into());
        self.call("on_layout", event)
            .into_iter()
            .filter_map(|(_, mut output)| {
                let value = output.remove("width")?.as_float().ok()?;
                value
                    .is_finite()
                    .then_some((value as f32).clamp(0.0, available))
            })
            .next_back()
            .unwrap_or(fallback)
    }
    /// Declarative picker shortcuts, placed immediately after the emoji control.
    pub fn composer_tools(&self) -> Vec<(String, String, PickerTab)> {
        self.call("on_composer_tools", Map::new())
            .into_iter()
            .filter_map(|(id, mut output)| {
                let command = output.remove("command")?.try_cast::<String>()?;
                let tab = match command.as_str() {
                    "open_stickers" => PickerTab::Stickers,
                    "open_emoji" => PickerTab::Emoji,
                    "open_gifs" => PickerTab::Gifs,
                    _ => return None,
                };
                let label = output.remove("label")?.try_cast::<String>()?;
                if label.trim().is_empty() || label.chars().count() > 80 {
                    return None;
                }
                Some((id, label, tab))
            })
            .take(4)
            .collect()
    }
    pub fn allow_notification(&self, account: &str, message: &Message, now: i64) -> bool {
        self.call(
            "on_notification",
            event(
                account,
                &message.chat,
                &message.sender,
                &message.id,
                message.timestamp,
                now,
            ),
        )
        .into_iter()
        .all(|(_, mut result)| {
            result
                .remove("allow")
                .and_then(|v| v.try_cast::<bool>())
                .unwrap_or(true)
        })
    }
    pub fn incoming(&mut self, account: &str, message: &Message, options: serde_json::Value) {
        let key = (account.to_owned(), message.chat.clone(), message.id.clone());
        if !self.recent.insert(key.clone()) {
            return;
        }
        self.order.push_back(key);
        while self.order.len() > 1024 {
            if let Some(old) = self.order.pop_front() {
                self.recent.remove(&old);
            }
        }
        let mut e = event(
            account,
            &message.chat,
            &message.sender,
            &message.id,
            message.timestamp,
            crate::util::now(),
        );
        e.insert(
            "text".into(),
            Dynamic::from(
                text(&message.content)
                    .chars()
                    .take(4096)
                    .collect::<String>(),
            ),
        );
        e.insert(
            "kind".into(),
            if matches!(message.content, Content::Text { .. }) {
                "text"
            } else {
                "other"
            }
            .into(),
        );
        e.insert(
            "account_config".into(),
            rhai::serde::to_dynamic(options).unwrap_or_else(|_| Dynamic::from(Map::new())),
        );
        for (plugin, mut output) in self.call("on_message", e) {
            if let Some(toast) = output.remove("toast").and_then(|v| v.try_cast::<String>())
                && self.toasts.len() < 32
            {
                self.toasts.push(toast.chars().take(500).collect());
            }
            if let Some(text) = output.remove("send").and_then(|v| v.try_cast::<String>())
                && !text.trim().is_empty()
                && text.len() <= 16_384
                && self.pending.len() < 128
            {
                let delay = output
                    .remove("delay_seconds")
                    .and_then(|v| v.try_cast::<i64>())
                    .unwrap_or(1)
                    .clamp(0, 300);
                self.pending.push_back(Pending {
                    account: account.into(),
                    chat: message.chat.clone(),
                    plugin,
                    text,
                    due: Instant::now() + Duration::from_secs(delay as u64),
                });
            }
        }
    }
    pub fn due(&mut self, now: Instant) -> Vec<Pending> {
        let mut ready = Vec::new();
        let queue = std::mem::take(&mut self.pending);
        for p in queue {
            if p.due <= now {
                if self.is_enabled(&p.plugin) {
                    ready.push(p);
                }
            } else {
                self.pending.push_back(p);
            }
        }
        ready
    }
    pub fn next_due(&self) -> Option<Duration> {
        self.pending
            .iter()
            .map(|p| p.due.saturating_duration_since(Instant::now()))
            .min()
    }
    pub fn cancel_chat(&mut self, account: &str, chat: &str) {
        self.pending
            .retain(|p| p.account != account || p.chat != chat);
    }
    pub fn before_delete(&self, account: &str, old: Option<&Message>) -> bool {
        let Some(old) = old else {
            return false;
        };
        if matches!(old.content, Content::Preserved { .. }) {
            return true;
        }
        if matches!(old.content, Content::Revoked) {
            return false;
        }
        let mut e = event(
            account,
            &old.chat,
            &old.sender,
            &old.id,
            old.timestamp,
            crate::util::now(),
        );
        e.insert("has_content".into(), true.into());
        self.call("on_before_delete", e)
            .into_iter()
            .any(|(_, mut output)| {
                output
                    .remove("preserve")
                    .and_then(|v| v.try_cast::<bool>())
                    .unwrap_or(false)
            })
    }
    pub fn cache_media(&self, account: &str, message: &Message) -> bool {
        let Some(media) = message.content.media() else {
            return false;
        };
        if media.path.is_some() || media.size > crate::model::ATTACHMENT_DOWNLOAD_LIMIT {
            return false;
        }
        let mut e = event(
            account,
            &message.chat,
            &message.sender,
            &message.id,
            message.timestamp,
            crate::util::now(),
        );
        e.insert(
            "size".into(),
            (media.size.min(i64::MAX as u64) as i64).into(),
        );
        e.insert("mime".into(), media.mime.clone().into());
        self.call("on_cache_media", e)
            .into_iter()
            .any(|(_, mut out)| {
                out.remove("download")
                    .and_then(|v| v.try_cast::<bool>())
                    .unwrap_or(false)
            })
    }
    pub fn menus(&self, message: &Message) -> Vec<(String, String)> {
        let mut e = event(
            &self.account,
            &message.chat,
            &message.sender,
            &message.id,
            message.timestamp,
            crate::util::now(),
        );
        e.insert(
            "text".into(),
            text(&message.content)
                .chars()
                .take(4096)
                .collect::<String>()
                .into(),
        );
        self.call("on_context_menu", e)
            .into_iter()
            .filter_map(|(id, mut output)| {
                if output.remove("command")?.try_cast::<String>()? != "card" {
                    return None;
                }
                let label = output.remove("label")?.try_cast::<String>()?;
                (!label.trim().is_empty() && label.len() <= 80).then_some((id, label))
            })
            .collect()
    }
    pub fn scene(
        &self,
        id: &str,
        draft: &crate::plugin_card::Draft,
    ) -> Option<crate::plugin_card::Scene> {
        let mut e = Map::new();
        for (key, value) in [
            ("text", draft.text.as_str()),
            ("author", draft.author.as_str()),
        ] {
            e.insert(key.into(), value.to_owned().into());
        }
        for (key, value) in [
            ("dark", draft.dark),
            ("portrait", draft.portrait),
            ("show_avatar", draft.show_avatar),
        ] {
            e.insert(key.into(), value.into());
        }
        for (plugin, mut output) in self.call("on_card", e) {
            if plugin == id {
                // Rhai distinguishes integer and float deserialization strictly.
                // JSON normalizes numeric coordinates for the typed renderer.
                let value =
                    rhai::serde::from_dynamic::<serde_json::Value>(&output.remove("scene")?)
                        .ok()?;
                let scene = serde_json::from_value::<crate::plugin_card::Scene>(value).ok()?;
                return scene.valid().then_some(scene);
            }
        }
        None
    }
}

pub fn preserve(content: &Content) -> Content {
    if matches!(content, Content::Preserved { .. }) {
        content.clone()
    } else {
        Content::Preserved {
            content: Box::new(content.clone()),
        }
    }
}

pub fn text(content: &Content) -> &str {
    match content {
        Content::Text { text, .. } | Content::Interactive { text, .. } => text,
        Content::Image { caption, .. }
        | Content::Video { caption, .. }
        | Content::Document { caption, .. } => caption.as_deref().unwrap_or(""),
        Content::Preserved { content } => text(content),
        _ => "",
    }
}

/// Settings use the same JSON editor for every plugin. No native feature registry.
pub fn settings(app: &mut crate::app::App, ui: &mut egui::Ui) {
    use crate::model::Action;
    ui.horizontal(|ui| {
        if ui.button("Eklenti klasörünü aç").clicked() {
            app.actions.push(Action::OpenFolder(app.dirs.plugins_dir()));
        }
        if ui.button("Yeniden yükle").clicked() {
            app.actions.push(Action::ReloadPlugins);
        }
    });
    for info in app.plugins.infos() {
        let mut enabled = app.plugins.is_enabled(&info.id);
        if ui.checkbox(&mut enabled, &info.name).changed() {
            app.actions.push(Action::SetPluginEnabled {
                id: info.id.clone(),
                enabled,
            });
        }
        ui.label(&info.description);
        if let Some(error) = info.error {
            ui.colored_label(app.palette.danger, error);
        }
        ui.collapsing(format!("{} · hesaba özel ayarlar", info.name), |ui| {
            let current = account_config(&app.account().settings, &info.id);
            let manifest_path = app.dirs.plugins_dir().join(&info.id).join("plugin.json");
            let manifest = std::fs::metadata(&manifest_path)
                .ok()
                .filter(|m| m.len() <= 128 * 1024)
                .and_then(|_| std::fs::read(&manifest_path).ok())
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .unwrap_or(serde_json::json!({}));
            let initial = if current.as_object().is_some_and(|o| !o.is_empty()) {
                current
            } else {
                manifest
                    .get("settings")
                    .cloned()
                    .unwrap_or(serde_json::json!({}))
            };
            let chats: Vec<_> = app
                .chats
                .iter()
                .filter(|c| !c.locked && c.can_send())
                .map(|c| (c.id.clone(), app.chat_title(c)))
                .collect();
            let key = format!("{}:{}", app.account().id.0, info.id);
            let buffer = app
                .plugins
                .edits
                .entry(key)
                .or_insert_with(|| serde_json::to_string_pretty(&initial).unwrap_or_default());
            if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(buffer) {
                let before = value.clone();
                fields(
                    ui,
                    &mut value,
                    manifest.get("ui").unwrap_or(&serde_json::Value::Null),
                    &chats,
                );
                if before != value {
                    *buffer = serde_json::to_string_pretty(&value).unwrap_or_default();
                }
            }
            ui.collapsing("Gelişmiş: ayar dosyası", |ui| {
                ui.add(
                    egui::TextEdit::multiline(buffer)
                        .code_editor()
                        .desired_rows(8)
                        .char_limit(32_768),
                );
            });
            if ui.button("Ayarları kaydet").clicked() {
                app.actions.push(Action::SavePluginSettings {
                    id: info.id.clone(),
                    json: buffer.clone(),
                });
            }
        });
        ui.separator();
    }
    for error in app.plugins.errors() {
        ui.colored_label(app.palette.danger, error);
    }
}

pub fn account_config(settings: &crate::settings::AccountSettings, id: &str) -> serde_json::Value {
    if let Some(value) = settings.plugin_settings.get(id) {
        return value.clone();
    }
    if id == "auto_reply" && settings.legacy.contains_key("auto_reply_enabled") {
        return serde_json::json!({"enabled":settings.legacy.get("auto_reply_enabled").unwrap_or(&serde_json::Value::Bool(false)),"rules":settings.legacy.get("auto_reply_rules").cloned().unwrap_or(serde_json::json!([])),"chats":settings.legacy.get("auto_reply_chats").cloned().unwrap_or(serde_json::json!([])),"delay_seconds":1});
    }
    serde_json::json!({})
}

fn fields(
    ui: &mut egui::Ui,
    value: &mut serde_json::Value,
    schema: &serde_json::Value,
    chats: &[(String, String)],
) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    for (key, value) in object {
        let label = schema
            .get("labels")
            .and_then(|v| v.get(key))
            .and_then(|v| v.as_str())
            .unwrap_or(key)
            .to_owned();
        ui.push_id(key, |ui| match value {
            serde_json::Value::Bool(v) => {
                ui.checkbox(v, &label);
            }
            serde_json::Value::String(v) => {
                ui.label(&label);
                ui.add(egui::TextEdit::singleline(v).char_limit(16_384));
            }
            serde_json::Value::Number(v) => {
                let mut number = v.as_i64().unwrap_or(0);
                ui.horizontal(|ui| {
                    ui.label(&label);
                    if ui
                        .add(egui::DragValue::new(&mut number).range(0..=4096))
                        .changed()
                    {
                        *v = number.into();
                    }
                });
            }
            serde_json::Value::Array(items) => {
                ui.label(&label);
                let chat_field = schema
                    .get("chat_fields")
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(key)));
                if chat_field {
                    egui::ScrollArea::vertical()
                        .max_height(200.0)
                        .id_salt("chat-options")
                        .show(ui, |ui| {
                            for (id, title) in chats {
                                let mut selected = items.iter().any(|v| v.as_str() == Some(id));
                                if ui.checkbox(&mut selected, title).changed() {
                                    items.retain(|v| v.as_str() != Some(id));
                                    if selected {
                                        items.push(id.clone().into());
                                    }
                                }
                            }
                        });
                } else {
                    let mut remove = None;
                    for (index, item) in items.iter_mut().enumerate() {
                        ui.push_id(index, |ui| {
                            ui.group(|ui| {
                                fields(ui, item, schema, chats);
                                if ui.button("Kaldır").clicked() {
                                    remove = Some(index);
                                }
                            });
                        });
                    }
                    if let Some(index) = remove {
                        items.remove(index);
                    }
                    if let Some(template) = schema.get("templates").and_then(|v| v.get(key))
                        && items.len() < 128
                        && ui.button("Ekle").clicked()
                    {
                        items.push(template.clone());
                    }
                }
            }
            serde_json::Value::Object(_) => {
                ui.collapsing(&label, |ui| fields(ui, value, schema, chats));
            }
            _ => {}
        });
    }
}

/// Compatibility with the previous custom archive's deleted-content envelope.
pub fn deserialize_content<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Content, D::Error> {
    let mut value = serde_json::Value::deserialize(d)?;
    convert_legacy(&mut value);
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}
pub fn convert_legacy(value: &mut serde_json::Value) {
    if value.get("kind").and_then(|v| v.as_str()) == Some("revoked")
        && let Some(mut content) = value
            .get("deleted_content")
            .filter(|v| !v.is_null())
            .cloned()
    {
        convert_legacy(&mut content);
        *value = serde_json::json!({"kind":"preserved","content":content});
    }
}
pub fn parse_content(raw: &str) -> serde_json::Result<Content> {
    let mut value = serde_json::from_str(raw)?;
    convert_legacy(&mut value);
    serde_json::from_value(value)
}

#[cfg(test)]
mod tests;
