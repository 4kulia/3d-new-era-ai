//! 3D exports of scene meshes: binary glTF (`.glb`) with embedded textures
//! and Wavefront OBJ/MTL with texture files beside it.
//!
//! Positions are meters with Y up, the convention of both formats.
//! Procedural patterns can't travel, so their surfaces keep their color.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use crate::mesh::{IMAGE_BASE, Mesh, Vertex};

/// A surface look shared by several triangles.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
struct Look {
    /// Image layer when textured.
    image: Option<usize>,
    color: [f32; 3],
    alpha: f32,
    /// Alpha mask cutoff of the image, when it cuts holes.
    cutoff: Option<f32>,
}

type LookKey = (Option<usize>, [u32; 4], Option<u32>);

/// Encoded bytes and file extension of an image layer.
pub type ImageSource<'a> = &'a dyn Fn(&str) -> Option<(Vec<u8>, String)>;

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn key(v: &Vertex) -> LookKey {
    let layer = crate::mesh::kind_layer(v.kind);
    let image = (layer >= IMAGE_BASE).then(|| (layer - IMAGE_BASE) as usize);
    let q = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u32;
    (
        image,
        [q(v.color[0]), q(v.color[1]), q(v.color[2]), q(v.color[3])],
        crate::mesh::kind_cutoff(v.kind).map(q),
    )
}

/// Triangles grouped by look, in a stable order.
fn groups(mesh: &Mesh) -> BTreeMap<LookKey, Vec<u32>> {
    let mut out: BTreeMap<LookKey, Vec<u32>> = BTreeMap::new();
    for tri in mesh.indices.chunks(3).chain(mesh.transparent.chunks(3)) {
        if tri.len() < 3 {
            continue;
        }
        let Some(first) = mesh.vertices.get(tri[0] as usize) else {
            continue;
        };
        out.entry(key(first)).or_default().extend_from_slice(tri);
    }
    out
}

#[allow(clippy::cast_precision_loss)]
fn look(key: &LookKey) -> Look {
    Look {
        image: key.0,
        color: [key.1[0], key.1[1], key.1[2]].map(|c| c as f32 / 255.0),
        alpha: key.1[3] as f32 / 255.0,
        cutoff: key.2.map(|c| c as f32 / 255.0),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("unsupported 3D export format `{0}` (use .glb or .obj)")]
    Format(String),
}

/// Writes the mesh to `path`, choosing the format by extension. `images`
/// gives the encoded bytes (PNG/JPEG) and extension of each image layer.
pub fn export_mesh(mesh: &Mesh, path: &Path, images: ImageSource<'_>) -> Result<(), ExportError> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("glb") => {
            std::fs::write(path, glb(mesh, images))?;
            Ok(())
        }
        Some("obj") => obj(mesh, path, images),
        other => Err(ExportError::Format(other.unwrap_or_default().to_owned())),
    }
}

