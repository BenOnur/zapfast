use super::*;
use crate::model::{Delivery, Media, MediaState};

fn host() -> (tempfile::TempDir, Host) {
    let dir = tempfile::tempdir().unwrap();
    let host = Host::load(dir.path(), &HashMap::new());
    assert!(host.errors().is_empty(), "{:?}", host.errors());
    (dir, host)
}
fn message() -> Message {
    Message {
        id: "fixture-id".into(),
        chat: "fixture@g.us".into(),
        sender: "sender@s.whatsapp.net".into(),
        sender_name: None,
        from_me: false,
        timestamp: 1,
        history_order: None,
        content: Content::text("/hello"),
        status: Delivery::None,
        delivered_at: None,
        read_at: None,
        quoted: None,
        reactions: Vec::new(),
        edited: false,
        mentions: Vec::new(),
        forwarded: false,
        thumbnail: None,
    }
}
fn options() -> serde_json::Value {
    serde_json::json!({"enabled":true,"chats":["fixture@g.us"],"rules":[{"command":"/hello","reply":"Fixture reply"}],"delay_seconds":1})
}

#[test]
fn four_plugins_load_as_files_and_removal_removes_behavior() {
    let (dir, mut h) = host();
    assert_eq!(h.infos().len(), 4);
    assert!(h.before_delete("one", Some(&message())));
    assert_eq!(h.menus(&message()).len(), 1);
    for id in ["auto_reply", "anti_delete", "quoter"] {
        std::fs::remove_dir_all(dir.path().join(id)).unwrap();
    }
    h.reload();
    assert!(!h.before_delete("one", Some(&message())));
    assert!(h.menus(&message()).is_empty());
    h.incoming("one", &message(), options());
    assert!(h.due(Instant::now() + Duration::from_secs(60)).is_empty());
    ScriptPlugins::initialize(dir.path()).unwrap();
    assert!(
        !dir.path().join("quoter").exists(),
        "removed plugins must not be reseeded"
    );
}

#[test]
fn account_notification_settings_override_file_defaults() {
    let (_dir, mut h) = host();
    h.overrides.insert(
        "notification_throttle".into(),
        serde_json::json!({"interval_seconds":60}),
    );
    assert!(h.allow_notification("one", &message(), 0));
    assert!(!h.allow_notification("one", &message(), 30));
    assert!(h.allow_notification("two", &message(), 30));
    assert!(h.allow_notification("one", &message(), 60));
}

#[test]
fn autoreply_is_delayed_deduplicated_and_account_isolated() {
    let (_dir, mut h) = host();
    h.incoming("one", &message(), options());
    h.incoming("one", &message(), options());
    h.incoming("two", &message(), options());
    assert!(h.due(Instant::now()).is_empty());
    let ready = h.due(Instant::now() + Duration::from_secs(2));
    assert_eq!(ready.len(), 2);
    assert_eq!(ready[0].text, "Fixture reply");
    assert_ne!(ready[0].account, ready[1].account);
}

#[test]
fn disabling_reloading_and_manual_reply_cancel_pending_sends() {
    for mode in 0..3 {
        let (_dir, mut h) = host();
        h.incoming("one", &message(), options());
        match mode {
            0 => h.set_enabled("auto_reply", false),
            1 => h.reload(),
            _ => h.cancel_chat("one", "fixture@g.us"),
        }
        assert!(h.due(Instant::now() + Duration::from_secs(30)).is_empty());
    }
}

#[test]
fn autoreply_requires_enabled_rule_and_selected_chat() {
    for options in [
        serde_json::json!({"enabled":false,"chats":[],"rules":[]}),
        serde_json::json!({"enabled":true,"chats":[],"rules":[{"command":"/hello","reply":"X"}]}),
    ] {
        let (_dir, mut h) = host();
        h.incoming("one", &message(), options);
        assert!(h.due(Instant::now() + Duration::from_secs(30)).is_empty());
    }
}

#[test]
fn legacy_account_rules_and_other_fields_survive_serialization() {
    let settings:crate::settings::AccountSettings=serde_json::from_value(serde_json::json!({"auto_reply_enabled":true,"auto_reply_rules":[{"command":"/hello","reply":"Fixture reply"}],"auto_reply_chats":["fixture@g.us"]})).unwrap();
    let config = account_config(&settings, "auto_reply");
    assert_eq!(config["rules"][0]["reply"], "Fixture reply");
    let again: crate::settings::AccountSettings =
        serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
    assert_eq!(account_config(&again, "auto_reply"), config);
}

