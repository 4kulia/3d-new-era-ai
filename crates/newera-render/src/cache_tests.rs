use super::{ModelCache, TopViews, asset_versions::AssetVersions};
use newera_core::{Furniture, vfs};
use std::path::PathBuf;

struct Assets(PathBuf);
impl Assets {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "newera-cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn mount(&self, name: &str, bytes: &[u8]) {
        vfs::mount(&self.0, [(name.into(), bytes.to_vec())]);
    }
    fn piece() -> Furniture {
        Furniture {
            model: Some("test.obj".into()),
            width: 80.0,
            depth: 60.0,
            height: 20.0,
            ..Furniture::default()
        }
    }
}
impl Drop for Assets {
    fn drop(&mut self) {
        vfs::unmount(&self.0);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
const OBJ: &[u8] =
    b"mtllib test.mtl\nv -1 0 -1\nv 1 0 -1\nv 1 0 1\nv -1 0 1\nusemtl body\nf 1 4 3\nf 1 3 2\n";
const RED: &[u8] = b"newmtl body\nKd 1 0 0\n";
const BLUE: &[u8] = b"newmtl body\nKd 0 0 1\n";

#[test]
fn missing_models_recover_and_material_replacements_change_the_mesh() {
    let assets = Assets::new();
    let piece = Assets::piece();
    let cache = ModelCache::default();
    assert!(cache.piece_model(&piece, Some(&assets.0)).is_none());
    assets.mount("test.obj", OBJ);
    assets.mount("test.mtl", RED);
    let red = cache.piece_model(&piece, Some(&assets.0)).unwrap();
    assets.mount("test.mtl", BLUE);
    let blue = cache.piece_model(&piece, Some(&assets.0)).unwrap();
    assert_eq!(red.positions, blue.positions);
    assert_ne!(red.colors, blue.colors);
    assert_eq!(
        blue.colors,
        cache.piece_model(&piece, Some(&assets.0)).unwrap().colors
    );
}

#[test]
fn dependency_changes_track_missing_and_retargeted_textures() {
    let assets = Assets::new();
    let path = assets.0.join("test.obj");
    assets.mount("test.obj", OBJ);
    let mut versions = AssetVersions::default();
    let missing = versions.get(&path);
    assets.mount("test.mtl", b"newmtl body\nmap_Kd a.png\n");
    let material = versions.get(&path);
    assert_ne!(missing, material);
    assets.mount("a.png", b"red");
    let image = versions.get(&path);
    assert_ne!(material, image);
    assets.mount("a.png", b"blu");
    assert_ne!(image, versions.get(&path));
    assets.mount("test.mtl", b"newmtl body\nmap_Kd b.png\n");
    let retargeted = versions.get(&path);
    assets.mount("a.png", b"unused");
    assert_eq!(retargeted, versions.get(&path));
    assets.mount("b.png", b"new");
    assert_ne!(retargeted, versions.get(&path));
}

#[test]
fn gltf_external_buffers_and_images_invalidate() {
    let assets = Assets::new();
    assets.mount(
        "model.gltf",
        br#"{"buffers":[{"uri":"mesh.bin"}],"images":[{"uri":"image.png"}]}"#,
    );
    let path = assets.0.join("model.gltf");
    let mut versions = AssetVersions::default();
    let first = versions.get(&path);
    assets.mount("mesh.bin", b"geometry");
    let buffer = versions.get(&path);
    assert_ne!(first, buffer);
    assets.mount("image.png", b"image");
    assert_ne!(buffer, versions.get(&path));
}

#[test]
fn persistent_png_is_replaced_after_material_or_renderer_change() {
    let assets = Assets::new();
    assets.mount("test.obj", OBJ);
    assets.mount("test.mtl", RED);
    let piece = Assets::piece();
    let views = TopViews::new(assets.0.join("png"), Some(assets.0.clone()), true);
    let red = views.image_for(&piece).unwrap();
    let red_pixels = image::open(&red).unwrap().to_rgba8();
    assert!(red_pixels.pixels().any(|p| p[3] > 0 && p[0] > p[2]));
    assert_eq!(Some(red.clone()), views.image_for(&piece));
    assets.mount("test.mtl", BLUE);
    let blue = views.image_for(&piece).unwrap();
    assert_ne!(red, blue);
    let blue_pixels = image::open(&blue).unwrap().to_rgba8();
    assert!(blue_pixels.pixels().any(|p| p[3] > 0 && p[2] > p[0]));
    assert_ne!(red_pixels, blue_pixels);
    let mut rebuilt = TopViews::new(assets.0.join("png"), Some(assets.0.clone()), true);
    rebuilt.source_revision = "a different renderer build";
    assert_ne!(Some(blue), rebuilt.image_for(&piece));
    let mut moved = piece.clone();
    moved.position.x = 500.0;
    moved.name = "renamed".into();
    assert_eq!(rebuilt.image_for(&piece), rebuilt.image_for(&moved));
}

#[test]
fn native_files_recover_and_same_length_edits_reload_materials() {
    let assets = Assets::new();
    let piece = Assets::piece();
    let cache = ModelCache::default();
    assert!(cache.piece_model(&piece, Some(&assets.0)).is_none());
    std::fs::write(assets.0.join("test.obj"), OBJ).unwrap();
    let mtl = assets.0.join("test.mtl");
    std::fs::write(&mtl, RED).unwrap();
    let red = cache.piece_model(&piece, Some(&assets.0)).unwrap();
    let old = std::fs::metadata(&mtl).unwrap().modified().unwrap();
    std::fs::write(&mtl, BLUE).unwrap();
    std::fs::File::options()
        .write(true)
        .open(&mtl)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(old + std::time::Duration::from_secs(2)))
        .unwrap();
    let blue = cache.piece_model(&piece, Some(&assets.0)).unwrap();
    assert_ne!(red.colors, blue.colors);
}

#[test]
fn image_overrides_and_changed_model_geometry_invalidate_views() {
    let assets = Assets::new();
    assets.mount("test.obj", OBJ);
    assets.mount("test.mtl", RED);
    let mut piece = Assets::piece();
    let views = TopViews::new(assets.0.join("png"), Some(assets.0.clone()), true);
    let cache = ModelCache::default();
    let original = cache.piece_model(&piece, Some(&assets.0)).unwrap();
    assets.mount(
        "test.obj",
        &String::from_utf8_lossy(OBJ)
            .replace("v 1 0 1", "v 0 0 1")
            .into_bytes(),
    );
    let changed = cache.piece_model(&piece, Some(&assets.0)).unwrap();
    assert_ne!(original.positions, changed.positions);
    for material_override in [false, true] {
        let texture = newera_core::Material {
            image: Some("finish.png".into()),
            ..Default::default()
        };
        if material_override {
            piece.texture = None;
            piece.materials.push(newera_core::ModelMaterial {
                name: "body".into(),
                key: None,
                color: None,
                texture: Some(texture),
                shininess: None,
                repeat: None,
            });
        } else {
            piece.texture = Some(texture);
        }
        for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
            let pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba(color));
            let mut bytes = std::io::Cursor::new(Vec::new());
            pixels
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            let before = views.key(&piece).unwrap();
            assets.mount("finish.png", &bytes.into_inner());
            let after = views.key(&piece).unwrap();
            assert_ne!(before, after);
            assert!(
                views.produce(&piece, before).is_none(),
                "an outdated queued version cannot publish a PNG"
            );
            assert!(views.image_for(&piece).is_some());
        }
        // Separate the second scenario from the first scenario's images.
        std::fs::remove_dir_all(&views.dir).unwrap();
        views.inner.lock().unwrap().done.clear();
    }
}

