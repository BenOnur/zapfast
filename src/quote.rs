//! Local quote cards. The editor previews these exact pixels, never a second layout.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use egui::{Color32, Pos2, Rect, TextureId, pos2, vec2};

use crate::model::{AccountId, Content, DecodedImage};

#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub text: String,
    pub author: String,
    pub avatar: Option<PathBuf>,
    pub show_avatar: bool,
    pub dark: bool,
    pub portrait: bool,
}

pub fn message_text(content: &Content) -> Option<&str> {
    let text = match content {
        Content::Text { text, .. } | Content::Interactive { text, .. } => Some(text.as_str()),
        Content::Image { caption, .. }
        | Content::Video { caption, .. }
        | Content::Document { caption, .. } => caption.as_deref(),
        _ => None,
    }?;
    (!text.trim().is_empty()).then_some(text)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Export {
    Copy,
    Save,
    Attach,
}

/// Ephemeral editor state, owned by the application shell and cleared on lock/switch.
pub struct Editor {
    pub account: AccountId,
    pub chat: String,
    pub draft: Draft,
    pub original: Draft,
    pub image: Option<Arc<DecodedImage>>,
    pub texture: Option<egui::TextureHandle>,
    pub error: Option<String>,
    pub notice: Option<String>,
    revision: u64,
    requests: mpsc::Sender<(u64, Draft)>,
    results: mpsc::Receiver<(u64, Result<DecodedImage, String>)>,
    saving: Option<mpsc::Receiver<Result<bool, String>>>,
}

impl Editor {
    pub fn new(account: AccountId, chat: String, draft: Draft) -> Self {
        let (requests, input) = mpsc::channel::<(u64, Draft)>();
        let (output, results) = mpsc::channel();
        std::thread::spawn(move || {
            let mut renderer = Renderer::new();
            while let Ok(mut request) = input.recv() {
                // Editing can outrun rendering. Only draw the newest queued revision.
                for newer in input.try_iter() {
                    request = newer;
                }
                let result = renderer.render(&request.1);
                if output.send((request.0, result)).is_err() {
                    break;
                }
            }
        });
        let mut editor = Self {
            account,
            chat,
            original: draft.clone(),
            draft,
            image: None,
            texture: None,
            error: None,
            notice: None,
            revision: 0,
            requests,
            results,
            saving: None,
        };
        editor.changed();
        editor
    }

    pub fn changed(&mut self) {
        self.revision += 1;
        self.image = None;
        self.texture = None;
        self.error = None;
        self.notice = None;
        let _ = self.requests.send((self.revision, self.draft.clone()));
    }

    pub fn poll(&mut self, ctx: &egui::Context) {
        for (revision, result) in self.results.try_iter() {
            if revision != self.revision {
                continue;
            }
            match result {
                Ok(image) => {
                    self.texture = Some(ctx.load_texture(
                        "quote-card",
                        egui::ColorImage::from_rgba_unmultiplied(
                            [image.width, image.height],
                            &image.bytes,
                        ),
                        egui::TextureOptions::LINEAR,
                    ));
                    self.image = Some(Arc::new(image));
                }
                Err(error) => self.error = Some(error),
            }
        }
        if let Some(saving) = &self.saving
            && let Ok(result) = saving.try_recv()
        {
            self.saving = None;
            match result {
                Ok(true) => self.notice = Some("PNG kaydedildi.".into()),
                Ok(false) => {}
                Err(error) => self.notice = Some(error),
            }
        }
        if (self.image.is_none() && self.error.is_none()) || self.saving.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }

    pub fn save(&mut self) {
        let Some(image) = self.image.clone() else {
            return;
        };
        if self.saving.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        self.saving = Some(receiver);
        std::thread::spawn(move || {
            let result = match rfd::FileDialog::new()
                .add_filter("PNG", &["png"])
                .set_file_name("quote.png")
                .save_file()
            {
                None => Ok(false),
                Some(path) => image::save_buffer_with_format(
                    path,
                    &image.bytes,
                    image.width as u32,
                    image.height as u32,
                    image::ColorType::Rgba8,
                    image::ImageFormat::Png,
                )
                .map(|()| true)
                .map_err(|_| "PNG kaydedilemedi. Dosya konumunu kontrol et.".into()),
            };
            let _ = sender.send(result);
        });
    }
}

/// egui supplies native font shaping and its glyph atlas; a small CPU compositor
/// renders the textured meshes on a worker without a graphics context or browser.
pub struct Renderer {
    ctx: egui::Context,
    emoji: fastframe_emoji::Emoji,
    textures: HashMap<TextureId, egui::ColorImage>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    pub fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.set_fonts(
            fastframe_fonts::FontSetup::default()
                .primary(fastframe_fonts::Primary::Inter)
                .definitions(),
        );
        Self {
            ctx,
            emoji: fastframe_emoji::EmojiSetup::default()
                .bundled(include_bytes!("../assets/fonts/NotoColorEmoji.ttf"))
                .load(),
            textures: HashMap::new(),
        }
    }

    pub fn render(&mut self, draft: &Draft) -> Result<DecodedImage, String> {
        if draft.text.trim().is_empty() {
            return Err("Alıntı metni boş olamaz.".into());
        }
        if draft.text.chars().count() > 6000 || draft.author.chars().count() > 200 {
            return Err("Metin çok uzun. Alıntıyı 6000, ismi 200 karakterin altına indir.".into());
        }
        let height = if draft.portrait { 1350 } else { 1080 };
        let background = if draft.dark {
            Color32::from_rgb(21, 23, 29)
        } else {
            Color32::from_rgb(246, 243, 235)
        };
        let foreground = if draft.dark {
            Color32::from_rgb(246, 243, 235)
        } else {
            Color32::from_rgb(27, 30, 36)
        };
        let accent = Color32::from_rgb(176, 148, 105);
        let mut error = None;
        let mut held = Vec::new();
        let avatar = draft
            .avatar
            .as_ref()
            .and_then(|path| image::ImageReader::open(path).ok())
            .and_then(|reader| reader.with_guessed_format().ok())
            .and_then(|mut reader| {
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(4096);
                limits.max_image_height = Some(4096);
                reader.limits(limits);
                reader.decode().ok()
            });
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1080.0, height as f32))),
                ..Default::default()
            },
            |ui| {
                let painter = ui.painter();
                painter.rect_filled(
                    Rect::from_min_size(Pos2::ZERO, vec2(1080.0, height as f32)),
                    0.0,
                    background,
                );
                painter.text(
                    pos2(100.0, 100.0),
                    egui::Align2::LEFT_TOP,
                    "“",
                    egui::FontId::proportional(144.0),
                    accent,
                );
                let mut size = 64.0;
                let (galley, placements) = loop {
                    let layout = layout(ui, &draft.text, size, foreground, 880.0);
                    if layout.0.size().y <= height as f32 - 490.0 && layout.0.size().x <= 881.0 {
                        break layout;
                    }
                    size -= 2.0;
                    if size < 28.0 {
                        error = Some(
                            "Metin bu karta okunaklı sığmıyor. Metni kısalt veya dikey boyutu seç."
                                .into(),
                        );
                        return;
                    }
                };
                let origin = pos2(
                    100.0,
                    250.0 + (height as f32 - 490.0 - galley.size().y) / 2.0,
                );
                paint_text(
                    ui,
                    &self.emoji,
                    galley,
                    &placements,
                    origin,
                    foreground,
                    &mut held,
                );
                let author_y = height as f32 - 150.0;
                painter.line_segment(
                    [pos2(100.0, author_y - 48.0), pos2(200.0, author_y - 48.0)],
                    egui::Stroke::new(3.0, accent),
                );
                let name_x = if draft.show_avatar { 200.0 } else { 100.0 };
                if draft.show_avatar {
                    let rect = Rect::from_min_size(pos2(100.0, author_y - 20.0), vec2(72.0, 72.0));
                    if let Some(avatar) = &avatar {
                        let mut rgba = avatar
                            .resize_to_fill(144, 144, image::imageops::FilterType::Lanczos3)
                            .to_rgba8();
                        for (x, y, pixel) in rgba.enumerate_pixels_mut() {
                            let distance = ((x as f32 + 0.5 - 72.0).powi(2)
                                + (y as f32 + 0.5 - 72.0).powi(2))
                            .sqrt();
                            pixel[3] = (pixel[3] as f32 * (72.0 - distance).clamp(0.0, 1.0)) as u8;
                        }
                        let texture = ui.ctx().load_texture(
                            "quote-avatar",
                            egui::ColorImage::from_rgba_unmultiplied([144, 144], rgba.as_raw()),
                            egui::TextureOptions::LINEAR,
                        );
                        painter.image(
                            texture.id(),
                            rect,
                            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        held.push(texture);
                    } else {
                        painter.circle_filled(rect.center(), 36.0, accent);
                        let initial = draft
                            .author
                            .trim()
                            .chars()
                            .next()
                            .unwrap_or('?')
                            .to_uppercase()
                            .to_string();
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            initial,
                            egui::FontId::proportional(32.0),
                            background,
                        );
                    }
                }
                let (name, placements) =
                    layout(ui, draft.author.trim(), 30.0, foreground, 980.0 - name_x);
                if name.size().y > 90.0 || name.size().x > 981.0 - name_x {
                    error = Some("İsim karta sığmıyor. Görünen ismi kısalt.".into());
                    return;
                }
                paint_text(
                    ui,
                    &self.emoji,
                    name,
                    &placements,
                    pos2(name_x, author_y),
                    foreground,
                    &mut held,
                );
            },
        );
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                let egui::ImageData::Color(image) = &delta.image;
                if let Some([x, y]) = delta.pos {
                    if let Some(texture) = self.textures.get_mut(id) {
                        for row in 0..image.size[1] {
                            let offset = (y + row) * texture.size[0] + x;
                            texture.pixels[offset..offset + image.size[0]].copy_from_slice(
                                &image.pixels[row * image.size[0]..(row + 1) * image.size[0]],
                            );
                        }
                    }
                } else {
                    self.textures.insert(*id, (**image).clone());
                }
            }
        }
        let mut image = DecodedImage {
            width: 1080,
            height,
            bytes: vec![0; 1080 * height * 4],
        };
        let free = std::mem::take(&mut output.textures_delta.free);
        output.textures_delta.clear();
        if error.is_none() {
            for primitive in self.ctx.tessellate(output.shapes, output.pixels_per_point) {
                if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive {
                    let Some(texture) = self.textures.get(&mesh.texture_id) else {
                        error = Some("Görsel dokusu üretilemedi.".into());
                        break;
                    };
                    raster_mesh(&mut image, &mesh, texture, primitive.clip_rect);
                }
            }
        }
        for id in free {
            self.textures.remove(&id);
        }
        // Handles allocated for this card can now be freed on the next pass.
        drop(held);
        error.map_or(Ok(image), Err)
    }
}

