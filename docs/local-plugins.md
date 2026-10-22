# Local plugins, API v2

This build follows upstream commit `bde5bfdc4fdf531d84b9aec32f134fd3f390152e` and adds a local Rhai host. It uses the normal upstream interface; previous private themes and interface changes are not carried forward.

Open **Settings > Plugins > Eklenti klasörünü aç**. Every plugin has its own folder with `plugin.json` and `main.rhai`. Use **Yeniden yükle** after adding or editing files. Application recompilation is unnecessary within this API. New native capabilities need a host update.

The four seeded plugins are `notification_throttle`, `auto_reply`, `anti_delete`, and `quoter`. Seeding never overwrites existing files and respects removal after the first migration. Plugin switches, account-specific settings and files survive application updates. A reload cancels pending sends. Removing or disabling a plugin removes its menu entries and future behavior; retained archive data is not erased.

The manifest declares `api_version` (1 or 2), `id` matching its folder, `name`, `description`, `default_enabled`, `hooks`, and `settings`. Optional `ui` metadata supplies field labels, list item templates and chat selectors for the generic settings editor. Account overrides merge into defaults. Previous `auto_reply_enabled`, `auto_reply_rules` and `auto_reply_chats` settings remain readable and editable without exporting chats.

Every hook is `fn hook(event, state, config)` and returns a map with `state`. State is isolated by account and plugin, preserved across successful reloads and cleared at restart. The host provides no arbitrary filesystem, network, module import, blocking sleep, or evaluation API. Scripts run locally and are trusted to inspect the event fields exposed to their declared hooks.

| Hook | Event / return |
| --- | --- |
| `on_notification` | Account/chat/sender/message IDs, timestamps and `now`; return `allow`. No message text or pictures. |
| `on_message` | Eligible live incoming text and account options; optional `send` and `delay_seconds` (0–300). Host rechecks connection, lock and chat eligibility before sending. |
| `on_before_delete` | Metadata and `has_content`; return `preserve`. Runs in the backend before a server revoke is applied. |
| `on_cache_media` | Metadata, MIME and declared byte size; return `download`. Host enforces its 64 MiB streaming limit. |
| `on_context_menu` | Message text and metadata; return a short `label` and `command: "card"`. |
| `on_card` | Edited text, author and display options; return `scene`. The plugin owns layout, dimensions, colors and text composition. |

Scenes contain width/height (64–2048), a `#rrggbb` background and up to 16 bounded `avatar` or `text` shapes. The native renderer handles shaping, emoji, images, rasterization and PNG/clipboard/attachment export. Scripts cannot supply arbitrary image paths; the host supplies the selected message's profile picture. Attaching a card does not send it automatically.

The throttle uses fixed windows per sender and account. The first eligible notification passes; further messages for 30 seconds stay quiet. The next eligible message after the window passes and starts another window. No delayed notification is scheduled. Normal muted/locked/chat eligibility and OS notification policy still apply; the plugin cannot override Windows Do Not Disturb.

Anti-Delete retains locally available content and proactively downloads eligible new audio/video up to 64 MiB. It cannot recover content never received. Files finishing after a revoke stay attached to the retained encrypted archive entry. Previous custom `revoked.deleted_content` rows remain readable. Turning it off stops new retention; it does not delete existing local copies.

Host limits: 32 plugins, 128 KiB per file, 25,000 operations per callback, bounded recursion/collections and 128 KiB returned-state budget. Invalid file edits retain the last loaded version with an error. Runtime faults disable that plugin until reload; notification faults fail open. Errors never expose message bodies in logs.

## Updates and build footprint

This fork uses `BenOnur/zapfast` releases and `assets/plugin-update-public-key.hex`, not upstream's signing key. The private key belongs in the repository secret `ZAPFAST_PLUGIN_UPDATE_SIGNING_KEY` and outside the source checkout. **Plugins Windows** performs the required Windows checks, builds a portable zip, signs its checksums, verifies them and optionally publishes a release. No Rust compiler is needed on the receiving machine.

**Plugin upstream compatibility** previews upstream integration weekly without changing main or publishing binaries. A clean textual merge is not proof of compatibility: adaptation and full checks still precede each release. Official stock exe updates omit this host. The installed exe should live outside Cargo target directories. Local cache or compiler removal is a separate operation; this feature does not delete them automatically.