#[test]
fn background_queue_discards_outdated_work_and_becomes_idle() {
    let assets = Assets::new();
    assets.mount("test.obj", OBJ);
    assets.mount("test.mtl", RED);
    let piece = Assets::piece();
    let views = TopViews::in_background(assets.0.join("png"), Some(assets.0.clone()), true);
    let stale = views.key(&piece).unwrap();
    {
        // Hold the cache so the worker cannot start loading until the edit is done.
        let mut cache = views.inner.lock().unwrap();
        cache.queued.insert(stale);
        views
            .background
            .as_ref()
            .unwrap()
            .send((stale, piece.clone()))
            .unwrap();
        assets.mount("test.mtl", BLUE);
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while views.generation() == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "worker never completed"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(!views.busy());
    assert!(!views.inner.lock().unwrap().done.contains_key(&stale));
    assert!(!views.dir.join(format!("{stale:016x}.png")).exists());
    let path = loop {
        if let Some(path) = views.image_for(&piece) {
            break path;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "updated image never completed"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    assert!(!views.busy());
    assert!(
        image::open(path)
            .unwrap()
            .to_rgba8()
            .pixels()
            .any(|p| p[3] > 0 && p[2] > p[0])
    );
}

#[test]
fn cache_publication_hides_partial_writes_and_cleans_failed_encodes() {
    use std::io::Write;
    let assets = Assets::new();
    let path = assets.0.join("atomic.png");
    let expected = image::RgbaImage::from_pixel(8, 8, image::Rgba([12, 34, 56, 255]));
    let mut encoded = std::io::Cursor::new(Vec::new());
    expected
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let encoded = encoded.into_inner();
    super::publish_cache_file(&path, |file| {
        file.write_all(&encoded[..12])?;
        assert!(
            !path.exists(),
            "another provider must not discover a partial PNG"
        );
        file.write_all(&encoded[12..])
    })
    .unwrap();
    assert_eq!(image::open(&path).unwrap().to_rgba8(), expected);
    let error = super::publish_cache_file(&path, |file| {
        file.write_all(b"partial replacement")?;
        assert_eq!(image::open(&path).unwrap().to_rgba8(), expected);
        Err(std::io::Error::other("injected encoding failure"))
    });
    assert!(error.is_err());
    assert_eq!(image::open(&path).unwrap().to_rgba8(), expected);
    assert!(!std::fs::read_dir(&assets.0).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "tmp")
    }));
}