/// Binary glTF 2.0 with one primitive per look.
#[allow(clippy::cast_possible_truncation, clippy::too_many_lines)]
pub fn glb(mesh: &Mesh, images: ImageSource<'_>) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut push_view = |bin: &mut Vec<u8>, bytes: &[u8], target: Option<u32>| {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let offset = bin.len();
        bin.extend_from_slice(bytes);
        let mut view = format!(
            r#"{{"buffer":0,"byteOffset":{offset},"byteLength":{}"#,
            bytes.len()
        );
        if let Some(t) = target {
            let _ = write!(view, r#","target":{t}"#);
        }
        view.push('}');
        views.push(view);
        views.len() - 1
    };

    // Shared vertex attributes.
    let positions: Vec<u8> = mesh
        .vertices
        .iter()
        .flat_map(|v| v.position.iter().flat_map(|c| c.to_le_bytes()))
        .collect();
    let normals: Vec<u8> = mesh
        .vertices
        .iter()
        .flat_map(|v| v.normal.iter().flat_map(|c| c.to_le_bytes()))
        .collect();
    let uvs: Vec<u8> = mesh
        .vertices
        .iter()
        .flat_map(|v| [v.uv[0], -v.uv[1]].into_iter().flat_map(f32::to_le_bytes))
        .collect();
    let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
    for v in &mesh.vertices {
        for k in 0..3 {
            min[k] = min[k].min(v.position[k]);
            max[k] = max[k].max(v.position[k]);
        }
    }
    if mesh.vertices.is_empty() {
        (min, max) = ([0.0; 3], [0.0; 3]);
    }
    let count = mesh.vertices.len();
    let pv = push_view(&mut bin, &positions, Some(34962));
    accessors.push(format!(
        r#"{{"bufferView":{pv},"componentType":5126,"count":{count},"type":"VEC3","min":[{},{},{}],"max":[{},{},{}]}}"#,
        min[0], min[1], min[2], max[0], max[1], max[2]
    ));
    let nv = push_view(&mut bin, &normals, Some(34962));
    accessors.push(format!(
        r#"{{"bufferView":{nv},"componentType":5126,"count":{count},"type":"VEC3"}}"#
    ));
    let uv = push_view(&mut bin, &uvs, Some(34962));
    accessors.push(format!(
        r#"{{"bufferView":{uv},"componentType":5126,"count":{count},"type":"VEC2"}}"#
    ));

    // Images and textures.
    let mut image_json = Vec::new();
    let mut texture_of_layer: BTreeMap<usize, usize> = BTreeMap::new();
    for (layer, file) in mesh.images.iter().enumerate() {
        if let Some((bytes, ext)) = images(file) {
            let view = push_view(&mut bin, &bytes, None);
            let mime = if ext.eq_ignore_ascii_case("jpg") || ext.eq_ignore_ascii_case("jpeg") {
                "image/jpeg"
            } else {
                "image/png"
            };
            image_json.push(format!(r#"{{"bufferView":{view},"mimeType":"{mime}"}}"#));
            texture_of_layer.insert(layer, image_json.len() - 1);
        }
    }

    let mut materials = Vec::new();
    let mut primitives = Vec::new();
    for (k, indices) in groups(mesh) {
        let l = look(&k);
        let bytes: Vec<u8> = indices.iter().flat_map(|i| i.to_le_bytes()).collect();
        let iv = push_view(&mut bin, &bytes, Some(34963));
        accessors.push(format!(
            r#"{{"bufferView":{iv},"componentType":5125,"count":{},"type":"SCALAR"}}"#,
            indices.len()
        ));
        let accessor = accessors.len() - 1;
        let texture = l.image.and_then(|layer| texture_of_layer.get(&layer));
        let mut material = format!(
            r#"{{"pbrMetallicRoughness":{{"baseColorFactor":[{},{},{},{}],"metallicFactor":0,"roughnessFactor":0.9"#,
            l.color[0], l.color[1], l.color[2], l.alpha
        );
        if let Some(t) = texture {
            let _ = write!(material, r#","baseColorTexture":{{"index":{t}}}"#);
        }
        material.push('}');
        // One mode: a see-through color blends, the image's alpha included.
        if l.alpha < 0.99 {
            material.push_str(r#","alphaMode":"BLEND","doubleSided":true"#);
        } else if let (Some(cutoff), Some(_)) = (l.cutoff, texture) {
            let _ = write!(material, r#","alphaMode":"MASK","alphaCutoff":{cutoff}"#);
        }
        material.push('}');
        materials.push(material);
        primitives.push(format!(
            r#"{{"attributes":{{"POSITION":0,"NORMAL":1,"TEXCOORD_0":2}},"indices":{accessor},"material":{}}}"#,
            materials.len() - 1
        ));
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let textures: Vec<String> = (0..image_json.len())
        .map(|i| format!(r#"{{"source":{i}}}"#))
        .collect();
    let mut json = format!(
        r#"{{"asset":{{"version":"2.0","generator":"3D New Era AI"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0,"name":"home"}}],"meshes":[{{"primitives":[{}]}}],"materials":[{}],"accessors":[{}],"bufferViews":[{}],"buffers":[{{"byteLength":{}}}]"#,
        primitives.join(","),
        materials.join(","),
        accessors.join(","),
        views.join(","),
        bin.len()
    );
    if !image_json.is_empty() {
        let _ = write!(
            json,
            r#","images":[{}],"textures":[{}]"#,
            image_json.join(","),
            textures.join(",")
        );
    }
    json.push('}');
    while json.len() % 4 != 0 {
        json.push(' ');
    }
    let total = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(json.as_bytes());
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

/// An image whose alpha is only clear (under `cutoff`) or solid, as PNG.
fn cut_at(bytes: &[u8], cutoff: f32) -> Option<Vec<u8>> {
    let mut image = image::load_from_memory(bytes).ok()?.to_rgba8();
    for p in image.pixels_mut() {
        p[3] = if f32::from(p[3]) / 255.0 < cutoff {
            0
        } else {
            255
        };
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

/// OBJ + MTL (named like `path`), textures copied beside them.
fn obj(mesh: &Mesh, path: &Path, images: ImageSource<'_>) -> Result<(), ExportError> {
    let stem = path
        .file_stem()
        .map_or_else(|| "home".to_owned(), |s| s.to_string_lossy().into_owned());
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mtl_name = format!("{stem}.mtl");
    let mut obj = format!("# 3D New Era AI\nmtllib {mtl_name}\n");
    let mut mtl = String::from("# 3D New Era AI\n");
    for v in &mesh.vertices {
        let _ = writeln!(
            obj,
            "v {} {} {}",
            v.position[0], v.position[1], v.position[2]
        );
    }
    for v in &mesh.vertices {
        let _ = writeln!(obj, "vn {} {} {}", v.normal[0], v.normal[1], v.normal[2]);
    }
    for v in &mesh.vertices {
        let _ = writeln!(obj, "vt {} {}", v.uv[0], v.uv[1]);
    }
    let mut written_images: BTreeMap<(usize, Option<u32>), Option<String>> = BTreeMap::new();
    for (n, (k, indices)) in groups(mesh).into_iter().enumerate() {
        let l = look(&k);
        let name = format!("m{n}");
        let _ = writeln!(
            mtl,
            "newmtl {name}\nKd {} {} {}\nd {}",
            l.color[0], l.color[1], l.color[2], l.alpha
        );
        if let Some(layer) = l.image {
            let file = written_images.entry((layer, k.2)).or_insert_with(|| {
                let (bytes, ext) = images(mesh.images.get(layer)?)?;
                // MTL has no cutoff and is read back at half: the image's
                // alpha is cut at this one beforehand.
                let (bytes, file) = match l.cutoff {
                    Some(cutoff) => (
                        cut_at(&bytes, cutoff)?,
                        format!("{stem}_tex{layer}_cut{}.png", k.2.unwrap_or_default()),
                    ),
                    None => (bytes, format!("{stem}_tex{layer}.{ext}")),
                };
                std::fs::write(dir.join(&file), bytes).ok()?;
                Some(file)
            });
            if let Some(file) = file {
                let _ = writeln!(mtl, "map_Kd {file}");
                // Its own alpha cuts the holes, as the importer reads it back.
                if l.cutoff.is_some() {
                    let _ = writeln!(mtl, "map_d {file}");
                }
            }
        }
        let _ = writeln!(obj, "usemtl {name}");
        for tri in indices.chunks(3) {
            let [a, b, c] = [tri[0] + 1, tri[1] + 1, tri[2] + 1];
            let _ = writeln!(obj, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}");
        }
    }
    std::fs::write(path, obj)?;
    std::fs::write(dir.join(mtl_name), mtl)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Selection;
    use newera_core::{Home, Point2, Room, Wall};

    fn home() -> Home {
        let mut home = Home::default();
        let pts = [(0.0, 0.0), (400.0, 0.0), (400.0, 300.0), (0.0, 300.0)];
        for i in 0..4 {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            let id = home.new_wall_id();
            home.walls
                .push(Wall::new(id, Point2::new(a.0, a.1), Point2::new(b.0, b.1)));
        }
        let id = home.new_room_id();
        home.rooms.push(Room::new(
            id,
            "Sala",
            pts.iter().map(|&(x, y)| Point2::new(x, y)).collect(),
        ));
        home
    }

    #[test]
    fn glb_is_valid_and_obj_round_trips_through_the_loader() {
        let mut mesh = Mesh::from_home(&home(), &Selection::new(), &|_| None);
        mesh.drop_ground();
        let glb = glb(&mesh, &|_| None);
        assert_eq!(&glb[..4], b"glTF");
        let total = u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize;
        assert_eq!(total, glb.len());
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json = std::str::from_utf8(&glb[20..20 + json_len]).unwrap();
        assert!(json.contains(r#""POSITION":0"#), "{json}");

        let dir = std::env::temp_dir().join(format!("newera-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        export_mesh(&mesh, &dir.join("casa.glb"), &|_| None).unwrap();
        let loaded = newera_catalog::load_model(&dir.join("casa.glb")).unwrap();
        assert_ne!(loaded.mesh.indices, [] as [u32; 0]);
        export_mesh(&mesh, &dir.join("casa.obj"), &|_| None).unwrap();
        let loaded = newera_catalog::load_model(&dir.join("casa.obj")).unwrap();
        // 4 m × 3 m house plus walls, in cm after unit detection.
        assert!(
            loaded.size[0] > 400.0 && loaded.size[1] > 300.0,
            "{:?}",
            loaded.size
        );
        assert!(matches!(
            export_mesh(&mesh, &dir.join("casa.fbx"), &|_| None),
            Err(ExportError::Format(_))
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// One textured triangle cut at `cutoff`, `alpha` opaque, and its image:
    /// a single texel at alpha 0.4.
    fn masked_triangle(cutoff: f32, alpha: f32) -> (Mesh, Vec<u8>) {
        let kind = crate::mesh::masked(crate::mesh::IMAGE_BASE, cutoff);
        let vertex = |position: [f32; 3], uv: [f32; 2]| Vertex {
            position,
            normal: [0.0, 1.0, 0.0],
            color: [1.0, 1.0, 1.0, alpha],
            uv,
            kind,
        };
        let mesh = Mesh {
            vertices: vec![
                vertex([0.0, 0.0, 0.0], [0.0, 0.0]),
                vertex([1.0, 0.0, 0.0], [1.0, 0.0]),
                vertex([0.0, 0.0, 1.0], [0.0, 1.0]),
            ],
            indices: vec![0, 1, 2],
            transparent: vec![],
            images: vec!["palha.png".into()],
        };
        let mut png = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(1, 1, image::Rgba([200, 170, 110, 102]))
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        (mesh, png.into_inner())
    }

    #[test]
    fn a_masked_see_through_material_has_one_alpha_mode() {
        let (mesh, png) = masked_triangle(0.3, 0.5);
        let glb = glb(&mesh, &|_| Some((png.clone(), "png".into())));
        let json_len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let json = std::str::from_utf8(&glb[20..20 + json_len]).unwrap();
        assert_eq!(json.matches("alphaMode").count(), 1, "{json}");
        assert!(json.contains(r#""alphaMode":"BLEND""#), "{json}");
    }

    #[test]
    fn a_mask_keeps_its_cutoff_through_obj() {
        // Alpha 0.4 over a 0.3 cutoff is solid; MTL is read back at 0.5.
        let (mesh, png) = masked_triangle(0.3, 1.0);
        let dir = std::env::temp_dir().join(format!("newera-export-cut-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        export_mesh(&mesh, &dir.join("cadeira.obj"), &|_| {
            Some((png.clone(), "png".into()))
        })
        .unwrap();
        let mtl = std::fs::read_to_string(dir.join("cadeira.mtl")).unwrap();
        let file = mtl
            .lines()
            .find_map(|l| l.strip_prefix("map_d "))
            .unwrap()
            .to_owned();
        assert!(mtl.contains(&format!("map_Kd {file}")), "{mtl}");
        let texel = image::open(dir.join(&file)).unwrap().to_rgba8();
        assert_eq!(texel.get_pixel(0, 0)[3], 255);
        let back = newera_catalog::load_model(&dir.join("cadeira.obj")).unwrap();
        assert_eq!(back.mesh.materials[0].cutoff, Some(0.5));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
