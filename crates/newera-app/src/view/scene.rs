//! Native 3D viewport: renders the home into an offscreen wgpu texture that
//! egui then shows as an image. Owning the render pass keeps depth testing
//! and MSAA independent from egui's own pass.

use bytemuck::{Pod, Zeroable};
use eframe::egui;
use eframe::egui_wgpu::RenderState;
use eframe::wgpu;
use glam::{Mat4, Vec3};
use newera_core::{ElementId, Home};

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use super::axes::Basis;
use super::plan::Selection;

use newera_render::{Cutaway, Mesh, Side, Vertex};

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const SAMPLES: u32 = 4;
const SKY: wgpu::Color = wgpu::Color {
    r: 0.62,
    g: 0.76,
    b: 0.90,
    a: 1.0,
};

/// Orbit camera around a target point, in meters.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OrbitCamera {
    target: Vec3,
    yaw: f32,
    pitch: f32,
    distance: f32,
    /// `(yaw, pitch)` the camera is easing towards, after a side was picked
    /// on the axes.
    goal: Option<(f32, f32)>,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            yaw: -0.8,
            pitch: 0.6,
            distance: 18.0,
            goal: None,
        }
    }
}

impl OrbitCamera {
    fn eye(&self) -> Vec3 {
        let dir = Vec3::new(
            self.yaw.cos() * self.pitch.cos(),
            self.pitch.sin(),
            self.yaw.sin() * self.pitch.cos(),
        );
        self.target + dir * self.distance
    }

    /// Right, up and back towards the eye. Right comes from the yaw alone, so
    /// up stays defined when the camera looks straight down.
    pub(crate) fn basis(&self) -> Basis {
        let back = (self.eye() - self.target).normalize();
        let right = Vec3::new(self.yaw.sin(), 0.0, -self.yaw.cos());
        Basis {
            right,
            up: right.cross(-back),
            back,
        }
    }

    fn view_proj(&self, aspect: f32) -> Mat4 {
        let view = glam::camera::rh::view::look_at_mat4(self.eye(), self.target, self.basis().up);
        // wgpu uses a 0..1 depth range, same as DirectX.
        let proj =
            glam::camera::rh::proj::directx::perspective(45f32.to_radians(), aspect, 0.05, 500.0);
        proj * view
    }

    fn orbit(&mut self, delta: egui::Vec2) {
        self.goal = None;
        self.yaw += delta.x * 0.008;
        self.pitch = (self.pitch + delta.y * 0.008).clamp(0.0, FRAC_PI_2);
    }

    /// Starts turning to look at the target from `side`, the short way
    /// round. The top keeps the plan's own reading: x right, y down.
    fn look_from(&mut self, side: Side) {
        let (yaw, pitch) = match side {
            Side::Right => (0.0, 0.0),
            Side::Left => (PI, 0.0),
            Side::Front => (FRAC_PI_2, 0.0),
            Side::Back => (-FRAC_PI_2, 0.0),
            Side::Top => (FRAC_PI_2, FRAC_PI_2),
        };
        let turn = (yaw - self.yaw + PI).rem_euclid(TAU) - PI;
        self.goal = Some((self.yaw + turn, pitch));
    }

    /// Moves towards the picked side for `seconds`; true while still on the
    /// way.
    fn settle(&mut self, seconds: f32) -> bool {
        let Some((yaw, pitch)) = self.goal else {
            return false;
        };
        let step = 1.0 - (-seconds * 12.0).exp();
        self.yaw += (yaw - self.yaw) * step;
        self.pitch += (pitch - self.pitch) * step;
        if (yaw - self.yaw).abs() < 1e-3 && (pitch - self.pitch).abs() < 1e-3 {
            (self.yaw, self.pitch) = (yaw, pitch);
            self.goal = None;
        }
        true
    }

    fn zoom(&mut self, scroll: f32) {
        self.closer((-scroll * 0.002).exp());
    }

    /// Straight to a factor, the way a pinch reports it: above 1.0 the
    /// fingers spread and the house comes closer.
    fn closer(&mut self, factor: f32) {
        self.distance = (self.distance / factor).clamp(1.0, 200.0);
    }

    /// Turns around the house without changing the height of the eye.
    fn turn(&mut self, radians: f32) {
        self.goal = None;
        self.yaw += radians;
    }