#[test]
fn old_deleted_content_and_late_media_stay_in_encrypted_archive_model() {
    let mut row = message();
    let media = Media {
        mime: "video/mp4".into(),
        size: 1024,
        width: None,
        height: None,
        album: None,
        path: None,
        state: MediaState::default(),
    };
    row.content = Content::Video {
        caption: None,
        seconds: None,
        gif: false,
        note: false,
        media,
    };
    let legacy = serde_json::json!({"kind":"revoked","deleted_content":row.content});
    row.content = parse_content(&legacy.to_string()).unwrap();
    assert!(matches!(row.content, Content::Preserved { .. }));
    let archive = crate::archive::Archive::in_memory().unwrap();
    archive.insert_message(&row, Some(b"fixture-raw")).unwrap();
    let path = std::path::Path::new("fixture-video.mp4");
    archive.set_media_path(&row.chat, &row.id, path).unwrap();
    let loaded = archive.message(&row.chat, &row.id).unwrap().unwrap();
    assert_eq!(loaded.content.media().unwrap().path.as_deref(), Some(path));
    assert!(
        archive
            .media_paths()
            .unwrap()
            .iter()
            .any(|(_, id, _)| id == &row.id)
    );
    assert!(
        matches!(preserve(&loaded.content),Content::Preserved{content} if !matches!(*content,Content::Preserved{..}))
    );
}

#[test]
fn file_edit_changes_card_layout_and_disable_removes_menu() {
    let (dir, mut h) = host();
    let draft = crate::plugin_card::Draft {
        scene: None,
        text: "Synthetic quote 🦀".into(),
        author: "Fixture".into(),
        avatar: None,
        show_avatar: true,
        dark: true,
        portrait: false,
    };
    let scene = h.scene("quoter", &draft).unwrap();
    assert_eq!((scene.width, scene.height), (1200, 600));
    let manifest = dir.path().join("quoter/plugin.json");
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    json["settings"]["width"] = 1000.into();
    std::fs::write(&manifest, serde_json::to_vec(&json).unwrap()).unwrap();
    h.reload();
    assert_eq!(h.scene("quoter", &draft).unwrap().width, 1000);
    h.set_enabled("quoter", false);
    assert!(h.menus(&message()).is_empty());
}

#[test]
fn generated_card_uses_the_script_scene_and_rejects_excessive_canvas() {
    let (_dir, h) = host();
    let mut draft = crate::plugin_card::Draft {
        scene: None,
        text: "Synthetic quote".into(),
        author: "Fixture".into(),
        avatar: None,
        show_avatar: false,
        dark: true,
        portrait: false,
    };
    draft.scene = h.scene("quoter", &draft);
    let mut renderer = crate::plugin_card::Renderer::new();
    let image = renderer.render(&draft).unwrap();
    assert_eq!((image.width, image.height), (1200, 600));
    assert!(
        image
            .bytes
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[0] > 150 && p[1] > 150)
    );
    draft.scene.as_mut().unwrap().width = 100000;
    assert!(renderer.render(&draft).is_err());
}

#[test]
fn composer_shortcut_loads_and_tracks_disable_reload_and_removal() {
    let (dir, mut h) = host();
    assert!(h.composer_tools().is_empty());
    let plugin = dir.path().join("quick_stickers");
    std::fs::create_dir(&plugin).unwrap();
    std::fs::write(
        plugin.join("plugin.json"),
        include_str!("../../plugins/quick_stickers/plugin.json"),
    )
    .unwrap();
    std::fs::write(
        plugin.join("main.rhai"),
        include_str!("../../plugins/quick_stickers/main.rhai"),
    )
    .unwrap();
    h.reload();
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    let buttons = h.composer_tools();
    assert_eq!(buttons.len(), 1);
    assert_eq!(buttons[0].0, "quick_stickers");
    assert_eq!(buttons[0].2, PickerTab::Stickers);
    h.set_enabled("quick_stickers", false);
    assert!(h.composer_tools().is_empty());
    h.set_enabled("quick_stickers", true);
    std::fs::write(
        plugin.join("main.rhai"),
        "fn on_composer_tools(e,s,c) { #{state:s,label:c.label,command: \"unknown\"} }",
    )
    .unwrap();
    h.reload();
    assert!(h.composer_tools().is_empty());
    std::fs::remove_dir_all(plugin).unwrap();
    h.reload();
    assert!(h.composer_tools().is_empty());
}

#[test]
fn bugfix_layout_uses_available_width_and_tracks_disable() {
    let (dir, mut h) = host();
    let plugin = dir.path().join("bugfix");
    std::fs::create_dir(&plugin).unwrap();
    std::fs::write(
        plugin.join("plugin.json"),
        include_str!("../../plugins/bugfix/plugin.json"),
    )
    .unwrap();
    std::fs::write(
        plugin.join("main.rhai"),
        include_str!("../../plugins/bugfix/main.rhai"),
    )
    .unwrap();
    h.reload();
    assert!(h.errors().is_empty(), "{:?}", h.errors());
    for available in [20.0, 120.0, 360.0, 640.0] {
        assert_eq!(
            h.layout_width("chat_header_title", available, 80.0),
            available
        );
    }
    assert_eq!(h.layout_width("other", 360.0, 80.0), 80.0);
    h.set_enabled("bugfix", false);
    assert_eq!(h.layout_width("chat_header_title", 360.0, 80.0), 80.0);
    h.set_enabled("bugfix", true);
    std::fs::write(
        plugin.join("main.rhai"),
        "fn on_layout(e,s,c) { #{state:s,width:99999.0} }",
    )
    .unwrap();
    h.reload();
    assert_eq!(h.layout_width("chat_header_title", 120.0, 80.0), 120.0);
}
