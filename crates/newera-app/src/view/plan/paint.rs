//! Paints a [`newera_draw::Scene`] with egui, plus grid and rulers.

use std::collections::HashMap;
use std::path::Path;

use eframe::egui::{
    self, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, TextureHandle, Vec2,
};
use newera_core::{LengthUnit, Point2, resolve_asset};
use newera_draw::{Align, Color, Palette, Primitive, Scene, Size};

use super::camera::Camera;

pub(crate) fn color(c: Color) -> Color32 {
    let [r, g, b, a] = c.0;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn size_px(camera: &Camera, size: Size) -> f32 {
    #[allow(clippy::cast_possible_truncation)]
    match size {
        Size::Px(px) => px,
        Size::Cm(cm) => (cm * f64::from(camera.zoom)) as f32,
    }
}

/// Background images uploaded to the GPU, keyed by resolved path, each
/// with the revision of the file it was read from: a file replaced under
/// the same path is read again.
#[derive(Default)]
pub(crate) struct Textures {
    loaded: HashMap<String, (newera_core::vfs::AssetRevision, Option<TextureHandle>)>,
}

impl std::fmt::Debug for Textures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Textures")
            .field("count", &self.loaded.len())
            .finish()
    }
}

impl Textures {
    fn get(
        &mut self,
        ctx: &egui::Context,
        project: Option<&Path>,
        path: &str,
    ) -> Option<&TextureHandle> {
        self.load(ctx, project, path, egui::TextureOptions::LINEAR)
    }

    /// The same image, repeating past its edges (floor finishes).
    fn get_tiled(
        &mut self,
        ctx: &egui::Context,
        project: Option<&Path>,
        path: &str,
    ) -> Option<&TextureHandle> {
        let options = egui::TextureOptions {
            wrap_mode: egui::TextureWrapMode::Repeat,
            ..egui::TextureOptions::LINEAR
        };
        self.load(ctx, project, path, options)
    }

    fn load(
        &mut self,
        ctx: &egui::Context,
        project: Option<&Path>,
        path: &str,
        options: egui::TextureOptions,
    ) -> Option<&TextureHandle> {
        let resolved = resolve_asset(project, path);
        let key = format!("{}#{:?}", resolved.display(), options.wrap_mode);
        let now = newera_core::vfs::revision(&resolved);
        if self.loaded.get(&key).is_some_and(|(seen, _)| *seen != now) {
            self.loaded.remove(&key);
        }
        self.loaded
            .entry(key.clone())
            .or_insert_with(|| {
                let texture = (|| {
                    let image =
                        newera_core::images::decode(&newera_core::vfs::read(&resolved).ok()?)
                            .ok()?
                            .to_rgba8();
                    let size = [image.width() as usize, image.height() as usize];
                    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
                    Some(ctx.load_texture(key, pixels, options))
                })();
                (now, texture)
            })
            .1
            .as_ref()
    }
}

