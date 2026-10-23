fn on_input_policy(event, state, config) {
    if event.gesture == "message_drag" {
        return #{ state: state, require_ctrl: config.ctrl_message_drag };
    }
    #{ state: state }
}
