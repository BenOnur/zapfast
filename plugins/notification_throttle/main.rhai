// State is isolated by linked account and survives Reload plugins.
fn on_notification(event, state, config) {
    let interval = max(0, min(config.interval_seconds, 3600));
    for key in state.keys() {
        if event.now - state[key] >= 3600 { state.remove(key); }
    }
    let sender = event.sender;
    if sender in state && event.now - state[sender] < interval {
        return #{ allow: false, state: state };
    }
    state[sender] = event.now;
    #{ allow: true, state: state }
}