/// A GLB floor tile, 2 × 2 units, whose color is a PNG in its binary chunk.
fn textured_glb(png: &[u8]) -> Vec<u8> {
    let mut bin = Vec::new();
    for v in [
        [-1.0f32, 0.0, -1.0],
        [-1.0, 0.0, 1.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, -1.0],
    ] {
        bin.extend(v.iter().flat_map(|c| c.to_le_bytes()));
    }
    for t in [[0.0f32, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]] {
        bin.extend(t.iter().flat_map(|c| c.to_le_bytes()));
    }
    for i in [0u16, 1, 2, 0, 2, 3] {
        bin.extend(i.to_le_bytes());
    }
    let image_at = bin.len();
    bin.extend_from_slice(png);
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let mut json = format!(
        r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],"nodes":[{{"mesh":0}}],
        "meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"TEXCOORD_0":1}},"indices":2,"material":0}}]}}],
        "materials":[{{"name":"body","pbrMetallicRoughness":{{"baseColorTexture":{{"index":0}}}}}}],
        "textures":[{{"source":0}}],"images":[{{"bufferView":3,"mimeType":"image/png"}}],
        "buffers":[{{"byteLength":{}}}],
        "bufferViews":[{{"buffer":0,"byteLength":48}},{{"buffer":0,"byteOffset":48,"byteLength":32}},
                       {{"buffer":0,"byteOffset":80,"byteLength":12}},{{"buffer":0,"byteOffset":{image_at},"byteLength":{}}}],
        "accessors":[{{"bufferView":0,"componentType":5126,"count":4,"type":"VEC3","min":[-1,0,-1],"max":[1,0,1]}},
                     {{"bufferView":1,"componentType":5126,"count":4,"type":"VEC2"}},
                     {{"bufferView":2,"componentType":5123,"count":6,"type":"SCALAR"}}]}}"#,
        bin.len(),
        png.len(),
    )
    .into_bytes();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let chunk = |kind: &[u8; 4], data: &[u8]| {
        let mut out = u32::try_from(data.len()).unwrap().to_le_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out
    };
    let body = [chunk(b"JSON", &json), chunk(b"BIN\0", &bin)].concat();
    let mut glb = b"glTF".to_vec();
    glb.extend(2u32.to_le_bytes());
    glb.extend(u32::try_from(12 + body.len()).unwrap().to_le_bytes());
    glb.extend(body);
    glb
}