    /// Moves the target on the ground plane, relative to the view direction.
    fn pan(&mut self, delta: egui::Vec2) {
        let forward = Vec3::new(self.yaw.cos(), 0.0, self.yaw.sin());
        let right = forward.cross(Vec3::Y);
        let speed = self.distance * 0.0015;
        self.target += (right * -delta.x + forward * -delta.y) * speed;
    }

    /// The way the camera looks, flat on the plan.
    fn toward(&self) -> newera_core::Point2 {
        newera_core::Point2::new(-f64::from(self.yaw.cos()), -f64::from(self.yaw.sin()))
    }

    fn look_at_home(&mut self, home: &Home) {
        if let Some((min, max)) = home.building_bounds() {
            #[allow(clippy::cast_possible_truncation)]
            let center = Vec3::new(
                ((min.x + max.x) / 200.0) as f32,
                1.0,
                ((min.y + max.y) / 200.0) as f32,
            );
            #[allow(clippy::cast_possible_truncation)]
            let size = (min.distance(max) / 100.0) as f32;
            self.target = center;
            self.distance = (size * 1.4).clamp(4.0, 150.0);
        }
    }
}

/// How tall the walls stand in the aerial view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Walls {
    #[default]
    Up,
    /// The walls between the camera and the rooms come down.
    Cutaway,
    Down,
}

impl Walls {
    pub(crate) const ALL: [Self; 3] = [Self::Up, Self::Cutaway, Self::Down];

    pub(crate) fn label(self) -> &'static str {
        crate::i18n::tr(match self {
            Self::Up => "Inteiras",
            Self::Cutaway => "Recortadas",
            Self::Down => "Baixas",
        })
    }

    /// A room seen from above at an angle, its walls as tall as this mode
    /// leaves them: the back pair and the front pair, drawn inside `rect` in
    /// `color` over `behind`, so the near walls hide the far ones.
    pub(crate) fn paint_icon(
        self,
        painter: &egui::Painter,
        rect: egui::Rect,
        color: egui::Color32,
        behind: egui::Color32,
    ) {
        let (back, front) = match self {
            Self::Up => (9.0, 9.0),
            Self::Cutaway => (9.0, 2.5),
            Self::Down => (2.5, 2.5),
        };
        let c = rect.center() + egui::vec2(0.0, 4.5);
        let (w, h) = (8.5, 4.5);
        let (top, right, bottom, left) = (
            c + egui::vec2(0.0, -h),
            c + egui::vec2(w, 0.0),
            c + egui::vec2(0.0, h),
            c + egui::vec2(-w, 0.0),
        );
        let stroke = egui::Stroke::new(1.2, color);
        let shade = |t: f32| behind.lerp_to_gamma(color, t);
        let wall = |a: egui::Pos2, b: egui::Pos2, tall: f32, fill: egui::Color32| {
            let up = egui::vec2(0.0, -tall);
            painter.add(egui::Shape::convex_polygon(
                vec![a, b, b + up, a + up],
                fill,
                stroke,
            ));
        };
        wall(left, top, back, shade(0.35));
        wall(top, right, back, shade(0.2));
        painter.add(egui::Shape::convex_polygon(
            vec![top, right, bottom, left],
            shade(0.1),
            stroke,
        ));
        wall(left, bottom, front, shade(0.5));
        wall(bottom, right, front, shade(0.3));
    }

    pub(crate) fn hint(self) -> &'static str {
        crate::i18n::tr(match self {
            Self::Up => "Todas as paredes em pé",
            Self::Cutaway => "As paredes da frente descem para mostrar os cômodos",
            Self::Down => "Todas as paredes descem",
        })
    }
}

/// Eye-level camera walking through the home (a stored point of view).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Visitor {
    pub(crate) camera: newera_core::Camera,
}

impl Visitor {
    #[allow(clippy::cast_possible_truncation)]
    fn eye(&self) -> Vec3 {
        let c = &self.camera;
        Vec3::new(c.x as f32, c.z as f32, c.y as f32) * 0.01
    }

    #[allow(clippy::cast_possible_truncation)]
    fn direction(&self) -> Vec3 {
        let (dx, dy) = self.camera.direction();
        let pitch = self.camera.pitch.to_radians() as f32;
        Vec3::new(
            dx as f32 * pitch.cos(),
            -pitch.sin(),
            dy as f32 * pitch.cos(),
        )
    }

