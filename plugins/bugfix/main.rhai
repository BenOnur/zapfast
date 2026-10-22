fn on_layout(event, state, config) {
    let width = event.default_width;
    if event.region == "chat_header_title" && config.header_title_width {
        width = event.available_width;
    }
    #{ state: state, width: width }
}
