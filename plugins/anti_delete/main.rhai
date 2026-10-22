fn on_before_delete(event, state, config) {
    #{state:state, preserve: config.retain && event.has_content,
      max_media_mib: config.max_media_mib}
}

fn on_cache_media(event, state, config) {
    let media = event.mime.starts_with("video/") || event.mime.starts_with("audio/");
    #{state:state, download:config.retain && media && event.size <= config.max_media_mib*1024*1024}
}