    #[allow(clippy::cast_possible_truncation)]
    fn view_proj(&self, aspect: f32) -> Mat4 {
        let eye = self.eye();
        let view = glam::camera::rh::view::look_at_mat4(eye, eye + self.direction(), Vec3::Y);
        // Stored fields of view are horizontal.
        let horizontal = (self.camera.fov.to_radians() as f32).clamp(0.1, 3.0);
        let vertical = 2.0 * ((horizontal / 2.0).tan() / aspect).atan();
        let proj = glam::camera::rh::proj::directx::perspective(vertical, aspect, 0.02, 500.0);
        proj * view
    }

    fn look(&mut self, delta: egui::Vec2) {
        self.camera.yaw += f64::from(delta.x) * 0.3;
        self.camera.pitch = (self.camera.pitch + f64::from(delta.y) * 0.3).clamp(-85.0, 85.0);
    }

    fn walk(&mut self, forward: f64, side: f64) {
        let (dx, dy) = self.camera.direction();
        self.camera.x += dx * forward - dy * side;
        self.camera.y += dy * forward + dx * side;
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    light_dir: [f32; 3],
    /// Sunlight strength 0–1 when the sun follows the compass; negative for
    /// the default soft studio light.
    sun: f32,
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    index_count: u32,
    /// Transparent triangles follow the opaque ones in `indices`.
    transparent_count: u32,
}

struct Targets {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolve: wgpu::TextureView,
}

/// Side of every image texture layer, in pixels.
const IMAGE_SIZE: u32 = 512;
const IMAGE_MIPS: u32 = 10;

/// GPU resources for the viewport. Created lazily on the first frame.
struct Gpu {
    pipeline: wgpu::RenderPipeline,
    transparent_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    bind_group: wgpu::BindGroup,
    /// Image files currently uploaded, in layer order, each as it was then.
    images: Vec<(String, newera_core::vfs::AssetRevision)>,
    mesh: Option<GpuMesh>,
    targets: Option<Targets>,
    texture_id: Option<egui::TextureId>,
}

/// Direction towards the sun and its strength (0 at night) at `hour` local
/// solar time, on the day of the home's camera, from the compass location.
pub(crate) fn sun_light(home: &Home, hour: f64) -> (Vec3, f32) {
    let base = match home.cameras.top.time {
        0 => 1_789_214_400_000,
        time => time,
    };
    let time = newera_render::at_local_hour(base, hour, home.compass.longitude.unwrap_or(-46.63));
    match newera_render::sun_direction(&home.compass, time) {
        #[allow(clippy::cast_possible_truncation)]
        Some((dir, elevation)) => (dir, (elevation / 25.0).clamp(0.0, 1.0) as f32),
        None => (Vec3::Y, 0.0),
    }
}

/// Revision, selection, cutaway, which pieces with a lighter far file were
/// drawn with it, and the revisions of the files the home draws with.
type BuiltFor = (u64, Vec<ElementId>, Option<Cutaway>, Vec<bool>, u64);

/// What the files `home` draws with are now: a file replaced under the
/// same path — a shop product's model or texture swapped in by the hosted
/// service — changes it, and the scene is built again.
/// Each distinct file is looked at once, and no path is copied: this runs
/// every frame.
fn files_seen(home: &newera_core::Home, project: Option<&std::path::Path>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut files = std::collections::BTreeSet::new();
    home.for_each_asset(&mut |file| {
        files.insert(file);
    });
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for file in files {
        newera_core::vfs::revision(&newera_core::resolve_asset(project, file)).hash(&mut hash);
    }
    hash.finish()
}

/// What the GPU mesh is built from now: when it differs from what it was
/// built from, the scene is built again.
fn built_for(
    revision: u64,
    selected: Vec<ElementId>,
    cutaway: Option<Cutaway>,
    far: Vec<bool>,
    home: &newera_core::Home,
    project: Option<&std::path::Path>,
) -> BuiltFor {
    (revision, selected, cutaway, far, files_seen(home, project))
}

pub(crate) struct SceneView {
    camera: OrbitCamera,
    /// When set, the view looks through this visitor instead of orbiting.
    pub(crate) visitor: Option<Visitor>,
    /// Local solar hour of the live sun; `None` keeps the soft studio light.
    pub(crate) sun_hour: Option<f64>,
    /// Walls up, cut away or down; a visitor always sees them up.
    pub(crate) walls: Walls,
    gpu: Option<Gpu>,
    /// What the GPU mesh was built from.
    built_for: Option<BuiltFor>,
    framed_once: bool,
    /// Imported models, reloaded when a file or its companions change.
    models: newera_render::ModelCache,
}

impl std::fmt::Debug for SceneView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceneView")
            .field("camera", &self.camera)
            .field("built_for", &self.built_for)
            .finish_non_exhaustive()
    }
}

