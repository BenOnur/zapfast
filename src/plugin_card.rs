//! Local quote cards. The editor previews these exact pixels, never a second layout.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use egui::{Color32, Pos2, Rect, TextureId, pos2, vec2};

use crate::model::{AccountId, DecodedImage};

#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub scene: Option<Scene>,
    pub text: String,
    pub author: String,
    pub avatar: Option<PathBuf>,
    pub show_avatar: bool,
    pub dark: bool,
    pub portrait: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Export {
    Copy,
    Save,
    Attach,
}

/// Ephemeral editor state, owned by the application shell and cleared on lock/switch.
pub struct Editor {
    pub plugin: String,
    pub account: AccountId,
    pub chat: String,
    pub avatar_id: Option<String>,
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
            plugin: String::new(),
            account,
            chat,
            avatar_id: None,
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

    /// Upgrade only the picture; asynchronous arrival must preserve the edits.
    pub fn upgrade_avatar(&mut self, path: PathBuf) {
        self.original.avatar = Some(path.clone());
        self.draft.avatar = Some(path);
        self.changed();
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
        let mut fonts = fastframe_fonts::FontSetup::default()
            .primary(fastframe_fonts::Primary::Inter)
            .definitions();
        fonts.font_data.insert(
            "quote-rounded".into(),
            Arc::new(egui::FontData::from_static(include_bytes!(
                "../assets/fonts/MPLUSRounded1c-Light.ttf"
            ))),
        );
        fonts
            .families
            .get_mut(&egui::FontFamily::Proportional)
            .unwrap()
            .insert(0, "quote-rounded".into());
        ctx.set_fonts(fonts);
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
        let scene = draft
            .scene
            .as_ref()
            .filter(|scene| scene.valid())
            .ok_or("Eklenti geçerli bir çizim üretmedi.")?;
        let width = scene.width;
        let height = scene.height;
        let background = color(&scene.background).ok_or("Geçersiz renk")?;
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
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    vec2(width as f32, height as f32),
                )),
                ..Default::default()
            },
            |ui| {
                let painter = ui.painter();
                painter.rect_filled(
                    Rect::from_min_size(Pos2::ZERO, vec2(width as f32, height as f32)),
                    0.0,
                    background,
                );
                for shape in &scene.shapes {
                    if shape.kind == "avatar" {
                        let w = shape.width as u32;
                        let h = shape.height as u32;
                        let mut rgba = avatar
                            .as_ref()
                            .map(|avatar| {
                                avatar
                                    .resize_to_fill(w, h, image::imageops::FilterType::Lanczos3)
                                    .to_rgba8()
                            })
                            .unwrap_or_else(|| {
                                image::RgbaImage::from_pixel(w, h, image::Rgba([80, 80, 80, 255]))
                            });
                        if shape.fade_end > shape.fade_start {
                            for (x, _, pixel) in rgba.enumerate_pixels_mut() {
                                let opacity = 1.0
                                    - ((x as f32 - shape.fade_start)
                                        / (shape.fade_end - shape.fade_start))
                                        .clamp(0.0, 1.0);
                                pixel[3] = (pixel[3] as f32 * opacity).round() as u8;
                            }
                        }
                        let texture = ui.ctx().load_texture(
                            "plugin-image",
                            egui::ColorImage::from_rgba_unmultiplied(
                                [w as usize, h as usize],
                                rgba.as_raw(),
                            ),
                            egui::TextureOptions::LINEAR,
                        );
                        painter.image(
                            texture.id(),
                            Rect::from_min_size(
                                pos2(shape.x, shape.y),
                                vec2(shape.width, shape.height),
                            ),
                            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                        held.push(texture);
                    } else if shape.kind == "text" {
                        let foreground = color(&shape.color).unwrap_or(Color32::WHITE);
                        let mut size = shape.size;
                        let (galley, placements, name, name_placements) = loop {
                            let (galley, placements) =
                                layout(ui, &shape.text, size, foreground, shape.width, false);
                            let (name, name_placements) = layout(
                                ui,
                                &shape.subtitle,
                                (size * 0.6).max(18.0),
                                foreground,
                                shape.width,
                                true,
                            );
                            if galley.size().y + shape.gap + name.size().y <= shape.height
                                && galley.size().x <= shape.width + 1.0
                                && name.size().x <= shape.width + 1.0
                            {
                                break (galley, placements, name, name_placements);
                            }
                            size -= 2.0;
                            if size < 16.0 {
                                error = Some("Metin çizim alanına sığmıyor.".into());
                                return;
                            }
                        };
                        let top = shape.y
                            + (shape.height - galley.size().y - shape.gap - name.size().y) / 2.0;
                        paint_text(
                            ui,
                            &self.emoji,
                            galley.clone(),
                            &placements,
                            pos2(shape.x + (shape.width - galley.size().x) / 2.0, top),
                            foreground,
                            &mut held,
                        );
                        paint_text(
                            ui,
                            &self.emoji,
                            name.clone(),
                            &name_placements,
                            pos2(
                                shape.x + (shape.width - name.size().x) / 2.0,
                                top + galley.size().y + shape.gap,
                            ),
                            foreground,
                            &mut held,
                        );
                    }
                }
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
            width,
            height,
            bytes: vec![0; width * height * 4],
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
    italic: bool,
) -> (Arc<egui::Galley>, Vec<String>) {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = width;
    job.halign = egui::Align::Center;
    let mut placements = Vec::new();
    let mut format = egui::TextFormat::simple(egui::FontId::proportional(size), color);
    format.italics = italic;
    format.line_height = Some(size * 1.25);
    crate::emoji::append(ui, &mut job, &mut placements, text, &format);
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
    let origin = origin - galley.rect.min.to_vec2();
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

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct Scene {
    pub width: usize,
    pub height: usize,
    pub background: String,
    pub shapes: Vec<Shape>,
}
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct Shape {
    pub kind: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub fade_start: f32,
    pub fade_end: f32,
    pub text: String,
    pub subtitle: String,
    pub size: f32,
    pub gap: f32,
    pub color: String,
}
impl Scene {
    pub fn valid(&self) -> bool {
        (64..=2048).contains(&self.width)
            && (64..=2048).contains(&self.height)
            && color(&self.background).is_some()
            && self.shapes.len() <= 16
            && self.shapes.iter().all(|s| {
                matches!(s.kind.as_str(), "avatar" | "text")
                    && s.text.chars().count() <= 6000
                    && s.subtitle.chars().count() <= 200
                    && [
                        s.x,
                        s.y,
                        s.width,
                        s.height,
                        s.fade_start,
                        s.fade_end,
                        s.size,
                        s.gap,
                    ]
                    .iter()
                    .all(|v| v.is_finite() && v.abs() <= 4096.0)
                    && s.width >= 1.0
                    && s.width <= 2048.0
                    && s.height >= 1.0
                    && s.height <= 2048.0
                    && (s.kind != "text"
                        || ((16.0..=144.0).contains(&s.size) && color(&s.color).is_some()))
            })
    }
}
fn color(s: &str) -> Option<Color32> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(Color32::from_rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
}
