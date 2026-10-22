fn on_context_menu(event, state, config) {
    let text = event.text;
    text.trim();
    if text.len == 0 { return #{state:state}; }
    #{state:state, label:config.menu_label, command:"card"}
}

// The plugin owns composition. The host only draws bounded text/image primitives.
fn on_card(event, state, config) {
    let height = if event.portrait { 1350 } else { config.height };
    let bg = if event.dark { config.background } else { "#ffffff" };
    let fg = if event.dark { config.foreground } else { "#000000" };
    let x = if event.show_avatar { config.text_x } else { 100 };
    let width = if event.show_avatar { config.text_width } else { config.width - 200 };
    let shapes = [];
    if event.show_avatar {
        shapes.push(#{kind:"avatar", x:0, y:(height-config.avatar_size)/2,
          width:config.avatar_size, height:config.avatar_size, fade_start:200, fade_end:600});
    }
    shapes.push(#{kind:"text", x:x, y:60, width:width, height:height-120,
      text:event.text, subtitle:"- " + event.author, size:config.font_size,
      color:fg, gap:config.gap});
    #{state:state, scene:#{width:config.width,height:height,background:bg,shapes:shapes}}
}
