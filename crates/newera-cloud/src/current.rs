//! A stored project as it is opened: drawn with its products' current
//! files.
//!
//! A product's model (`crate::models`) and a finish's texture
//! (`crate::textures`) are kept once, beside the product, and copied into
//! each project that draws them, as `products/<id>/model.glb` and
//! `products/<id>/finish.jpg`. When one changes — a model attached or
//! generated, a new swatch — the copy open now takes it at once
//! (`crate::staging::swap`); every other project takes it here, the next
//! time it is opened: by the editor tab or a download
//! (`GET /account/projects/{id}`), or by the engine. The files it holds that
//! are not the product's current ones are replaced, and the surfaces drawn
//! with a finish's texture take the size it is drawn at now
//! ([`crate::textures::drawn_tile`]).
//!
//! What is opened is brought up to date; what is stored is not touched. The
//! stored revision stays the one it was, so an editor tab saving on it is
//! not turned away, and the next save — the tab's or the engine's — keeps
//! the current files, counted against the plan as any save is. A project
//! that draws no product, or only with their current files, opens as the
//! very bytes stored.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use newera_core::{BundledFiles, Document, Project};
use newera_shop::product::Fields;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use sqlx::types::Json;

use crate::secret;
use crate::staging::model_path;
use crate::textures::texture_path;

/// The product a path inside a project is the `file` of
/// (`products/<id>/<file>`).
fn product_of<'a>(path: &'a str, file: &str) -> Option<&'a str> {
    let id = path.strip_prefix("products/")?.strip_suffix(file)?;
    let id = id.strip_suffix('/')?;
    (!id.is_empty() && !id.contains('/')).then_some(id)
}

/// The products whose files the project draws with: those with a model,
/// and those with a texture.
fn held(project: &Project) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut models = BTreeSet::new();
    let mut textures = BTreeSet::new();
    for (_, home) in &project.variants {
        home.for_each_asset(&mut |path| {
            if let Some(id) = product_of(path, "model.glb") {
                models.insert(id.to_owned());
            } else if let Some(id) = product_of(path, "finish.jpg") {
                textures.insert(id.to_owned());
            }
        });
    }
    (models, textures)
}

/// What the server keeps of one product a project draws.
#[derive(Debug, Clone, Default)]
pub struct Kept {
    pub fields: Fields,
    /// The SHA-256 of its model's bytes, when it has one.
    pub model: Option<Vec<u8>>,
    /// Its texture, and the link it was made from.
    pub texture: Option<(Vec<u8>, Option<String>)>,
}

/// What brings a project up to date.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Changes {
    /// Products whose model the project holds is not the current one.
    pub models: Vec<String>,
    /// Files to put in the project, by path inside it.
    pub files: Vec<(String, Vec<u8>)>,
    /// Textures, by path, whose surfaces are drawn at another size now.
    pub tiles: Vec<(String, [f64; 2])>,
}

impl Changes {
    fn is_empty(&self) -> bool {
        self.models.is_empty() && self.files.is_empty() && self.tiles.is_empty()
    }
}

/// What differs between the products' files `project` holds (`bundled`)
/// and the ones `kept` now. A product the server no longer has leaves its
/// files as they are.
#[must_use]
pub fn changes(
    project: &Project,
    bundled: &BundledFiles,
    kept: &BTreeMap<String, Kept>,
) -> Changes {
    let inside = |path: &str| {
        bundled
            .iter()
            .find(|(name, _)| name == path)
            .map(|(_, bytes)| bytes.as_slice())
    };
    let (models, textures) = held(project);
    let mut out = Changes::default();
    for id in models {
        let Some(digest) = kept.get(&id).and_then(|k| k.model.as_deref()) else {
            continue;
        };
        if inside(&model_path(&id)).is_none_or(|b| Sha256::digest(b).as_slice() != digest) {
            out.models.push(id);
        }
    }
    for id in textures {
        let Some(product) = kept.get(&id) else {
            continue;
        };
        let Some((bytes, basis)) = &product.texture else {
            continue;
        };
        let path = texture_path(&id);
        // A bundle keeps a large JPEG recompressed: either form is the file.
        let holds = inside(&path).is_some_and(|b| {
            b == bytes.as_slice() || b == newera_core::packed_asset(&path, bytes.clone())
        });
        if !holds {
            out.files.push((path.clone(), bytes.clone()));
        }
        if let Some(tile) = crate::textures::drawn_tile(&product.fields, basis.as_deref(), bytes)
            && project
                .variants
                .iter()
                .any(|(_, home)| !newera_core::finishes::retiled(home, &path, tile).is_empty())
        {
            out.tiles.push((path, tile));
        }
    }
    out
}

