//! Quote editor controls; all export side effects travel through Actions.
use crate::app::App;
use crate::model::Action;
use crate::quote::Export;

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let can_attach = app.quote_editor.as_ref().is_some_and(|editor| {
        app.account().id == editor.account
            && app.open_chat.as_ref() == Some(&editor.chat)
            && app.chat(&editor.chat).is_some_and(|chat| chat.can_send())
    });
    let Some(editor) = app.quote_editor.as_mut() else {
        return;
    };
    editor.poll(ui.ctx());
    ui.heading("Quote");
    ui.label("Mesajı bir alıntı kartına dönüştür.");
    let before = editor.draft.clone();
    let height = (ui.ctx().content_rect().height() - 220.0).max(100.0);
    egui::ScrollArea::vertical()
        .max_height(height)
        .show(ui, |ui| {
            if let Some(error) = &editor.error {
                ui.colored_label(app.palette.danger, error);
            }
            if let Some(notice) = &editor.notice {
                ui.label(notice);
            }
            if ui.available_width() >= 650.0 {
                ui.columns(2, |columns| {
                    controls(editor, &mut columns[0]);
                    preview(editor, &mut columns[1]);
                });
            } else {
                controls(editor, ui);
                ui.separator();
                preview(editor, ui);
            }
        });
    if before != editor.draft {
        editor.changed();
    }
    ui.separator();
    let ready = editor.image.is_some();
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(ready, egui::Button::new("PNG kaydet"))
            .clicked()
        {
            app.actions.push(Action::ExportQuote(Export::Save));
        }
        if ui
            .add_enabled(ready, egui::Button::new("Görseli kopyala"))
            .clicked()
        {
            app.actions.push(Action::ExportQuote(Export::Copy));
        }
        if ui
            .add_enabled(ready && can_attach, egui::Button::new("Sohbete ekle"))
            .on_hover_text("Gönderilmeden önce yazma alanına eklenir.")
            .clicked()
        {
            app.actions.push(Action::ExportQuote(Export::Attach));
        }
        if ui.button("Kapat").clicked() {
            app.actions.push(Action::CloseDialog);
        }
    });
}

fn controls(editor: &mut crate::quote::Editor, ui: &mut egui::Ui) {
    ui.label("Alıntı metni");
    ui.add(
        egui::TextEdit::multiline(&mut editor.draft.text)
            .desired_rows(7)
            .desired_width(f32::INFINITY)
            .char_limit(6000),
    );
    ui.label("Görünen isim");
    ui.add(
        egui::TextEdit::singleline(&mut editor.draft.author)
            .desired_width(f32::INFINITY)
            .char_limit(200),
    );
    ui.checkbox(&mut editor.draft.show_avatar, "Profil fotoğrafını göster");
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut editor.draft.dark, true, "Koyu");
        ui.selectable_value(&mut editor.draft.dark, false, "Açık");
    });
    ui.horizontal_wrapped(|ui| {
        ui.selectable_value(&mut editor.draft.portrait, false, "Kare · 1080×1080");
        ui.selectable_value(&mut editor.draft.portrait, true, "Dikey · 1080×1350");
    });
    if ui.button("Orijinale dön").clicked() {
        editor.draft = editor.original.clone();
    }
    ui.small("Bu düzenleme yalnız kartı değiştirir.");
}

fn preview(editor: &crate::quote::Editor, ui: &mut egui::Ui) {
    if let Some(texture) = &editor.texture {
        let width = ui.available_width().min(360.0);
        let height = width * texture.size()[1] as f32 / texture.size()[0] as f32;
        ui.add(egui::Image::new((texture.id(), egui::vec2(width, height))));
    } else if editor.error.is_none() {
        ui.spinner();
        ui.label("Kart hazırlanıyor…");
    }
}
