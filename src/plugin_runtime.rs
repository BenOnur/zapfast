//! Local Rhai plugins, loaded independently of the application binary.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use rhai::{AST, Dynamic, Engine, Map, Scope};
use serde::Deserialize;

const MAX_FILE: u64 = 128 * 1024;
const MAX_PLUGINS: usize = 32;
const DEFAULT_MANIFEST: &str = include_str!("../plugins/notification_throttle/plugin.json");
const DEFAULT_SCRIPT: &str = include_str!("../plugins/notification_throttle/main.rhai");

#[derive(Clone, Debug)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub default_enabled: bool,
    pub error: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    api_version: u32,
    #[serde(default, rename = "ui")]
    _ui: serde_json::Value,
    id: String,
    name: String,
    description: String,
    #[serde(default = "enabled_by_default")]
    default_enabled: bool,
    hooks: Vec<String>,
    #[serde(default)]
    settings: serde_json::Map<String, serde_json::Value>,
}

fn enabled_by_default() -> bool {
    true
}

struct Script {
    info: PluginInfo,
    ast: AST,
    hooks: Vec<String>,
    config: Map,
    // Each linked account gets independent ephemeral plugin state.
    states: HashMap<String, Map>,
}

pub struct ScriptPlugins {
    engine: Engine,
    scripts: Vec<Script>,
    pub errors: Vec<String>,
}

impl Default for ScriptPlugins {
    fn default() -> Self {
        let mut engine = Engine::new();
        engine
            .set_max_operations(25_000)
            .set_max_call_levels(16)
            .set_max_expr_depths(32, 16)
            .set_max_variables(128)
            .set_max_functions(64)
            .set_max_string_size(16 * 1024)
            .set_max_array_size(2048)
            .set_max_map_size(2048);
        engine.disable_symbol("eval");
        // Rhai's standard sleep is a blocking native function, so operation
        // limits cannot interrupt it. Override both built-in overloads.
        engine.register_fn("sleep", |_: i64| -> Result<(), Box<rhai::EvalAltResult>> {
            Err("Blocking calls are unavailable".into())
        });
        engine.register_fn("sleep", |_: f64| -> Result<(), Box<rhai::EvalAltResult>> {
            Err("Blocking calls are unavailable".into())
        });
        // Script output may contain private messages. Never send it to logs.
        engine.on_print(|_| {});
        engine.on_debug(|_, _, _| {});
        Self {
            engine,
            scripts: Vec::new(),
            errors: Vec::new(),
        }
    }
}

impl ScriptPlugins {
    pub fn infos(&self) -> Vec<PluginInfo> {
        self.scripts
            .iter()
            .map(|script| script.info.clone())
            .collect()
    }