#[test]
fn a_glb_is_drawn_with_the_image_inside_it() {
    let assets = Assets::new();
    let pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba([220, 20, 20, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    pixels.write_to(&mut png, image::ImageFormat::Png).unwrap();
    assets.mount("chair.glb", &textured_glb(&png.into_inner()));
    let piece = Furniture {
        model: Some("chair.glb".into()),
        ..Assets::piece()
    };
    let views = TopViews::new(assets.0.join("png"), Some(assets.0.clone()), true);
    let top = image::open(views.image_for(&piece).unwrap())
        .unwrap()
        .to_rgba8();
    let drawn: Vec<_> = top.pixels().filter(|p| p[3] > 0).collect();
    assert_ne!(drawn, [] as [&image::Rgba<u8>; 0]);
    assert!(
        drawn.iter().all(|p| p[0] > p[1].saturating_add(80)),
        "the tile shows its red image, not the plain material color"
    );
}

#[test]
fn a_textured_piece_exported_as_glb_comes_back_with_its_image() {
    let assets = Assets::new();
    let pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba([220, 20, 20, 255]));
    let mut png = std::io::Cursor::new(Vec::new());
    pixels.write_to(&mut png, image::ImageFormat::Png).unwrap();
    assets.mount("red.png", &png.into_inner());
    assets.mount(
        "test.obj",
        b"mtllib test.mtl\nv -1 0 -1\nv 1 0 -1\nv 1 0 1\nv -1 0 1\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl body\nf 1/1 4/4 3/3\nf 1/1 3/3 2/2\n",
    );
    assets.mount("test.mtl", b"newmtl body\nmap_Kd red.png\n");
    let mut home = newera_core::Home::default();
    home.furniture.push(Assets::piece());
    let out = assets.0.join("export");
    std::fs::create_dir_all(&out).unwrap();
    crate::export_home(&home, &out.join("sala.glb"), Some(&assets.0)).unwrap();
    let loaded = newera_catalog::load_model(&out.join("sala.glb")).unwrap();
    let texture = loaded
        .mesh
        .materials
        .iter()
        .find_map(|m| m.texture.as_ref())
        .expect("the exported image is read back");
    let image = newera_core::images::decode(&vfs::read(texture).unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(image.get_pixel(0, 0).0, [220, 20, 20, 255]);
}

#[test]
fn a_hidden_part_is_not_drawn_and_the_rest_keeps_its_place() {
    let assets = Assets::new();
    assets.mount(
        "test.obj",
        b"o assento\nv -1 0 -1\nv 1 0 -1\nv 1 0 1\nf 1 3 2\no encosto\nv -1 0 1\nv 1 0 1\nv 1 2 1\nf 4 5 6\n",
    );
    let mut piece = Assets::piece();
    let cache = ModelCache::default();
    let whole = cache.piece_model(&piece, Some(&assets.0)).unwrap();
    piece.model_parts.push(newera_core::ModelPart {
        name: "encosto".into(),
        hidden: true,
        ..Default::default()
    });
    let seat = cache.piece_model(&piece, Some(&assets.0)).unwrap();
    assert_eq!((whole.indices.len(), seat.indices.len()), (6, 3));
    assert_eq!(seat.parts.len(), 1);
    assert_eq!(
        &seat.positions[..3],
        &whole.positions[..3],
        "fitted before hiding"
    );
}

#[test]
fn a_masked_weave_shows_what_is_under_its_holes() {
    let assets = Assets::new();
    // Clear on its left half: the holes of the weave.
    let mut pixels = image::RgbaImage::from_pixel(8, 8, image::Rgba([200, 170, 110, 255]));
    for x in 0..4 {
        for y in 0..8 {
            pixels.put_pixel(x, y, image::Rgba([200, 170, 110, 0]));
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    pixels.write_to(&mut png, image::ImageFormat::Png).unwrap();
    assets.mount("palha.png", &png.into_inner());
    assets.mount(
        "test.obj",
        b"mtllib test.mtl\nv -1 0 -1\nv 1 0 -1\nv 1 0 1\nv -1 0 1\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl palha\nf 1/1 4/4 3/3\nf 1/1 3/3 2/2\n",
    );
    let drawn = |mtl: &[u8]| {
        assets.mount("test.mtl", mtl);
        let views = TopViews::new(
            assets.0.join(format!("png{}", mtl.len())),
            Some(assets.0.clone()),
            true,
        );
        let top = image::open(views.image_for(&Assets::piece()).unwrap())
            .unwrap()
            .to_rgba8();
        top.pixels().filter(|p| p[3] > 0).count()
    };
    let solid = drawn(b"newmtl palha\nmap_Kd palha.png\n");
    let masked = drawn(b"newmtl palha\nmap_Kd palha.png\nmap_d palha.png\n");
    assert!(solid > 0);
    #[allow(clippy::cast_precision_loss)]
    let share = masked as f64 / solid as f64;
    assert!((0.35..0.65).contains(&share), "{masked} of {solid} drawn");
}

#[test]
fn a_masked_weave_is_seen_from_both_sides_and_exported_as_a_mask() {
    let assets = Assets::new();
    let pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba([200, 170, 110, 0]));
    let mut png = std::io::Cursor::new(Vec::new());
    pixels.write_to(&mut png, image::ImageFormat::Png).unwrap();
    assets.mount("palha.png", &png.into_inner());
    assets.mount(
        "test.obj",
        b"mtllib test.mtl\nv -1 0 -1\nv 1 0 -1\nv 1 0 1\nvt 0 0\nvt 1 0\nvt 1 1\nusemtl palha\nf 1/1 3/3 2/2\n",
    );
    assets.mount(
        "test.mtl",
        b"newmtl palha\nmap_Kd palha.png\nmap_d palha.png\n",
    );
    let mut home = newera_core::Home::default();
    home.furniture.push(Assets::piece());
    let cache = ModelCache::default();
    let models = |piece: &Furniture| cache.piece_model(piece, Some(&assets.0));
    let mesh = crate::Mesh::from_home(&home, &crate::Selection::new(), &models);
    let masked: Vec<_> = mesh
        .vertices
        .iter()
        .filter(|v| crate::mesh::kind_cutoff(v.kind).is_some())
        .collect();
    // One triangle, and its back facing the other way.
    assert_eq!(masked.len(), 6);
    assert!((masked[0].normal[1] + masked[3].normal[1]).abs() < 1e-6);
    let out = assets.0.join("export");
    std::fs::create_dir_all(&out).unwrap();
    crate::export_home(&home, &out.join("sala.glb"), Some(&assets.0)).unwrap();
    let back = newera_catalog::load_model(&out.join("sala.glb")).unwrap();
    assert!(
        back.mesh.materials.iter().any(|m| m.cutoff.is_some()),
        "{:?}",
        back.mesh.materials
    );
}

#[test]
fn a_piece_far_from_the_camera_is_drawn_with_its_lighter_file() {
    let assets = Assets::new();
    // The detailed file: two triangles; the lighter one: a single one.
    assets.mount("test.obj", OBJ);
    assets.mount("test.mtl", RED);
    assets.mount("light.obj", b"v -1 0 -1\nv 1 0 -1\nv 1 0 1\nf 1 3 2\n");
    let mut piece = Assets::piece();
    piece.model_far = Some(newera_core::FarModel {
        file: "light.obj".into(),
        beyond: 300.0,
        off: false,
    });
    let cache = ModelCache::default();
    let tris = |distance: Option<f64>, piece: &Furniture| {
        cache
            .piece_model_seen(piece, Some(&assets.0), distance)
            .unwrap()
            .indices
            .len()
            / 3
    };
    assert_eq!(tris(Some(100.0), &piece), 2);
    assert_eq!(tris(Some(500.0), &piece), 1);
    // Fitted to the same box either way.
    let far = cache
        .piece_model_seen(&piece, Some(&assets.0), Some(500.0))
        .unwrap();
    let (min, max) = far.bounds().unwrap();
    assert!((max[0] - min[0] - 80.0).abs() < 1e-3, "{min:?} {max:?}");
    // Held detailed for a review; and without a camera (exports, top views).
    let mut review = piece.clone();
    review.model_far.as_mut().unwrap().off = true;
    assert_eq!(tris(Some(500.0), &review), 2);
    assert_eq!(tris(None, &piece), 2);
    // A lighter file that is gone leaves the piece drawn in full.
    let mut lost = piece.clone();
    lost.model_far.as_mut().unwrap().file = "missing.obj".into();
    assert_eq!(tris(Some(500.0), &lost), 2);
    // And it travels with the project like the model does.
    let mut home = newera_core::Home::default();
    home.furniture.push(piece.clone());
    assert!(home.asset_paths().contains(&"light.obj".to_owned()));
    // Distance from a camera, cm: 4 m in front of a piece standing at the origin.
    let home = newera_core::Home::default();
    let d = crate::camera_distance(&home, &piece, glam::Vec3::new(0.0, 0.1, 4.0));
    assert!((d - 400.0).abs() < 1e-3, "{d}");
}
