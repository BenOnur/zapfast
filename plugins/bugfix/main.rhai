fn on_layout(event, state, config) {
    let width = event.default_width;
    if event.region == "chat_header_title" && config.header_title_width {
        width = event.available_width;
    }
    #{ state: state, width: width }
}

fn on_media_policy(event, state, config) {
    #{ state: state, poster_first: event.kind == "sticker" && config.sticker_preview }
}