impl SceneView {
    pub(crate) fn new() -> Self {
        Self {
            camera: OrbitCamera::default(),
            visitor: None,
            gpu: None,
            sun_hour: None,
            walls: Walls::Up,
            built_for: None,
            framed_once: false,
            models: newera_render::ModelCache::default(),
        }
    }

    /// Frames the whole home on the next frame.
    /// The current point of view, for photos and exports.
    pub(crate) fn current_view(&self) -> newera_render::View {
        match &self.visitor {
            Some(visitor) => newera_render::View::from_camera(&visitor.camera, 4.0 / 3.0),
            None => newera_render::View {
                eye: self.camera.eye(),
                target: self.camera.target,
                fov_y: 45f32.to_radians(),
                ortho: None,
                near: None,
            },
        }
    }

    /// `(direction the light travels, sun strength)` for the shader.
    fn light(&self, home: &Home) -> (Vec3, f32) {
        let Some(hour) = self.sun_hour else {
            return (Vec3::new(-0.4, -1.0, -0.3).normalize(), -1.0);
        };
        let (dir, strength) = sun_light(home, hour);
        (-dir, strength)
    }

    pub(crate) fn request_frame(&mut self) {
        self.framed_once = false;
    }

    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        render_state: Option<&RenderState>,
        home: &Home,
        revision: u64,
        selection: &Selection,
        project: Option<&std::path::Path>,
    ) {
        let Some(rs) = render_state else {
            ui.centered_and_justified(|ui| {
                ui.label(crate::i18n::tr("Visualização 3D requer o backend wgpu."))
            });
            return;
        };

        let rect = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        // The axes only make sense from above; a visitor walks at eye level.
        let axes = self.visitor.is_none().then(|| {
            let outcome = super::axes::interact(ui, rect, self.camera.basis());
            if let Some(side) = outcome.look_from {
                self.camera.look_from(side);
            }
            if outcome.drag != egui::Vec2::ZERO {
                self.camera.orbit(outcome.drag);
            }
            outcome
        });
        self.handle_input(ui, &response, home);
        if self.camera.settle(ui.input(|i| i.stable_dt).min(0.1)) {
            ui.ctx().request_repaint();
        }

        let ppp = ui.ctx().pixels_per_point();
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let size = [
            ((rect.width() * ppp).round() as u32).max(1),
            ((rect.height() * ppp).round() as u32).max(1),
        ];

        if !self.framed_once && home.bounds().is_some() {
            self.camera.look_at_home(home);
            self.framed_once = true;
        }

        let light = self.light(home);
        let gpu = self.gpu.get_or_insert_with(|| Gpu::new(&rs.device));
        let cutaway = match self.walls {
            _ if self.visitor.is_some() => None,
            Walls::Up => None,
            Walls::Cutaway => Some(Cutaway::facing(home, self.camera.toward())),
            Walls::Down => Some(Cutaway::all(home)),
        };
        // Pieces with a lighter file for afar switch as the camera crosses
        // their distance, which rebuilds the scene like an edit would.
        let eye = self
            .visitor
            .as_ref()
            .map_or_else(|| self.camera.eye(), Visitor::eye);
        let distance =
            |piece: &newera_core::Furniture| newera_render::camera_distance(home, piece, eye);
        let far: Vec<bool> = home
            .furniture
            .iter()
            .flat_map(newera_core::Furniture::flatten)
            .filter_map(|piece| {
                let lod = piece.model_far.as_ref()?;
                Some(lod.file_at(Some(distance(piece))).is_some())
            })
            .collect();
        let key = built_for(
            revision,
            selection.iter().copied().collect::<Vec<_>>(),
            cutaway,
            far,
            home,
            project,
        );
        if self.built_for.as_ref() != Some(&key) {
            // The same cache as photos: a file replaced under the same path
            // (edit_model replace or lod) is read again, not kept.
            let models = |piece: &newera_core::Furniture| {
                self.models
                    .piece_model_seen(piece, project, Some(distance(piece)))
            };
            let mesh = Mesh::from_home_cut(home, selection, &models, key.2.as_ref());
            gpu.upload_images(rs, &mesh.images, project);
            gpu.upload_mesh(rs, &mesh);
            self.built_for = Some(key);
        }
        #[allow(clippy::cast_precision_loss)]
        let aspect = size[0] as f32 / size[1] as f32;
        let view_proj = match &self.visitor {
            Some(visitor) => visitor.view_proj(aspect),
            None => self.camera.view_proj(aspect),
        };
        gpu.render(rs, size, view_proj, light);

        if let Some(texture_id) = gpu.texture_id {
            ui.painter().image(
                texture_id,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        ui.painter().text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            if self.visitor.is_some() {
                crate::i18n::tr("Visitante — arraste: olhar · W/A/S/D ou setas: andar · Scroll: avançar · Esc: visão aérea")
            } else {
                crate::i18n::tr("Arraste ou dois dedos: girar · Shift: mover · Pinça ou scroll: zoom · F: enquadrar")
            },
            egui::FontId::proportional(11.0),
            egui::Color32::from_black_alpha(160),
        );
        if let Some(outcome) = axes {
            super::axes::paint(ui, rect, self.camera.basis(), outcome);
        }
    }

    fn handle_input(&mut self, ui: &egui::Ui, response: &egui::Response, home: &Home) {
        if let Some(visitor) = &mut self.visitor {
            if response.dragged_by(egui::PointerButton::Primary)
                || response.dragged_by(egui::PointerButton::Secondary)
            {
                visitor.look(response.drag_delta());
            }
            if response.hovered() {
                // Walking with the hands: two fingers up and down walk, left
                // and right step aside, and the keys do what they always did.
                let hands = crate::view::gesture::Gesture::read(ui);
                let (forward, side) = ui.input(|i| {
                    let key = |k| if i.key_down(k) { 1.0 } else { 0.0 };
                    (
                        f64::from(hands.glide.y + hands.wheel) * 0.5
                            + (key(egui::Key::W) + key(egui::Key::ArrowUp)
                                - key(egui::Key::S)
                                - key(egui::Key::ArrowDown))
                                * 4.0,
                        f64::from(-hands.glide.x) * 0.5
                            + (key(egui::Key::D) + key(egui::Key::ArrowRight)
                                - key(egui::Key::A)
                                - key(egui::Key::ArrowLeft))
                                * 4.0,
                    )
                });
                if forward != 0.0 || side != 0.0 {
                    visitor.walk(forward, side);
                    ui.ctx().request_repaint();
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.visitor = None;
                }
            }
            return;
        }
        let panning = response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary)
                && ui.input(|i| i.modifiers.shift));
        if panning {
            self.camera.pan(response.drag_delta());
        } else if response.dragged_by(egui::PointerButton::Primary)
            || response.dragged_by(egui::PointerButton::Secondary)
        {
            self.camera.orbit(response.drag_delta());
        }
        if response.hovered() {
            // Two fingers turn the house around, as they would a model held
            // in the hands; with Shift they slide it; the pinch comes closer
            // and a twist spins it on the spot.
            let hands = crate::view::gesture::Gesture::read(ui);
            if !hands.is_idle() {
                if hands.wheel != 0.0 {
                    self.camera.zoom(hands.wheel);
                }
                if let Some(pinch) = hands.pinch() {
                    self.camera.closer(pinch);
                }
                if hands.twist != 0.0 {
                    self.camera.turn(-hands.twist);
                }
                if hands.glide != egui::Vec2::ZERO {
                    if ui.input(|i| i.modifiers.shift) {
                        self.camera.pan(hands.glide);
                    } else {
                        self.camera.orbit(hands.glide * 0.6);
                    }
                }
            }
            if ui.input(|i| i.key_pressed(egui::Key::F)) {
                self.camera.look_at_home(home);
            }
        }
    }
}

