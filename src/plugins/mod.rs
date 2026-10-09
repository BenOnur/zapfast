//! Modular plugin system for ZapFast, inspired by Vencord/Equicord.
//!
//! Each plugin encapsulates custom UI extensions, event hooks, and lifecycle behaviors.

pub mod anti_delete;
pub mod auto_reply;
pub mod quoter;

use std::collections::HashMap;

use egui::Ui;
use serde::{Deserialize, Serialize};

use crate::app::App;
use crate::model::{Action, Message};
use crate::theme::Palette;

/// Metadata and behavior for an extensible plugin.
pub trait Plugin: Send + Sync + 'static {
    /// Unique identifier for settings and persistence (e.g., "quoter", "anti_delete").
    fn id(&self) -> &'static str;

    /// User-facing display name.
    fn name(&self) -> &'static str;

    /// Short explanation of what the plugin does.
    fn description(&self) -> &'static str;

    /// Authors of the plugin.
    fn authors(&self) -> &'static [&'static str] {
        &["Onur"]
    }

    /// Whether this plugin is enabled by default on clean installs.
    fn default_enabled(&self) -> bool {
        true
    }

    /// Whether this plugin offers configurable settings in the UI.
    fn has_settings(&self) -> bool {
        false
    }

    /// Renders items into the message right-click context menu.
    fn message_context_menu(
        &self,
        _ui: &mut Ui,
        _palette: &Palette,
        _message: &Message,
        _chat: &str,
        _actions: &mut Vec<Action>,
    ) {
    }

    /// Renders items into the chat list right-click context menu.
    fn chat_context_menu(
        &self,
        _ui: &mut Ui,
        _palette: &Palette,
        _chat: &str,
        _actions: &mut Vec<Action>,
    ) {
    }

    /// Renders items beside the message composer bar.
    fn composer_tools(
        &self,
        _ui: &mut Ui,
        _palette: &Palette,
        _chat: &str,
        _actions: &mut Vec<Action>,
    ) {
    }

    /// Renders the plugin-specific configuration panel inside Settings > Plugins.
    fn settings_ui(&mut self, _ui: &mut Ui, _app: &mut App) {}

    /// Intercepts and renders dedicated plugin modal dialogs.
    /// Returns true if the plugin consumed and drew a dialog.
    fn render_dialog(&mut self, _ctx: &egui::Context, _app: &mut App) -> bool {
        false
    }

    /// Hook triggered when a live incoming message is received.
    fn on_incoming_message(&mut self, _chat: &str, _message: &Message, _app: &mut App) {}

    /// Hook triggered when a message deletion is requested by the server.
    /// Returns true if the plugin intercepts and handles/preserves the deletion.
    fn on_message_deleted(&mut self, _chat: &str, _id: &str, _app: &mut App) -> bool {
        false
    }

    /// Hook triggered on every frame tick (for timers, pending queues, etc.).
    fn tick(&mut self, _app: &mut App) {}
}

/// Persisted configuration per plugin.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PluginConfig {
    pub enabled: bool,
}

/// Central registry and lifecycle manager for all ZapFast plugins.
pub struct PluginManager {
    plugins: Vec<Box<dyn Plugin>>,
    states: HashMap<String, bool>,
}

impl Default for PluginManager {
    fn default() -> Self {
        let mut manager = Self {
            plugins: Vec::new(),
            states: HashMap::new(),
        };

        // Register default built-in plugins
        manager.register(Box::new(quoter::QuoterPlugin));
        manager.register(Box::new(anti_delete::AntiDeletePlugin));
        manager.register(Box::new(auto_reply::AutoReplyPlugin));

        manager
    }
}

