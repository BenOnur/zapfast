//! Anti-Delete plugin: Retains and displays deleted messages, videos, and voice notes.

use crate::app::App;
use crate::plugins::Plugin;

pub struct AntiDeletePlugin;

impl Plugin for AntiDeletePlugin {
    fn id(&self) -> &'static str {
        "anti_delete"
    }

    fn name(&self) -> &'static str {
        "Anti-Delete"
    }

    fn description(&self) -> &'static str {
        "Karşı tarafın sildiği mesajları, ses kayıtlarını ve videoları yerel arşivde tutar ve incelemenizi sağlar."
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

    fn on_message_deleted(&mut self, _chat: &str, _id: &str, _app: &mut App) -> bool {
        // Returning true informs the app shell that anti-delete is intercepting
        // the revocation, preserving the message locally instead of purging it.
        true
    }
}