/// The project, as the bytes of a `.newera`, with `changes` made: its files
/// replaced and its surfaces retiled. Its files are read from memory,
/// mounted in a directory of this call's own under `under` (the engine's
/// root).
///
/// # Errors
///
/// When a variant refuses the new size, or the bundle cannot be written.
pub fn rewrite(
    project: Project,
    bundled: BundledFiles,
    changes: &Changes,
    under: &Path,
) -> Result<Vec<u8>, String> {
    let mut files = bundled;
    for (path, bytes) in &changes.files {
        files.retain(|(name, _)| name != path);
        files.push((path.clone(), bytes.clone()));
    }
    // Inside the engine's root, where the jail lets reads go: a file the
    // project names but does not hold is then not found, and kept as a
    // dangling reference, as every save keeps it. Nothing is on disk there.
    let dir = under.join(format!("current-project-{}", secret::id()));
    newera_core::vfs::mount(&dir, files);
    let made = (|| {
        let mut doc = Document::default();
        project.load_into(&mut doc);
        doc.set_asset_dir(Some(dir.clone()));
        for (path, tile) in &changes.tiles {
            newera_core::finishes::retile(&mut doc, path, *tile).map_err(|e| e.to_string())?;
        }
        newera_core::to_bundle_bytes(&doc).map_err(|e| e.to_string())
    })();
    newera_core::vfs::unmount(&dir);
    made
}

/// A product's id and fields, its model's digest, its texture and the
/// link it was made from.
type Row = (
    String,
    Json<Fields>,
    Option<Vec<u8>>,
    Option<Vec<u8>>,
    Option<String>,
);

/// What the server keeps of the account's `products`.
async fn kept(
    db: &PgPool,
    account: &str,
    products: &[String],
) -> sqlx::Result<BTreeMap<String, Kept>> {
    let rows: Vec<Row> = sqlx::query_as(
        "select p.id, p.fields,
                    (select sha256(m.data) from product_assets m
                     where m.product_id = p.id and m.kind = 'model'),
                    t.data, t.basis
             from products p
             left join product_assets t on t.product_id = p.id and t.kind = 'texture'
             where p.account_id = $1 and p.id = any($2)",
    )
    .bind(account)
    .bind(products)
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, fields, model, texture, basis)| {
            (
                id,
                Kept {
                    fields: fields.0,
                    model,
                    texture: texture.map(|bytes| (bytes, basis)),
                },
            )
        })
        .collect())
}