    /// Installs the bundled example only on the first migration. Removing it
    /// later is respected, and user edits are never overwritten.
    pub fn initialize(root: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(root)?;
        let marker = root.join(".initialized-v2");
        if !marker.exists() {
            for (id, manifest, script) in [
                ("notification_throttle", DEFAULT_MANIFEST, DEFAULT_SCRIPT),
                (
                    "auto_reply",
                    include_str!("../plugins/auto_reply/plugin.json"),
                    include_str!("../plugins/auto_reply/main.rhai"),
                ),
                (
                    "anti_delete",
                    include_str!("../plugins/anti_delete/plugin.json"),
                    include_str!("../plugins/anti_delete/main.rhai"),
                ),
                (
                    "quoter",
                    include_str!("../plugins/quoter/plugin.json"),
                    include_str!("../plugins/quoter/main.rhai"),
                ),
            ] {
                let directory = root.join(id);
                std::fs::create_dir_all(&directory)?;
                for (name, contents) in [("plugin.json", manifest), ("main.rhai", script)] {
                    use std::io::Write;
                    match std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(directory.join(name))
                    {
                        Ok(mut file) => file.write_all(contents.as_bytes())?,
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error),
                    }
                }
            }
            std::fs::write(marker, b"2\n")?;
        }
        Ok(())
    }

    /// Explicit reload keeps the last good version on parse failure and
    /// preserves account state across successful edits. Removed folders unload.
    pub fn reload(&mut self, root: &Path, reserved: &[&str]) {
        self.errors.clear();
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(_) => {
                self.errors.push("Eklenti klasörü okunamadı.".into());
                return;
            }
        };
        let mut paths: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .map(|entry| entry.path())
            .collect();
        paths.sort();
        if paths.len() > MAX_PLUGINS {
            self.errors
                .push("En fazla 32 dış eklenti yüklenebilir.".into());
        }
        let mut previous: HashMap<_, _> = std::mem::take(&mut self.scripts)
            .into_iter()
            .map(|script| (script.info.id.clone(), script))
            .collect();
        for path in paths.into_iter().take(MAX_PLUGINS) {
            let id = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if !safe_id(id) || reserved.contains(&id) {
                self.errors
                    .push("Geçersiz veya çakışan eklenti klasörü.".into());
                continue;
            }
            match self.load(&path, id) {
                Ok(mut script) => {
                    if let Some(old) = previous.remove(id) {
                        script.states = old.states;
                    }
                    self.scripts.push(script);
                }
                Err(()) => {
                    if let Some(script) = previous.remove(id) {
                        self.errors.push(format!(
                            "{id}: Dosyalar veya API sürümü geçersiz; son çalışan sürüm korundu."
                        ));
                        self.scripts.push(script);
                    } else {
                        self.errors.push(format!(
                            "{id}: Dosyalar veya API sürümü desteklenmiyor; eklenti yüklenmedi."
                        ));
                    }
                }
            }
        }
    }

    fn load(&self, directory: &Path, id: &str) -> Result<Script, ()> {
        let manifest: Manifest =
            serde_json::from_str(&read_small(&directory.join("plugin.json"))?).map_err(|_| ())?;
        if !(1..=3).contains(&manifest.api_version)
            || manifest.id != id
            || manifest.name.trim().is_empty()
            || manifest.name.len() > 128
            || manifest.description.len() > 1024
            || manifest.hooks.is_empty()
            || manifest.hooks.iter().any(|hook| {
                !matches!(
                    hook.as_str(),
                    "on_notification"
                        | "on_message"
                        | "on_before_delete"
                        | "on_cache_media"
                        | "on_context_menu"
                        | "on_card"
                        | "on_composer_tools"
                        | "on_layout"
                        | "on_media_policy"
                )
            })
        {
            return Err(());
        }
        let ast = self
            .engine
            .compile(read_small(&directory.join("main.rhai"))?)
            .map_err(|_| ())?;
        if manifest.hooks.iter().any(|hook| {
            !ast.iter_functions()
                .any(|function| function.name == hook && function.params.len() == 3)
        }) {
            return Err(());
        }
        let config = rhai::serde::to_dynamic(&manifest.settings)
            .map_err(|_| ())?
            .try_cast::<Map>()
            .ok_or(())?;
        Ok(Script {
            info: PluginInfo {
                id: id.into(),
                name: manifest.name,
                description: manifest.description,
                default_enabled: manifest.default_enabled,
                error: None,
            },
            ast,
            hooks: manifest.hooks,
            config,
            states: HashMap::new(),
        })
    }

    /// Runs bounded local scripts. Returned commands are validated by the host.
    pub fn values(
        &mut self,
        hook: &str,
        event: Map,
        enabled: &HashMap<String, bool>,
    ) -> Vec<(String, Map)> {
        let account = event
            .get("account")
            .and_then(|v| v.clone().try_cast::<String>())
            .unwrap_or_default();
        let mut outputs = Vec::new();
        for script in &mut self.scripts {
            if script.info.error.is_some()
                || !enabled
                    .get(&script.info.id)
                    .copied()
                    .unwrap_or(script.info.default_enabled)
                || !script.hooks.iter().any(|name| name == hook)
            {
                continue;
            }
            let state = script.states.get(&account).cloned().unwrap_or_default();
            let mut config = script.config.clone();
            if let Some(configs) = event
                .get("configs")
                .and_then(|v| v.clone().try_cast::<Map>())
                && let Some(overrides) = configs
                    .get(script.info.id.as_str())
                    .and_then(|v| v.clone().try_cast::<Map>())
            {
                config.extend(overrides);
            }
            let mut callback_event = event.clone();
            callback_event.remove("configs");
            let result = self.engine.call_fn::<Map>(
                &mut Scope::new(),
                &script.ast,
                hook,
                (callback_event, state, config),
            );
            let valid = result.ok().and_then(|mut result| {
                let state = result.remove("state")?.try_cast::<Map>()?;
                if !valid_state(&state) || !valid_state(&result) {
                    return None;
                }
                if hook == "on_notification"
                    && result
                        .get("allow")
                        .and_then(|v| v.clone().try_cast::<bool>())
                        .is_none()
                {
                    return None;
                }
                Some((state, result))
            });
            if let Some((state, result)) = valid {
                script.states.insert(account.clone(), state);
                outputs.push((script.info.id.clone(), result));
            } else {
                script.info.error =
                    Some("Çalışma hatası; dosyayı düzeltip yeniden yükleyin.".into());
            }
        }
        outputs
    }

    pub fn dispatch(
        &mut self,
        hook: &str,
        event: Map,
        enabled: &HashMap<String, bool>,
    ) -> (bool, Vec<String>) {
        let mut allow = true;
        let mut toasts = Vec::new();
        for (_, mut result) in self.values(hook, event, enabled) {
            if hook == "on_notification" {
                allow &= result
                    .remove("allow")
                    .and_then(|v| v.try_cast::<bool>())
                    .unwrap_or(true);
            }
            if hook == "on_message"
                && let Some(toast) = result.remove("toast").and_then(|v| v.try_cast::<String>())
            {
                toasts.push(toast.chars().take(500).collect());
            }
        }
        (allow, toasts)
    }
}