impl Gpu {
    fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("scene shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("scene uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("scene bind group layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("scene image sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let bind_group =
            Self::bind_images(device, &bind_group_layout, &uniforms, &sampler, None, &[]);

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene pipeline layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            ..Default::default()
        });

        let make_pipeline = |label: &str, transparent: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        step_mode: wgpu::VertexStepMode::Vertex,
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        attributes: &wgpu::vertex_attr_array![
                            0 => Float32x3,
                            1 => Float32x3,
                            2 => Float32x4,
                            3 => Float32x2,
                            4 => Uint32,
                        ],
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOR_FORMAT,
                        blend: transparent.then_some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    // Glass panes are often single-sided; show both faces.
                    cull_mode: (!transparent).then_some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(!transparent),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = make_pipeline("scene pipeline", false);
        let transparent_pipeline = make_pipeline("scene transparent pipeline", true);

        Self {
            pipeline,
            transparent_pipeline,
            uniforms,
            layout: bind_group_layout,
            sampler,
            bind_group,
            images: Vec::new(),
            mesh: None,
            targets: None,
            texture_id: None,
        }
    }

    /// Builds the bind group with a texture array holding `images` (decoded
    /// RGBA, one per layer); an empty list binds a single white layer.
    fn bind_images(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        uniforms: &wgpu::Buffer,
        sampler: &wgpu::Sampler,
        queue: Option<&wgpu::Queue>,
        images: &[image::RgbaImage],
    ) -> wgpu::BindGroup {
        // GL backends (WebGL) guess the view from the layer count: one layer
        // means plain 2D and multiples of six mean cube maps, and sampling the
        // array then reads black. A spare layer avoids both.
        let mut layers = u32::try_from(images.len().max(2)).expect("layer count fits in u32");
        if layers % 6 == 0 {
            layers += 1;
        }
        let (size, mips) = if images.is_empty() {
            (1, 1)
        } else {
            (IMAGE_SIZE, IMAGE_MIPS)
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("scene images"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers,
            },
            mip_level_count: mips,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        if let Some(queue) = queue {
            for (layer, image) in images.iter().enumerate() {
                let mut level_image = image.clone();
                for mip in 0..mips {
                    let side = (size >> mip).max(1);
                    if mip > 0 {
                        level_image = image::imageops::resize(
                            image,
                            side,
                            side,
                            image::imageops::FilterType::Triangle,
                        );
                    }
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &texture,
                            mip_level: mip,
                            origin: wgpu::Origin3d {
                                x: 0,
                                y: 0,
                                z: u32::try_from(layer).expect("layer fits in u32"),
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        level_image.as_raw(),
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(4 * side),
                            rows_per_image: Some(side),
                        },
                        wgpu::Extent3d {
                            width: side,
                            height: side,
                            depth_or_array_layers: 1,
                        },
                    );
                }
            }
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene bind group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    /// Uploads material images when the set of files, or one of them,
    /// changed. Unreadable files become a neutral gray layer so the rest
    /// still renders.
    fn upload_images(
        &mut self,
        rs: &RenderState,
        images: &[String],
        project: Option<&std::path::Path>,
    ) {
        let seen: Vec<(String, newera_core::vfs::AssetRevision)> = images
            .iter()
            .map(|file| {
                let path = newera_core::resolve_asset(project, file);
                (file.clone(), newera_core::vfs::revision(&path))
            })
            .collect();
        if self.images == seen {
            return;
        }
        let decoded: Vec<image::RgbaImage> = images
            .iter()
            .map(|file| {
                let path = newera_core::resolve_asset(project, file);
                match newera_core::vfs::read(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|b| newera_core::images::decode(&b).map_err(|e| e.to_string()))
                {
                    Ok(img) => image::imageops::resize(
                        &img.to_rgba8(),
                        IMAGE_SIZE,
                        IMAGE_SIZE,
                        image::imageops::FilterType::Triangle,
                    ),
                    Err(err) => {
                        tracing::warn!("cannot load texture {}: {err}", path.display());
                        #[cfg(target_arch = "wasm32")]
                        web_sys::console::warn_1(
                            &format!("cannot load texture {}: {err}", path.display()).into(),
                        );
                        image::RgbaImage::from_pixel(
                            IMAGE_SIZE,
                            IMAGE_SIZE,
                            image::Rgba([180, 180, 180, 255]),
                        )
                    }
                }
            })
            .collect();
        self.bind_group = Self::bind_images(
            &rs.device,
            &self.layout,
            &self.uniforms,
            &self.sampler,
            Some(&rs.queue),
            &decoded,
        );
        self.images = seen;
    }

    fn upload_mesh(&mut self, rs: &RenderState, mesh: &Mesh) {
        // WebGPU may reject mapped-at-creation buffers even below the device's
        // max_buffer_size. Upload through the queue instead: no JS mapping of
        // the whole scene and no second copy concatenating opaque/glass indices.
        let vertex_bytes: &[u8] = bytemuck::cast_slice(&mesh.vertices);
        let opaque_bytes: &[u8] = bytemuck::cast_slice(&mesh.indices);
        let glass_bytes: &[u8] = bytemuck::cast_slice(&mesh.transparent);
        let buffer = |label, size: u64, usage| {
            rs.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: size.max(4),
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let vertices = buffer(
            "scene vertices",
            vertex_bytes.len() as u64,
            wgpu::BufferUsages::VERTEX,
        );
        let indices = buffer(
            "scene indices",
            (opaque_bytes.len() + glass_bytes.len()) as u64,
            wgpu::BufferUsages::INDEX,
        );
        let upload = |buffer: &wgpu::Buffer, offset: u64, bytes: &[u8]| {
            // Bound each browser-side transfer, including after large MCP edits.
            for (i, chunk) in bytes.chunks(64 * 1024).enumerate() {
                rs.queue
                    .write_buffer(buffer, offset + (i * 64 * 1024) as u64, chunk);
            }
        };
        upload(&vertices, 0, vertex_bytes);
        upload(&indices, 0, opaque_bytes);
        upload(&indices, opaque_bytes.len() as u64, glass_bytes);
        self.mesh = Some(GpuMesh {
            vertices,
            indices,
            index_count: u32::try_from(mesh.indices.len()).expect("index count fits in u32"),
            transparent_count: u32::try_from(mesh.transparent.len())
                .expect("index count fits in u32"),
        });
    }

    fn ensure_targets(&mut self, rs: &RenderState, size: [u32; 2]) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let device = &rs.device;
        let texture = |label, format, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size[0],
                        height: size[1],
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let targets = Targets {
            size,
            msaa: texture(
                "scene msaa",
                COLOR_FORMAT,
                SAMPLES,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ),
            depth: texture(
                "scene depth",
                DEPTH_FORMAT,
                SAMPLES,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            ),
            resolve: texture(
                "scene color",
                COLOR_FORMAT,
                1,
                wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            ),
        };

        let mut renderer = rs.renderer.write();
        match self.texture_id {
            Some(id) => renderer.update_egui_texture_from_wgpu_texture(
                device,
                &targets.resolve,
                wgpu::FilterMode::Linear,
                id,
            ),
            None => {
                self.texture_id = Some(renderer.register_native_texture(
                    device,
                    &targets.resolve,
                    wgpu::FilterMode::Linear,
                ));
            }
        }
        self.targets = Some(targets);
    }

    fn render(&mut self, rs: &RenderState, size: [u32; 2], view_proj: Mat4, light: (Vec3, f32)) {
        self.ensure_targets(rs, size);
        let (Some(targets), Some(mesh)) = (&self.targets, &self.mesh) else {
            return;
        };

        let uniforms = Uniforms {
            view_proj: view_proj.to_cols_array_2d(),
            light_dir: light.0.to_array(),
            sun: light.1,
        };
        rs.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("scene encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &targets.msaa,
                    depth_slice: None,
                    resolve_target: Some(&targets.resolve),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(SKY),
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, mesh.vertices.slice(..));
            pass.set_index_buffer(mesh.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            if mesh.transparent_count > 0 {
                pass.set_pipeline(&self.transparent_pipeline);
                pass.draw_indexed(
                    mesh.index_count..mesh.index_count + mesh.transparent_count,
                    0,
                    0..1,
                );
            }
        }
        rs.queue.submit([encoder.finish()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eases the camera all the way to the side it was sent to.
    fn arrive(camera: &mut OrbitCamera) {
        for _ in 0..200 {
            camera.settle(1.0 / 60.0);
        }
        assert!(camera.goal.is_none(), "{camera:?}");
    }

    #[test]
    fn axes_look_at_the_house_from_each_side() {
        let mut camera = OrbitCamera::default();
        for (side, eye) in [
            (Side::Front, Vec3::Z),
            (Side::Back, Vec3::NEG_Z),
            (Side::Left, Vec3::NEG_X),
            (Side::Right, Vec3::X),
            (Side::Top, Vec3::Y),
        ] {
            camera.look_from(side);
            arrive(&mut camera);
            let from = (camera.eye() - camera.target).normalize();
            assert!(from.distance(eye) < 1e-3, "{side:?}: {from}");
        }
    }

    #[test]
    fn the_top_view_reads_like_the_plan() {
        let mut camera = OrbitCamera::default();
        camera.look_from(Side::Top);
        arrive(&mut camera);
        let basis = camera.basis();
        // Plan x to the right of the screen, plan y (world z) down it.
        assert!(basis.right.distance(Vec3::X) < 1e-3, "{}", basis.right);
        assert!(basis.up.distance(Vec3::NEG_Z) < 1e-3, "{}", basis.up);
        assert!(camera.view_proj(1.5).is_finite());
    }

    #[test]
    fn axes_turn_the_short_way_and_a_drag_takes_over() {
        let mut camera = OrbitCamera {
            yaw: 3.0 * TAU + 0.1,
            ..OrbitCamera::default()
        };
        camera.look_from(Side::Right);
        assert!((camera.goal.unwrap().0 - 3.0 * TAU).abs() < 1e-4);
        camera.settle(1.0 / 60.0);
        camera.orbit(egui::vec2(10.0, 0.0));
        assert!(camera.goal.is_none());
        assert!(!camera.settle(1.0 / 60.0));
    }

    #[test]
    fn live_sun_follows_the_compass_and_the_hour() {
        let mut home = Home::default();
        home.compass.latitude = Some(-23.5);
        home.compass.longitude = Some(-46.6);
        let (noon, strength) = sun_light(&home, 12.0);
        assert!(noon.y > 0.7 && strength > 0.9, "{noon} {strength}");
        let (_, night) = sun_light(&home, 0.0);
        assert!(night.abs() < f32::EPSILON);
        // Morning sun rises in the east; north up on the plan puts east at +x.
        let (morning, _) = sun_light(&home, 8.0);
        assert!(morning.x > 0.5, "{morning}");
        // Turning the compass half a turn moves it to the other side.
        home.compass.north_degrees = 180.0;
        let (turned, _) = sun_light(&home, 8.0);
        assert!(turned.x < -0.5, "{turned}");
    }

    /// A file swapped under the same path — a shop product's texture or
    /// model replaced by the hosted service — makes the scene build again;
    /// the same bytes do not.
    #[test]
    fn a_file_replaced_under_its_path_rebuilds_the_scene() {
        let dir = std::path::PathBuf::from("/memory/scene-files-seen-test");
        let mut home = newera_core::Home::default();
        let id = home.new_wall_id();
        let mut wall = newera_core::Wall::new(
            id,
            newera_core::Point2::new(0.0, 0.0),
            newera_core::Point2::new(400.0, 0.0),
        );
        wall.left_side = Some(newera_core::Material {
            image: Some("products/p1/finish.jpg".into()),
            tile: Some([30.0, 60.0]),
            ..newera_core::Material::default()
        });
        wall.right_side = Some(newera_core::Material {
            image: Some("products/p2/finish.jpg".into()),
            ..newera_core::Material::default()
        });
        home.walls.push(wall);
        // The second of the files the home draws with is the one replaced.
        let path = "products/p2/finish.jpg".to_owned();
        newera_core::vfs::mount(
            &dir,
            [
                ("products/p1/finish.jpg".to_owned(), vec![9]),
                (path.clone(), vec![1, 2, 3]),
            ],
        );
        let key = || built_for(7, Vec::new(), None, Vec::new(), &home, Some(&dir));
        let first = key();
        newera_core::vfs::mount(&dir, [(path.clone(), vec![1, 2, 3])]);
        assert_eq!(key(), first);
        newera_core::vfs::mount(&dir, [(path, vec![4, 5, 6])]);
        assert_ne!(key(), first);
        newera_core::vfs::unmount(&dir);
    }
}
