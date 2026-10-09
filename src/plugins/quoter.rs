//! Quoter plugin: Generates Equicord-style high resolution quote cards from messages.

use egui::Ui;

use crate::model::{Action, Message};
use crate::plugins::Plugin;
use crate::theme::{Icon, Palette};
use crate::ui::widgets;

pub struct QuoterPlugin;

impl Plugin for QuoterPlugin {
    fn id(&self) -> &'static str {
        "quoter"
    }

    fn name(&self) -> &'static str {
        "Quoter"
    }

    fn description(&self) -> &'static str {
        "Mesajları Equicord Quoter tarzı şık ve yüksek çözünürlüklü alıntı kartlarına dönüştürür."
    }

    fn authors(&self) -> &'static [&'static str] {
        &["Onur"]
    }

    fn default_enabled(&self) -> bool {
        true
    }

    fn has_settings(&self) -> bool {
        false
    }

    fn message_context_menu(
        &self,
        ui: &mut Ui,
        palette: &Palette,
        message: &Message,
        chat: &str,
        actions: &mut Vec<Action>,
    ) {
        if crate::quote::message_text(&message.content).is_some()
            && widgets::menu_item(ui, palette, Some(Icon::Quote), "Quote")
        {
            actions.push(Action::OpenQuote {
                chat: chat.to_string(),
                message: message.id.clone(),
            });
        }
    }
}