pub fn event(
    account: &str,
    chat: &str,
    sender: &str,
    message_id: &str,
    timestamp: i64,
    now: i64,
) -> Map {
    let mut event = Map::new();
    for (key, value) in [
        ("account", account),
        ("chat", chat),
        ("sender", sender),
        ("message_id", message_id),
    ] {
        event.insert(key.into(), Dynamic::from(value.to_owned()));
    }
    event.insert("timestamp".into(), Dynamic::from(timestamp));
    event.insert("now".into(), Dynamic::from(now));
    event
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

/// Bound cumulative state growth across calls, including nested collections.
/// Interpreter collection limits alone apply to each individual container.
fn valid_state(state: &Map) -> bool {
    fn value_fits(value: &Dynamic, depth: usize, bytes: &mut usize) -> bool {
        if depth > 16 || *bytes < 16 {
            return false;
        }
        *bytes -= 16;
        if let Some(map) = value.read_lock::<Map>() {
            return map.iter().all(|(key, value)| {
                if key.len() > *bytes {
                    return false;
                }
                *bytes -= key.len();
                value_fits(value, depth + 1, bytes)
            });
        }
        if let Some(array) = value.read_lock::<rhai::Array>() {
            return array
                .iter()
                .all(|value| value_fits(value, depth + 1, bytes));
        }
        if let Some(text) = value.read_lock::<rhai::ImmutableString>() {
            if text.len() > *bytes {
                return false;
            }
            *bytes -= text.len();
            return true;
        }
        value.is_unit() || value.is::<bool>() || value.is::<i64>() || value.is::<f64>()
    }
    let mut bytes = 128 * 1024;
    state.iter().all(|(key, value)| {
        if key.len() > bytes {
            return false;
        }
        bytes -= key.len();
        value_fits(value, 1, &mut bytes)
    })
}

fn read_small(path: &Path) -> Result<String, ()> {
    if std::fs::symlink_metadata(path)
        .map_err(|_| ())?
        .file_type()
        .is_symlink()
    {
        return Err(());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| ())?
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(());
    }
    String::from_utf8(bytes).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> (tempfile::TempDir, ScriptPlugins) {
        let directory = tempfile::tempdir().unwrap();
        ScriptPlugins::initialize(directory.path()).unwrap();
        // These existing API-v1 tests deliberately isolate the notification fixture.
        for id in ["auto_reply", "anti_delete", "quoter"] {
            std::fs::remove_dir_all(directory.path().join(id)).unwrap();
        }
        let mut host = ScriptPlugins::default();
        host.reload(directory.path(), &[]);
        assert!(host.errors.is_empty(), "{:?}", host.errors);
        (directory, host)
    }

    fn allowed(host: &mut ScriptPlugins, account: &str, sender: &str, time: i64) -> bool {
        host.dispatch(
            "on_notification",
            event(account, "chat", sender, "id", time, time),
            &HashMap::new(),
        )
        .0
    }

    #[test]
    fn notification_windows_are_fixed_and_isolated_by_sender_and_account() {
        let (_directory, mut host) = host();
        assert!(allowed(&mut host, "one", "a", 0));
        for time in 1..30 {
            assert!(!allowed(&mut host, "one", "a", time));
        }
        assert!(allowed(&mut host, "one", "b", 29));
        assert!(allowed(&mut host, "two", "a", 29));
        assert!(allowed(&mut host, "one", "a", 30));
        assert!(!allowed(&mut host, "one", "a", 31));
    }

    #[test]
    fn reload_changes_settings_preserves_state_and_respects_disable_and_remove() {
        let (directory, mut host) = host();
        assert!(allowed(&mut host, "one", "a", 0));
        let manifest = directory.path().join("notification_throttle/plugin.json");
        std::fs::write(&manifest, DEFAULT_MANIFEST.replace(": 30", ": 60")).unwrap();
        host.reload(directory.path(), &[]);
        assert!(!allowed(&mut host, "one", "a", 30));
        assert!(allowed(&mut host, "one", "a", 60));
        let disabled = HashMap::from([("notification_throttle".into(), false)]);
        assert!(
            host.dispatch(
                "on_notification",
                event("one", "chat", "a", "id", 61, 61),
                &disabled
            )
            .0
        );
        std::fs::remove_dir_all(directory.path().join("notification_throttle")).unwrap();
        ScriptPlugins::initialize(directory.path()).unwrap();
        host.reload(directory.path(), &[]);
        assert!(host.infos().is_empty());
        assert!(allowed(&mut host, "one", "a", 61));
    }

    #[test]
    fn malformed_edits_keep_last_good_code_and_runtime_faults_fail_open() {
        let (directory, mut host) = host();
        assert!(allowed(&mut host, "one", "a", 0));
        let script = directory.path().join("notification_throttle/main.rhai");
        std::fs::write(&script, "fn broken(").unwrap();
        host.reload(directory.path(), &[]);
        assert_eq!(host.errors.len(), 1);
        assert!(!allowed(&mut host, "one", "a", 1));
        std::fs::write(
            &script,
            "fn on_notification(event, state, config) { loop {} }",
        )
        .unwrap();
        host.reload(directory.path(), &[]);
        assert!(allowed(&mut host, "one", "a", 2));
        assert!(host.infos()[0].error.is_some());
        // Runtime faults remain disabled until an explicit reload.
        assert!(allowed(&mut host, "one", "a", 3));
    }

    #[test]
    fn new_plugin_is_discovered_without_native_registration() {
        let (directory, mut host) = host();
        let plugin = directory.path().join("message_counter");
        std::fs::create_dir(&plugin).unwrap();
        std::fs::write(
            plugin.join("plugin.json"),
            r#"{
          "api_version":1,"id":"message_counter","name":"Counter","description":"Local counter",
          "hooks":["on_message"]
        }"#,
        )
        .unwrap();
        std::fs::write(
            plugin.join("main.rhai"),
            r#"
          fn on_message(event, state, config) {
            if !("count" in state) { state.count = 0; }
            state.count += 1;
            #{state:state, toast:state.count.to_string()}
          }
        "#,
        )
        .unwrap();
        host.reload(directory.path(), &[]);
        assert_eq!(host.infos().len(), 2);
        for (account, expected) in [("one", "1"), ("one", "2"), ("two", "1")] {
            assert_eq!(
                host.dispatch(
                    "on_message",
                    event(account, "chat", "sender", "id", 0, 0),
                    &HashMap::new()
                )
                .1,
                [expected]
            );
        }
    }

    #[test]
    fn invalid_manifests_and_oversized_files_are_rejected() {
        let (directory, mut host) = host();
        let plugin = directory.path().join("oversized");
        std::fs::create_dir(&plugin).unwrap();
        std::fs::write(
            plugin.join("plugin.json"),
            " ".repeat(MAX_FILE as usize + 1),
        )
        .unwrap();
        host.reload(directory.path(), &[]);
        assert_eq!(host.errors.len(), 1);
        assert_eq!(host.infos().len(), 1);
        // A plugin cannot take over a built-in plugin's ID.
        host.reload(directory.path(), &["notification_throttle"]);
        assert!(host.infos().is_empty());
    }

    #[test]
    fn nested_state_cannot_grow_unbounded_across_messages() {
        let (directory, mut host) = host();
        std::fs::write(
            directory.path().join("notification_throttle/main.rhai"),
            "fn on_notification(event, state, config) { #{state: #{nested: state}, allow: false} }",
        )
        .unwrap();
        host.reload(directory.path(), &[]);
        for time in 0..16 {
            assert!(!allowed(&mut host, "one", "a", time));
        }
        assert!(allowed(&mut host, "one", "a", 16));
        assert!(host.infos()[0].error.is_some());
    }

    #[test]
    fn blocking_sleep_is_rejected_for_both_numeric_overloads() {
        let (directory, mut host) = host();
        for duration in ["0", "0.0"] {
            std::fs::write(directory.path().join("notification_throttle/main.rhai"),
                format!("fn on_notification(event, state, config) {{ sleep({duration}); #{{state:state, allow:false}} }}")).unwrap();
            host.reload(directory.path(), &[]);
            assert!(allowed(&mut host, "one", "a", 0));
            assert!(host.infos()[0].error.is_some());
        }
    }

    #[test]
    fn notification_event_excludes_content_and_display_names() {
        let event = event("one", "chat", "sender", "id", 1, 2);
        assert_eq!(event.len(), 6);
        assert!(!event.contains_key("text"));
        assert!(!event.contains_key("sender_name"));
        assert!(!event.contains_key("picture"));
    }
}
