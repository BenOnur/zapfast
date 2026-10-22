fn on_message(event, state, config) {
    if event.kind != "text" { return #{state:state}; }
    let options = config;
    if event.account_config.len() > 0 { options = event.account_config; }
    if !options.enabled || !options.chats.contains(event.chat) { return #{state:state}; }
    let command = event.text;
    command.trim();
    for rule in options.rules {
        let configured = rule.command;
        configured.trim();
        let is_command = configured.starts_with("/");
        if !is_command { continue; }
        if configured != command { continue; }
        let reply = rule.reply;
        reply.trim();
        if reply == "" { continue; }
        let delay = 1;
        if options.contains("delay_seconds") { delay = options.delay_seconds; }
        return #{state:state, send:rule.reply, delay_seconds:delay};
    }
    #{state:state}
}