pub(crate) fn paint_scene(
    painter: &Painter,
    rect: Rect,
    camera: &Camera,
    scene: &Scene,
    textures: &mut Textures,
    project: Option<&Path>,
) {
    let to = |p: Point2| camera.to_screen(rect, p);
    for item in &scene.items {
        match &item.primitive {
            Primitive::Fill {
                points,
                triangles,
                color: c,
                texture,
            } => {
                let tiled = texture.as_ref().and_then(|t| {
                    textures
                        .get_tiled(painter.ctx(), project, &t.path)
                        .map(|handle| (t, handle.id()))
                });
                let mut mesh = egui::Mesh::default();
                if let Some((t, id)) = tiled {
                    // Image finishes at their real tile size, turned with the finish.
                    mesh.texture_id = id;
                    let (sin, cos) = t.angle.to_radians().sin_cos();
                    for p in points {
                        #[allow(clippy::cast_possible_truncation)]
                        let uv = Pos2::new(
                            ((p.x * cos + p.y * sin) / t.tile[0]) as f32,
                            ((-p.x * sin + p.y * cos) / t.tile[1]) as f32,
                        );
                        mesh.vertices.push(egui::epaint::Vertex {
                            pos: to(*p),
                            uv,
                            color: Color32::WHITE,
                        });
                    }
                } else {
                    let fill = color(*c);
                    for p in points {
                        mesh.colored_vertex(to(*p), fill);
                    }
                }
                for [a, b, c] in triangles {
                    #[allow(clippy::cast_possible_truncation)]
                    mesh.add_triangle(*a as u32, *b as u32, *c as u32);
                }
                painter.add(Shape::mesh(mesh));
            }
            Primitive::Line {
                points,
                closed,
                color: c,
                width,
            } => {
                let mut screen: Vec<Pos2> = points.iter().map(|p| to(*p)).collect();
                if *closed && let Some(&first) = screen.first() {
                    screen.push(first);
                }
                painter.add(Shape::line(
                    screen,
                    Stroke::new(size_px(camera, *width).max(0.5), color(*c)),
                ));
            }
            Primitive::Text {
                text,
                position,
                size,
                color: c,
                align,
                angle,
                look,
            } => {
                let px = size_px(camera, *size);
                if px >= 4.0 {
                    let anchor = to(*position);
                    if let Some(halo) = look.outline {
                        let r = (px * 0.08).max(1.0);
                        for k in 0u8..8 {
                            let t = f32::from(k) / 8.0 * std::f32::consts::TAU;
                            let offset = Vec2::new(r * t.cos(), r * t.sin());
                            paint_text(
                                painter,
                                anchor + offset,
                                text,
                                px,
                                color(halo),
                                *align,
                                *angle,
                            );
                        }
                    }
                    paint_text(painter, anchor, text, px, color(*c), *align, *angle);
                    if look.bold {
                        let shift = Vec2::new((px * 0.045).max(0.6), 0.0);
                        paint_text(painter, anchor + shift, text, px, color(*c), *align, *angle);
                    }
                }
            }
            Primitive::Image {
                path,
                min,
                max,
                opacity,
                angle,
            } => {
                if let Some(texture) = textures.get(painter.ctx(), project, path) {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                    let tint = Color32::from_white_alpha((opacity * 255.0) as u8);
                    if angle.abs() < 1e-6 {
                        painter.image(
                            texture.id(),
                            Rect::from_two_pos(to(*min), to(*max)),
                            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                            tint,
                        );
                    } else {
                        let center = Point2::new(min.x.midpoint(max.x), min.y.midpoint(max.y));
                        let (sin, cos) = angle.to_radians().sin_cos();
                        let corner = |x: f64, y: f64| {
                            let (dx, dy) = (x - center.x, y - center.y);
                            to(Point2::new(
                                center.x + dx * cos - dy * sin,
                                center.y + dx * sin + dy * cos,
                            ))
                        };
                        let mut mesh = egui::Mesh::with_texture(texture.id());
                        let uv = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
                        let pos = [
                            corner(min.x, min.y),
                            corner(max.x, min.y),
                            corner(max.x, max.y),
                            corner(min.x, max.y),
                        ];
                        for (p, (u, v)) in pos.iter().zip(uv) {
                            mesh.vertices.push(egui::epaint::Vertex {
                                pos: *p,
                                uv: Pos2::new(u, v),
                                color: tint,
                            });
                        }
                        mesh.indices.extend([0, 1, 2, 0, 2, 3]);
                        painter.add(egui::Shape::mesh(mesh));
                    }
                }
            }
        }
    }
}

pub(crate) fn paint_text(
    painter: &Painter,
    anchor: Pos2,
    text: &str,
    size_px: f32,
    color: Color32,
    align: Align,
    angle_deg: f64,
) {
    let mut job = egui::text::LayoutJob::simple(
        text.to_owned(),
        FontId::proportional(size_px),
        color,
        f32::INFINITY,
    );
    job.halign = match align {
        Align::BaselineLeft => egui::Align::LEFT,
        Align::BaselineRight => egui::Align::RIGHT,
        _ => egui::Align::Center,
    };
    let galley = painter.layout_job(job);
    let rect = galley.rect;
    let local = match align {
        Align::Center => Vec2::new(0.0, -rect.height() / 2.0),
        Align::Above => Vec2::new(0.0, -rect.height() - 1.0),
        // Descent is roughly a fifth of the font size.
        Align::BaselineLeft | Align::BaselineCenter | Align::BaselineRight => {
            Vec2::new(0.0, -rect.height() + size_px * 0.22)
        }
    };
    #[allow(clippy::cast_possible_truncation)]
    let angle = angle_deg.to_radians() as f32;
    let rot = egui::emath::Rot2::from_angle(angle);
    let pos = anchor + rot * local;
    painter.add(egui::epaint::TextShape::new(pos, galley, color).with_angle(angle));
}

pub(crate) fn paint_grid(painter: &Painter, rect: Rect, camera: &Camera, palette: &Palette) {
    let min = camera.to_world(rect, rect.left_top());
    let max = camera.to_world(rect, rect.right_bottom());
    let step = if f64::from(camera.zoom) * 50.0 < 8.0 {
        500.0
    } else {
        50.0
    };
    #[allow(clippy::cast_possible_truncation)]
    let range = |lo: f64, hi: f64| (lo / step).floor() as i64..=(hi / step).ceil() as i64;
    let stroke = |i: i64| {
        Stroke::new(
            1.0,
            color(if i % 2 == 0 {
                palette.grid_major
            } else {
                palette.grid_minor
            }),
        )
    };
    for i in range(min.x, max.x) {
        #[allow(clippy::cast_precision_loss)]
        let x = camera.to_screen(rect, Point2::new(i as f64 * step, 0.0)).x;
        painter.vline(x, rect.y_range(), stroke(i));
    }
    for i in range(min.y, max.y) {
        #[allow(clippy::cast_precision_loss)]
        let y = camera.to_screen(rect, Point2::new(0.0, i as f64 * step)).y;
        painter.hline(rect.x_range(), y, stroke(i));
    }
}

pub(crate) const RULER: f32 = 18.0;