/// The stored project `data` with its products' current files, for the
/// account that owns it: `None` when it holds them already (or draws no
/// product), and it opens as stored. `under` is the engine's root, where
/// the copy is put together ([`rewrite`]).
///
/// # Errors
///
/// When the database fails.
pub async fn current(
    db: &PgPool,
    account: &str,
    data: &[u8],
    under: &Path,
) -> anyhow::Result<Option<Vec<u8>>> {
    let owned = data.to_vec();
    let read = tokio::task::spawn_blocking(move || {
        newera_core::project_from_bytes(&owned).map(|(project, files)| {
            let (models, textures) = held(&project);
            (
                project,
                files,
                models.into_iter().chain(textures).collect::<Vec<_>>(),
            )
        })
    })
    .await?;
    // A project that does not read opens as stored, and says why there.
    let Ok((project, bundled, products)) = read else {
        return Ok(None);
    };
    if products.is_empty() {
        return Ok(None);
    }
    let kept = kept(db, account, &products).await?;
    // Hashing models and repacking textures is CPU work: off the runtime.
    let (project, bundled, mut changes) = tokio::task::spawn_blocking(move || {
        let changes = changes(&project, &bundled, &kept);
        (project, bundled, changes)
    })
    .await?;
    if changes.is_empty() {
        return Ok(None);
    }
    // Each id is the account's: `kept` found it among its products.
    for id in &changes.models {
        let model: Option<Vec<u8>> = sqlx::query_scalar(
            "select data from product_assets where product_id = $1 and kind = 'model'",
        )
        .bind(id)
        .fetch_optional(db)
        .await?;
        if let Some(bytes) = model {
            changes.files.push((model_path(id), bytes));
        }
    }
    let under = under.to_owned();
    let made =
        tokio::task::spawn_blocking(move || rewrite(project, bundled, &changes, &under)).await?;
    match made {
        Ok(bytes) => Ok(Some(bytes)),
        Err(why) => {
            tracing::warn!("a project opens with its products' old files: {why}");
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use newera_core::{Furniture, Home, Material, Project, Room, Wall};
    use newera_shop::product::{Evidence, Field, Fields};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};

    use super::{Changes, Kept};

    fn fields(of: &[(&str, Value)]) -> Fields {
        of.iter()
            .map(|(name, value)| {
                (
                    (*name).to_owned(),
                    Field {
                        value: value.clone(),
                        evidence: Evidence::Agent,
                        from: "test".to_owned(),
                    },
                )
            })
            .collect()
    }

    fn jpeg(w: u32, h: u32) -> Vec<u8> {
        let image = image::RgbImage::from_fn(w, h, |x, y| {
            image::Rgb([
                u8::try_from(x % 256).unwrap(),
                u8::try_from(y % 256).unwrap(),
                9,
            ])
        });
        let mut out = std::io::Cursor::new(Vec::new());
        image.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    fn tile_fields(url: &str) -> Fields {
        fields(&[
            ("finish_type", json!("covering")),
            ("swatch_image", json!(url)),
            ("tile_width_cm", json!(30.0)),
            ("tile_length_cm", json!(60.0)),
        ])
    }

    /// A home with a sofa drawn with product p1's model, a wall side and a
    /// floor drawn with product t1's texture at `tile`, and a chair of the
    /// user's own.
    fn home(tile: [f64; 2]) -> Home {
        let mut doc = newera_core::Document::default();
        let (wall_id, room_id) = (doc.new_wall_id(), doc.new_room_id());
        let mut home = doc.home().clone();
        let mut sofa = Furniture {
            model: Some("products/p1/model.glb".into()),
            ..Furniture::default()
        };
        sofa.id = home.new_furniture_id();
        home.furniture.push(sofa);
        let mut chair = Furniture {
            model: Some("models/chair.glb".into()),
            ..Furniture::default()
        };
        chair.id = home.new_furniture_id();
        home.furniture.push(chair);
        let look = Material {
            image: Some("products/t1/finish.jpg".into()),
            tile: Some(tile),
            ..Material::default()
        };
        let mut wall = Wall::new(
            wall_id,
            newera_core::Point2::new(0.0, 0.0),
            newera_core::Point2::new(300.0, 0.0),
        );
        wall.left_side = Some(look.clone());
        home.walls.push(wall);
        let mut room = Room::new(
            room_id,
            "Bath",
            vec![
                newera_core::Point2::new(0.0, 0.0),
                newera_core::Point2::new(300.0, 0.0),
                newera_core::Point2::new(300.0, 200.0),
            ],
        );
        room.floor_material = Some(look);
        home.rooms.push(room);
        home
    }

    fn project(tile: [f64; 2]) -> Project {
        Project {
            variants: vec![("main".into(), home(tile))],
            active: 0,
        }
    }

    #[test]
    fn only_a_file_under_a_products_own_folder_is_a_products() {
        assert_eq!(
            super::product_of("products/p1/model.glb", "model.glb"),
            Some("p1")
        );
        assert_eq!(
            super::product_of("products/t1/finish.jpg", "finish.jpg"),
            Some("t1")
        );
        assert_eq!(
            super::product_of("products/p1/finish.jpg", "model.glb"),
            None
        );
        assert_eq!(super::product_of("products//model.glb", "model.glb"), None);
        assert_eq!(
            super::product_of("products/a/b/model.glb", "model.glb"),
            None
        );
        assert_eq!(super::product_of("products/p1model.glb", "model.glb"), None);
        assert_eq!(super::product_of("models/p1/model.glb", "model.glb"), None);
        let (models, textures) = super::held(&project([30.0, 60.0]));
        assert_eq!(
            (
                models.into_iter().collect::<Vec<_>>(),
                textures.into_iter().collect::<Vec<_>>()
            ),
            (vec!["p1".to_owned()], vec!["t1".to_owned()])
        );
    }

    /// What the project holds is compared with what the server keeps: a
    /// model by its bytes, a texture by its bytes or the form a bundle
    /// keeps of it, and the size its surfaces are drawn at.
    #[test]
    fn a_project_is_stale_where_it_holds_another_file_or_size() {
        let picture = jpeg(80, 40);
        let model = b"glTF new".to_vec();
        let kept = BTreeMap::from([
            (
                "p1".to_owned(),
                Kept {
                    fields: Fields::new(),
                    model: Some(Sha256::digest(&model).to_vec()),
                    texture: None,
                },
            ),
            (
                "t1".to_owned(),
                Kept {
                    fields: tile_fields("https://shop/t1.jpg"),
                    model: None,
                    texture: Some((picture.clone(), Some("https://shop/t1.jpg".into()))),
                },
            ),
        ]);
        let current = vec![
            ("products/p1/model.glb".to_owned(), model.clone()),
            ("products/t1/finish.jpg".to_owned(), picture.clone()),
            ("models/chair.glb".to_owned(), b"glTF mine".to_vec()),
        ];
        // The picture lies wide: the 60 cm side across.
        assert_eq!(
            super::changes(&project([60.0, 30.0]), &current, &kept),
            Changes::default()
        );

        let old = vec![
            ("products/p1/model.glb".to_owned(), b"glTF old".to_vec()),
            ("products/t1/finish.jpg".to_owned(), jpeg(40, 80)),
            ("models/chair.glb".to_owned(), b"glTF mine".to_vec()),
        ];
        assert_eq!(
            super::changes(&project([30.0, 60.0]), &old, &kept),
            Changes {
                models: vec!["p1".to_owned()],
                files: vec![("products/t1/finish.jpg".to_owned(), picture.clone())],
                tiles: vec![("products/t1/finish.jpg".to_owned(), [60.0, 30.0])],
            }
        );
        // A file missing from the bundle is not the current one either.
        assert_eq!(
            super::changes(&project([60.0, 30.0]), &Vec::new(), &kept),
            Changes {
                models: vec!["p1".to_owned()],
                files: vec![("products/t1/finish.jpg".to_owned(), picture.clone())],
                tiles: Vec::new(),
            }
        );
        // A texture made from another link than the swatch named now says
        // nothing of the size; products the server no longer has change
        // nothing.
        let mut elsewhere = kept.clone();
        elsewhere.get_mut("t1").unwrap().texture =
            Some((picture, Some("https://shop/old.jpg".into())));
        assert_eq!(
            super::changes(&project([30.0, 60.0]), &current, &elsewhere),
            Changes::default()
        );
        assert_eq!(
            super::changes(&project([30.0, 60.0]), &old, &BTreeMap::new()),
            Changes::default()
        );
    }

    /// A large texture is kept recompressed inside a bundle: that form is
    /// the current file, not a stale one.
    #[test]
    fn a_texture_the_bundle_recompressed_is_the_current_one() {
        let big = {
            let image = image::RgbImage::from_fn(1024, 1024, |x, y| {
                let n = x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503);
                image::Rgb([(n % 251) as u8, (n % 241) as u8, (n % 239) as u8])
            });
            let mut out = Vec::new();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 100)
                .encode_image(&image)
                .unwrap();
            out
        };
        let packed = newera_core::packed_asset("products/t1/finish.jpg", big.clone());
        assert_ne!(packed, big, "the bundle recompresses it");
        let kept = BTreeMap::from([(
            "t1".to_owned(),
            Kept {
                fields: tile_fields("https://shop/t1.jpg"),
                model: None,
                texture: Some((big, Some("https://shop/t1.jpg".into()))),
            },
        )]);
        let bundled = vec![("products/t1/finish.jpg".to_owned(), packed)];
        assert_eq!(
            super::changes(&project([30.0, 60.0]), &bundled, &kept),
            Changes::default()
        );
    }

    /// The rewritten bundle holds the new files, the user's own as they
    /// were, and the surfaces at the new size.
    #[test]
    fn a_rewritten_project_holds_the_new_files_at_the_new_size() {
        let picture = jpeg(80, 40);
        let bundled = vec![
            ("products/p1/model.glb".to_owned(), b"glTF old".to_vec()),
            ("products/t1/finish.jpg".to_owned(), jpeg(40, 80)),
            ("models/chair.glb".to_owned(), b"glTF mine".to_vec()),
        ];
        let changes = Changes {
            models: vec!["p1".to_owned()],
            files: vec![
                ("products/t1/finish.jpg".to_owned(), picture.clone()),
                ("products/p1/model.glb".to_owned(), b"glTF new".to_vec()),
            ],
            tiles: vec![("products/t1/finish.jpg".to_owned(), [60.0, 30.0])],
        };
        let bytes = super::rewrite(
            project([30.0, 60.0]),
            bundled,
            &changes,
            &std::env::temp_dir(),
        )
        .unwrap();
        let (back, mut files) = newera_core::project_from_bytes(&bytes).unwrap();
        files.sort();
        assert_eq!(
            files,
            vec![
                ("models/chair.glb".to_owned(), b"glTF mine".to_vec()),
                ("products/p1/model.glb".to_owned(), b"glTF new".to_vec()),
                ("products/t1/finish.jpg".to_owned(), picture),
            ]
        );
        let home = &back.variants[0].1;
        assert_eq!(
            home.walls[0].left_side.as_ref().unwrap().tile,
            Some([60.0, 30.0])
        );
        assert_eq!(
            home.rooms[0].floor_material.as_ref().unwrap().tile,
            Some([60.0, 30.0])
        );
        assert_eq!(home.furniture, super::tests::home([60.0, 30.0]).furniture);
    }
}