impl PluginManager {
    /// Registers a new plugin instance.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        let id = plugin.id().to_string();
        let default_state = plugin.default_enabled();
        self.states.entry(id).or_insert(default_state);
        self.plugins.push(plugin);
    }

    /// Synchronizes enabled states from loaded settings.
    pub fn apply_saved_states(&mut self, saved: &HashMap<String, PluginConfig>) {
        for (id, config) in saved {
            self.states.insert(id.clone(), config.enabled);
        }
    }

    /// Returns the persisted state map for settings serialization.
    pub fn export_states(&self) -> HashMap<String, PluginConfig> {
        self.states
            .iter()
            .map(|(k, &v)| (k.clone(), PluginConfig { enabled: v }))
            .collect()
    }

    /// Checks whether a specific plugin is enabled.
    pub fn is_enabled(&self, id: &str) -> bool {
        self.states.get(id).copied().unwrap_or(true)
    }

    /// Sets the enabled state of a plugin.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) {
        self.states.insert(id.to_string(), enabled);
    }

    /// Toggles a plugin enabled/disabled.
    pub fn toggle(&mut self, id: &str) {
        let current = self.is_enabled(id);
        self.set_enabled(id, !current);
    }

    /// Returns references to all registered plugins.
    pub fn all(&self) -> &[Box<dyn Plugin>] {
        &self.plugins
    }

    /// Returns a mutable slice of all registered plugins.
    pub fn all_mut(&mut self) -> &mut [Box<dyn Plugin>] {
        &mut self.plugins
    }

    /// Dispatches message context menu drawing to all enabled plugins.
    pub fn dispatch_message_context_menu(
        &self,
        ui: &mut Ui,
        palette: &Palette,
        message: &Message,
        chat: &str,
        actions: &mut Vec<Action>,
    ) {
        for plugin in &self.plugins {
            if self.is_enabled(plugin.id()) {
                plugin.message_context_menu(ui, palette, message, chat, actions);
            }
        }
    }

    /// Dispatches chat context menu drawing to all enabled plugins.
    pub fn dispatch_chat_context_menu(
        &self,
        ui: &mut Ui,
        palette: &Palette,
        chat: &str,
        actions: &mut Vec<Action>,
    ) {
        for plugin in &self.plugins {
            if self.is_enabled(plugin.id()) {
                plugin.chat_context_menu(ui, palette, chat, actions);
            }
        }
    }

    /// Dispatches composer toolbar drawing to all enabled plugins.
    pub fn dispatch_composer_tools(
        &self,
        ui: &mut Ui,
        palette: &Palette,
        chat: &str,
        actions: &mut Vec<Action>,
    ) {
        for plugin in &self.plugins {
            if self.is_enabled(plugin.id()) {
                plugin.composer_tools(ui, palette, chat, actions);
            }
        }
    }

    /// Dispatches incoming message events to all enabled plugins.
    pub fn dispatch_incoming_message(&mut self, chat: &str, message: &Message, app: &mut App) {
        // Collect enabled IDs to avoid borrow conflicts
        let enabled_ids: Vec<String> = self
            .plugins
            .iter()
            .filter(|p| self.is_enabled(p.id()))
            .map(|p| p.id().to_string())
            .collect();

        for id in enabled_ids {
            if let Some(plugin) = self.plugins.iter_mut().find(|p| p.id() == id) {
                plugin.on_incoming_message(chat, message, app);
            }
        }
    }

    /// Dispatches message deletion event. If any enabled plugin intercepts it, returns true.
    pub fn dispatch_message_deleted(&mut self, chat: &str, id: &str, app: &mut App) -> bool {
        let mut intercepted = false;
        let enabled_ids: Vec<String> = self
            .plugins
            .iter()
            .filter(|p| self.is_enabled(p.id()))
            .map(|p| p.id().to_string())
            .collect();

        for plugin_id in enabled_ids {
            if let Some(plugin) = self.plugins.iter_mut().find(|p| p.id() == plugin_id)
                && plugin.on_message_deleted(chat, id, app)
            {
                intercepted = true;
            }
        }
        intercepted
    }

    /// Dispatches tick to all enabled plugins.
    pub fn dispatch_tick(&mut self, app: &mut App) {
        let enabled_ids: Vec<String> = self
            .plugins
            .iter()
            .filter(|p| self.is_enabled(p.id()))
            .map(|p| p.id().to_string())
            .collect();

        for id in enabled_ids {
            if let Some(plugin) = self.plugins.iter_mut().find(|p| p.id() == id) {
                plugin.tick(app);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_manager_registers_core_plugins() {
        let manager = PluginManager::default();
        let registered_ids: Vec<&str> = manager.all().iter().map(|p| p.id()).collect();

        assert!(registered_ids.contains(&"quoter"));
        assert!(registered_ids.contains(&"anti_delete"));
        assert!(registered_ids.contains(&"auto_reply"));

        assert!(manager.is_enabled("quoter"));
        assert!(manager.is_enabled("anti_delete"));
        // Auto reply starts off by default on new setups until configured
        assert!(manager.is_enabled("auto_reply"));
    }

    #[test]
    fn plugin_states_can_be_toggled_and_queried() {
        let mut manager = PluginManager::default();
        assert!(manager.is_enabled("quoter"));

        manager.set_enabled("quoter", false);
        assert!(!manager.is_enabled("quoter"));

        manager.toggle("quoter");
        assert!(manager.is_enabled("quoter"));
    }

    #[test]
    fn plugin_states_export_and_apply_cleanly() {
        let mut manager = PluginManager::default();
        manager.set_enabled("quoter", false);
        manager.set_enabled("anti_delete", true);

        let exported = manager.export_states();
        assert_eq!(exported.get("quoter").map(|c| c.enabled), Some(false));
        assert_eq!(exported.get("anti_delete").map(|c| c.enabled), Some(true));

        let mut fresh = PluginManager::default();
        assert!(fresh.is_enabled("quoter"));
        fresh.apply_saved_states(&exported);
        assert!(!fresh.is_enabled("quoter"));
        assert!(fresh.is_enabled("anti_delete"));
    }

    #[test]
    fn plugin_metadata_is_coherent() {
        let manager = PluginManager::default();
        for plugin in manager.all() {
            assert!(!plugin.id().is_empty());
            assert!(!plugin.name().is_empty());
            assert!(!plugin.description().is_empty());
            assert!(!plugin.authors().is_empty());
        }
    }
}