/// Top and left rulers with labeled ticks in the display unit. They are
/// chrome, not drawing: they wear the colors of the window, so the sheet
/// underneath is the only thing that looks like paper.
pub(crate) fn paint_rulers(
    painter: &Painter,
    rect: Rect,
    camera: &Camera,
    unit: LengthUnit,
    cursor: Option<Point2>,
    t: crate::theme::Tokens,
) {
    let bg = t.surface.gamma_multiply(0.97);
    let ink = t.ink_faint;
    let top = Rect::from_min_max(rect.left_top(), Pos2::new(rect.right(), rect.top() + RULER));
    let left = Rect::from_min_max(
        rect.left_top(),
        Pos2::new(rect.left() + RULER, rect.bottom()),
    );
    painter.rect_filled(top, 0.0, bg);
    painter.rect_filled(left, 0.0, bg);
    let edge = Stroke::new(1.0, t.rule);
    painter.hline(top.x_range(), top.bottom(), edge);
    painter.vline(left.right(), left.y_range(), edge);

    // Major tick spacing: the smallest "nice" step that is at least 60 px apart.
    let major = [
        1.0, 5.0, 10.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10_000.0,
    ]
    .into_iter()
    .find(|s| f64::from(camera.zoom) * s >= 60.0)
    .unwrap_or(50_000.0);
    let minor = major / 5.0;
    let min = camera.to_world(rect, rect.left_top());
    let max = camera.to_world(rect, rect.right_bottom());
    let font = FontId::proportional(10.0);

    #[allow(clippy::cast_possible_truncation)]
    for i in (min.x / minor).floor() as i64..=(max.x / minor).ceil() as i64 {
        #[allow(clippy::cast_precision_loss)]
        let v = i as f64 * minor;
        let x = camera.to_screen(rect, Point2::new(v, 0.0)).x;
        if x < rect.left() + RULER {
            continue;
        }
        let is_major = i % 5 == 0;
        let len = if is_major { RULER * 0.6 } else { RULER * 0.25 };
        painter.vline(
            x,
            (top.bottom() - len)..=top.bottom(),
            Stroke::new(1.0, ink),
        );
        if is_major {
            painter.text(
                Pos2::new(x + 2.0, top.top() + 1.0),
                egui::Align2::LEFT_TOP,
                unit.format_length(v),
                font.clone(),
                ink,
            );
        }
    }
    #[allow(clippy::cast_possible_truncation)]
    for i in (min.y / minor).floor() as i64..=(max.y / minor).ceil() as i64 {
        #[allow(clippy::cast_precision_loss)]
        let v = i as f64 * minor;
        let y = camera.to_screen(rect, Point2::new(0.0, v)).y;
        if y < rect.top() + RULER {
            continue;
        }
        let is_major = i % 5 == 0;
        let len = if is_major { RULER * 0.6 } else { RULER * 0.25 };
        painter.hline(
            (left.right() - len)..=left.right(),
            y,
            Stroke::new(1.0, ink),
        );
        if is_major {
            paint_text(
                painter,
                Pos2::new(left.left() + 7.0, y + 3.0),
                &unit.format_length(v),
                10.0,
                ink,
                Align::Center,
                -90.0,
            );
        }
    }
    if let Some(c) = cursor {
        let marker = t.accent;
        let s = camera.to_screen(rect, c);
        painter.vline(s.x, top.y_range(), Stroke::new(1.0, marker));
        painter.hline(left.x_range(), s.y, Stroke::new(1.0, marker));
    }
    // The corner square covers where the two rulers cross.
    let corner = Rect::from_min_size(rect.left_top(), Vec2::splat(RULER));
    painter.rect_filled(corner, 0.0, bg);
    painter.hline(corner.x_range(), corner.bottom(), edge);
    painter.vline(corner.right(), corner.y_range(), edge);
    painter.text(
        corner.center(),
        egui::Align2::CENTER_CENTER,
        unit.label(),
        FontId::monospace(8.0),
        ink,
    );
}

#[cfg(test)]
mod tests {
    /// A tiled floor image replaced under the same path is read again; the
    /// same file is not.
    #[test]
    fn a_texture_replaced_under_its_path_is_read_again() {
        let png = |rgb: [u8; 3]| {
            let mut out = std::io::Cursor::new(Vec::new());
            image::RgbaImage::from_pixel(2, 2, image::Rgba([rgb[0], rgb[1], rgb[2], 255]))
                .write_to(&mut out, image::ImageFormat::Png)
                .unwrap();
            out.into_inner()
        };
        let dir = std::path::PathBuf::from("/memory/plan-textures-test");
        let path = "products/p1/finish.jpg";
        let ctx = eframe::egui::Context::default();
        let mut textures = super::Textures::default();
        newera_core::vfs::mount(&dir, [(path.to_owned(), png([200, 30, 30]))]);
        let first = textures.get_tiled(&ctx, Some(&dir), path).unwrap().id();
        assert_eq!(
            textures.get_tiled(&ctx, Some(&dir), path).unwrap().id(),
            first
        );
        newera_core::vfs::mount(&dir, [(path.to_owned(), png([30, 30, 200]))]);
        assert_ne!(
            textures.get_tiled(&ctx, Some(&dir), path).unwrap().id(),
            first
        );
        newera_core::vfs::unmount(&dir);
    }
}