fn layout(
    ui: &egui::Ui,
    text: &str,
    size: f32,
    color: Color32,
    width: f32,
) -> (Arc<egui::Galley>, Vec<String>) {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    let mut placements = Vec::new();
    crate::emoji::append(
        ui,
        &mut job,
        &mut placements,
        text,
        &egui::TextFormat::simple(egui::FontId::proportional(size), color),
    );
    (crate::bidi::layout_job(ui, job), placements)
}

fn paint_text(
    ui: &egui::Ui,
    emoji: &fastframe_emoji::Emoji,
    galley: Arc<egui::Galley>,
    placements: &[String],
    origin: Pos2,
    color: Color32,
    held: &mut Vec<egui::TextureHandle>,
) {
    ui.painter().galley(origin, galley.clone(), color);
    for (rect, cluster) in fastframe_emoji::placeholder_rects(&galley).zip(placements) {
        let rect = rect.translate(origin.to_vec2());
        if let Some(picture) = emoji.render(cluster, rect.height().ceil() as u32) {
            let size = vec2(picture.size[0] as f32, picture.size[1] as f32);
            let scale = (rect.width() / size.x).min(rect.height() / size.y);
            let rect = Rect::from_center_size(rect.center(), size * scale);
            let texture = ui.ctx().load_texture(
                "quote-emoji",
                egui::ColorImage::from_rgba_premultiplied(picture.size, &picture.rgba),
                egui::TextureOptions::LINEAR,
            );
            ui.painter().image(
                texture.id(),
                rect,
                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            held.push(texture);
        }
    }
}

fn edge(a: Pos2, b: Pos2, p: Pos2) -> f32 {
    (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x)
}

fn raster_mesh(
    image: &mut DecodedImage,
    mesh: &egui::epaint::Mesh,
    texture: &egui::ColorImage,
    clip: Rect,
) {
    for triangle in mesh.indices.as_chunks::<3>().0 {
        let [a, mut b, mut c] = [
            mesh.vertices[triangle[0] as usize],
            mesh.vertices[triangle[1] as usize],
            mesh.vertices[triangle[2] as usize],
        ];
        if edge(a.pos, b.pos, c.pos) < 0.0 {
            std::mem::swap(&mut b, &mut c);
        }
        let area = edge(a.pos, b.pos, c.pos);
        if area < 0.0001 {
            continue;
        }
        let bounds = Rect::from_min_max(
            pos2(
                a.pos.x.min(b.pos.x).min(c.pos.x),
                a.pos.y.min(b.pos.y).min(c.pos.y),
            ),
            pos2(
                a.pos.x.max(b.pos.x).max(c.pos.x),
                a.pos.y.max(b.pos.y).max(c.pos.y),
            ),
        )
        .intersect(clip);
        for y in (bounds.min.y.floor().max(0.0) as usize)
            ..(bounds.max.y.ceil().min(image.height as f32) as usize)
        {
            for x in (bounds.min.x.floor().max(0.0) as usize)
                ..(bounds.max.x.ceil().min(image.width as f32) as usize)
            {
                let point = pos2(x as f32 + 0.5, y as f32 + 0.5);
                let weights = [
                    edge(b.pos, c.pos, point),
                    edge(c.pos, a.pos, point),
                    edge(a.pos, b.pos, point),
                ];
                let edges = [(b.pos, c.pos), (c.pos, a.pos), (a.pos, b.pos)];
                if weights.iter().zip(edges).any(|(w, (start, end))| {
                    *w < 0.0
                        || (*w == 0.0
                            && !(end.y < start.y || (end.y == start.y && end.x > start.x)))
                }) {
                    continue;
                }
                let weights = weights.map(|w| w / area);
                let uv = a.uv.to_vec2() * weights[0]
                    + b.uv.to_vec2() * weights[1]
                    + c.uv.to_vec2() * weights[2];
                let sample = sample(texture, uv);
                let colors = [a.color.to_array(), b.color.to_array(), c.color.to_array()];
                let color: [f32; 4] = std::array::from_fn(|channel| {
                    sample[channel]
                        * (colors[0][channel] as f32 * weights[0]
                            + colors[1][channel] as f32 * weights[1]
                            + colors[2][channel] as f32 * weights[2])
                        / 255.0
                });
                let offset = (y * image.width + x) * 4;
                for (channel, source) in color.iter().enumerate() {
                    image.bytes[offset + channel] =
                        (source + image.bytes[offset + channel] as f32 * (1.0 - color[3] / 255.0))
                            .round()
                            .clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
}

fn sample(texture: &egui::ColorImage, uv: egui::Vec2) -> [f32; 4] {
    let x = (uv.x * texture.size[0] as f32 - 0.5).clamp(0.0, (texture.size[0] - 1) as f32);
    let y = (uv.y * texture.size[1] as f32 - 0.5).clamp(0.0, (texture.size[1] - 1) as f32);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(texture.size[0] - 1);
    let y1 = (y0 + 1).min(texture.size[1] - 1);
    let pixels = [
        texture[(x0, y0)].to_array(),
        texture[(x1, y0)].to_array(),
        texture[(x0, y1)].to_array(),
        texture[(x1, y1)].to_array(),
    ];
    let dx = x - x0 as f32;
    let dy = y - y0 as f32;
    std::array::from_fn(|i| {
        (pixels[0][i] as f32 * (1.0 - dx) + pixels[1][i] as f32 * dx) * (1.0 - dy)
            + (pixels[2][i] as f32 * (1.0 - dx) + pixels[3][i] as f32 * dx) * dy
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> Draft {
        Draft {
            text: "Hayat, plan yaparken başımıza gelen şeydir.\nBiraz cesaret, biraz kahve. ☕✨"
                .into(),
            author: "Deniz Yılmaz".into(),
            avatar: None,
            show_avatar: true,
            dark: true,
            portrait: false,
        }
    }

    #[test]
    fn quote_card_is_opaque_and_contains_text_and_colour_emoji() {
        let image = Renderer::new().render(&draft()).unwrap();
        assert_eq!((image.width, image.height), (1080, 1080));
        assert!(
            image.bytes.as_chunks::<4>().0.iter().all(|p| p[3] == 255),
            "the card must fill the export canvas"
        );
        let center = image
            .bytes
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .filter(|(i, _)| (250..800).contains(&(i / 1080)))
            .collect::<Vec<_>>();
        assert!(
            center
                .iter()
                .filter(|(_, p)| p[0] > 200 && p[1] > 200 && p[2] > 200)
                .count()
                > 1000,
            "text was drawn"
        );
        assert!(
            center
                .iter()
                .any(|(_, p)| p[0].abs_diff(p[1]) > 40 || p[1].abs_diff(p[2]) > 40),
            "colour emoji were drawn"
        );
        if let Ok(directory) = std::env::var("ZAPFAST_QUOTE_QA") {
            std::fs::create_dir_all(&directory).unwrap();
            image::save_buffer_with_format(
                std::path::Path::new(&directory).join("quote-dark.png"),
                &image.bytes,
                1080,
                1080,
                image::ColorType::Rgba8,
                image::ImageFormat::Png,
            )
            .unwrap();
        }
    }

    #[test]
    fn quote_portrait_light_and_missing_avatar_use_the_same_renderer() {
        let mut draft = draft();
        draft.dark = false;
        draft.portrait = true;
        let mut renderer = Renderer::new();
        let image = renderer.render(&draft).unwrap();
        assert_eq!((image.width, image.height), (1080, 1350));
        assert_eq!(&image.bytes[..4], &[246, 243, 235, 255]);
        draft.show_avatar = false;
        assert_ne!(image, renderer.render(&draft).unwrap());
        if let Ok(directory) = std::env::var("ZAPFAST_QUOTE_QA") {
            std::fs::create_dir_all(&directory).unwrap();
            image::save_buffer_with_format(
                std::path::Path::new(&directory).join("quote-light.png"),
                &image.bytes,
                1080,
                1350,
                image::ColorType::Rgba8,
                image::ImageFormat::Png,
            )
            .unwrap();
        }
    }

    #[test]
    fn quote_rejects_empty_and_unreadable_content_instead_of_truncating() {
        let mut draft = draft();
        let mut renderer = Renderer::new();
        draft.text = " \n ".into();
        assert!(renderer.render(&draft).is_err());
        draft.text = "uzun satır\n".repeat(300);
        assert!(renderer.render(&draft).unwrap_err().contains("sığmıyor"));
        draft.text = "x".repeat(6001);
        assert!(renderer.render(&draft).is_err());
        draft.text = "https://example.com/".to_owned() + &"a".repeat(100);
        assert!(renderer.render(&draft).is_ok(), "long unbroken URLs wrap");
    }

    #[test]
    fn quote_avatar_is_cropped_and_clipped_to_a_circle() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("avatar.png");
        image::RgbaImage::from_pixel(90, 60, image::Rgba([220, 20, 40, 255]))
            .save(&path)
            .unwrap();
        let mut draft = draft();
        draft.avatar = Some(path);
        let image = Renderer::new().render(&draft).unwrap();
        let at = |x: usize, y: usize| &image.bytes[(y * 1080 + x) * 4..(y * 1080 + x) * 4 + 4];
        assert_eq!(at(136, 946), &[220, 20, 40, 255]);
        assert_eq!(at(100, 910), &[21, 23, 29, 255]);
    }

    #[test]
    fn quote_editor_discards_old_render_results_after_an_edit() {
        let (requests, _input) = mpsc::channel();
        let (output, results) = mpsc::channel();
        let mut editor = Editor {
            account: AccountId::first(),
            chat: "synthetic".into(),
            draft: draft(),
            original: draft(),
            image: None,
            texture: None,
            error: None,
            notice: None,
            revision: 2,
            requests,
            results,
            saving: None,
        };
        let image = DecodedImage {
            width: 1,
            height: 1,
            bytes: vec![255; 4],
        };
        output.send((1, Ok(image.clone()))).unwrap();
        editor.poll(&egui::Context::default());
        assert!(editor.image.is_none());
        output.send((2, Ok(image))).unwrap();
        editor.poll(&egui::Context::default());
        assert!(editor.image.is_some());
        editor.changed();
        assert!(editor.image.is_none());
        assert!(editor.texture.is_none());
    }
}
