fn on_layout(event, state, config) {
    if event.region == "chat_header_title" && config.header_title_width {
        return #{ state: state, width: event.available_width };
    }
    if event.region == "message_row" && config.message_row_width {
        return #{ state: state, width: event.available_width };
    }
    #{ state: state }
}

fn on_media_policy(event, state, config) {
    #{ state: state, poster_first: event.kind == "sticker" && config.sticker_preview }
}
