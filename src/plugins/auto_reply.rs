//! Auto Responder plugin: Automated command replies in configured chats with delay and cancellation.

use egui::Ui;

use crate::app::App;
use crate::model::Message;
use crate::plugins::Plugin;

pub struct AutoReplyPlugin;

impl Plugin for AutoReplyPlugin {
    fn id(&self) -> &'static str {
        "auto_reply"
    }

    fn name(&self) -> &'static str {
        "Auto Responder"
    }

    fn description(&self) -> &'static str {
        "Seçtiğiniz sohbetlerde gelen /komut mesajlarına belirlediğiniz kurallarla otomatik yanıt gönderir."
    }

    fn authors(&self) -> &'static [&'static str] {
        &["Onur"]
    }

    fn default_enabled(&self) -> bool {
        true
    }

    fn has_settings(&self) -> bool {
        true
    }

    fn settings_ui(&mut self, ui: &mut Ui, app: &mut App) {
        crate::ui::settings::render_auto_reply_plugin_editor(ui, app);
    }

    fn on_incoming_message(&mut self, chat: &str, message: &Message, app: &mut App) {
        app.maybe_auto_reply(chat, message);
    }
}
