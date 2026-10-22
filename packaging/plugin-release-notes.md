Clean upstream ZapFast with local file plugins. Auto Responder, Anti-Delete, Quoter and Notification Throttle load from the Plugins folder and can be reloaded without rebuilding the app.

## New

* **Four separate plugins.** Rules, notification timing, retention decisions and card composition live in editable local files.
* **Signed plugin builds.** Updates use BenOnur/zapfast and its own publisher key. The official binary does not contain this extension host.

## Fixed

* **Existing rules and retained messages stay readable.** Account settings and the previous deleted-message format are supported.
* **TLS trust fixes.** Updated rustls-webpki to 0.103.16 and webpki-roots to 1.0.10 for [RUSTSEC-2026-0382](https://rustsec.org/advisories/RUSTSEC-2026-0382) and [RUSTSEC-2026-0383](https://rustsec.org/advisories/RUSTSEC-2026-0383).

## Limits

This package targets Windows x64. Script changes work within API v2; new native capabilities still need an application update. Upstream changes require adaptation and verification before publishing here. Plugin files and user data are outside the portable application folder.

## Thanks

crmne and ZapFast contributors for the native client; Onur for the plugin requirements and testing.

[Full changelog](https://github.com/BenOnur/zapfast/compare/f258f559fed6f9f123bf612fda63ccb68136aeeb...v0.19.1)
