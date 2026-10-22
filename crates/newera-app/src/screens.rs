//! Headless screenshots of the real UI for visual review:
//! `cargo test -p newera-app screens -- --ignored` writes `target/screenshots/*.png`.

use egui_kittest::Harness;
use newera_core::{Command, Document, Point2, SharedDocument, Wall};

use crate::app::NewEraApp;
use crate::dialogs::Dialog;

fn house(doc: &mut Document, width: f64) {
    let pts = [(0.0, 0.0), (width, 0.0), (width, 500.0), (0.0, 500.0)];
    let commands = (0..4)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % 4]);
            Command::insert(Wall::new(
                doc.new_wall_id(),
                Point2::new(a.0, a.1),
                Point2::new(b.0, b.1),
            ))
        })
        .collect();
    doc.execute(Command::Batch { commands }).unwrap();
    let room =
        newera_core::detect_room(&doc.home().walls, Point2::new(width / 2.0, 250.0)).unwrap();
    let room = newera_core::Room::new(doc.new_room_id(), "Sala", room);
    doc.execute(Command::insert(room)).unwrap();
}

pub(crate) fn render(name: &str, document: SharedDocument, setup: impl FnOnce(&mut NewEraApp)) {
    render_in(name, crate::theme::Mode::System, document, setup);
}

/// The same, in the theme asked for instead of the one the machine is set to.
pub(crate) fn render_in(
    name: &str,
    mode: crate::theme::Mode,
    document: SharedDocument,
    setup: impl FnOnce(&mut NewEraApp),
) {
    let mut harness = Harness::builder()
        .with_size(eframe::egui::vec2(1400.0, 860.0))
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| NewEraApp::new(cc, document, None));
    crate::theme::set_mode(&harness.ctx, mode);
    harness.run_steps(5);
    setup(harness.state_mut());
    harness.run_steps(12);
    let image = harness.render().expect("render");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
    std::fs::create_dir_all(&dir).unwrap();
    image.save(dir.join(format!("{name}.png"))).unwrap();
}

#[test]
#[ignore = "visual review; needs a GPU"]
fn variants() {
    let mut doc = Document::default();
    house(&mut doc, 600.0);
    doc.add_variant(Some("Sala ampliada".into()), false);
    house(&mut doc, 800.0);
    doc.add_variant(Some("Opção compacta".into()), false);
    house(&mut doc, 450.0);
    render("variants-tabs", SharedDocument::new(doc), |_| {});

    let mut doc = Document::default();
    house(&mut doc, 600.0);
    doc.add_variant(Some("Sala ampliada".into()), false);
    house(&mut doc, 800.0);
    render(
        "variants-compare",
        SharedDocument::new(doc),
        NewEraApp::open_compare,
    );
    let _ = Dialog::Help;
}

#[test]
#[ignore = "visual review; needs a GPU"]
fn levels() {
    let mut doc = Document::default();
    house(&mut doc, 700.0);
    newera_core::ops::add_level(&mut doc, Some("Superior".into()), None).unwrap();
    house(&mut doc, 450.0);
    let stairs = newera_catalog::find("stairs")
        .expect("stairs in catalog")
        .instantiate(doc.new_furniture_id(), Point2::new(300.0, 250.0));
    let ground = doc.home().base_level();
    let mut stairs = stairs;
    stairs.level = ground;
    doc.execute(Command::insert(stairs)).unwrap();
    render("levels", SharedDocument::new(doc), |_| {});
}

#[test]
#[ignore = "visual review; needs a GPU"]
fn materials() {
    use newera_core::{Element, Material};
    let mut doc = Document::default();
    house(&mut doc, 700.0);
    let mut home = doc.home().clone();
    let finishes = ["brick", "#9fb8c8", "subway 20x10", "stone"];
    for (wall, finish) in home.walls.iter_mut().zip(finishes) {
        wall.right_side = Some(finish.parse().unwrap());
        wall.left_side = Some(finish.parse().unwrap());
        wall.apply_type(newera_core::wall_type("tijolo-14").unwrap());
    }
    home.rooms[0].floor_material = Some("wood".parse::<Material>().unwrap());
    let commands = home
        .walls
        .iter()
        .cloned()
        .map(Element::Wall)
        .chain(home.rooms.iter().cloned().map(Element::Room))
        .map(|element| Command::Update { element })
        .collect();
    doc.execute(Command::Batch { commands }).unwrap();
    let shared = SharedDocument::new(doc);
    render("materials", shared.clone(), |_| {});
    render("materials-wall-dialog", shared.clone(), |app| {
        app.open_modify(&[newera_core::WallId(1).into()]);
    });
    render("materials-room-dialog", shared, |app| {
        app.open_modify(&[newera_core::RoomId(5).into()]);
    });
}

/// The real Sweet Home 3D project given by `NEWERA_SH3D_SAMPLE`.
#[test]
#[ignore = "visual review; needs a GPU and NEWERA_SH3D_SAMPLE"]
fn sh3d_import() {
    let path = std::env::var("NEWERA_SH3D_SAMPLE").expect("NEWERA_SH3D_SAMPLE");
    let mut doc = Document::default();
    let opened = newera_sh3d::open_file(&mut doc, std::path::Path::new(&path)).unwrap();
    assert!(opened.warnings.is_empty(), "{:?}", opened.warnings);
    let shared = SharedDocument::new(doc);
    render("sh3d-import", shared.clone(), |_| {});
    let cameras = shared.read().home().cameras.stored.clone();
    for (i, camera) in cameras.into_iter().take(3).enumerate() {
        render(&format!("sh3d-camera{i}"), shared.clone(), |app| {
            app.scene.visitor = Some(crate::view::scene::Visitor { camera });
        });
    }
    let levels: Vec<_> = shared
        .read()
        .home()
        .sorted_levels()
        .iter()
        .map(|l| l.id)
        .collect();
    for hour in [8.0, 16.0] {
        render(&format!("sh3d-sun-{hour}h"), shared.clone(), |app| {
            app.scene.sun_hour = Some(hour);
            app.scene.visitor = None;
        });
    }
    for (i, level) in levels.into_iter().enumerate() {
        shared.write().select_level(Some(level));
        render(&format!("sh3d-import-level{i}"), shared.clone(), |_| {});
    }
}

#[test]
#[ignore = "visual review; needs a GPU"]
fn disciplines() {
    use newera_core::Discipline;
    let mut doc = Document::default();
    house(&mut doc, 700.0);
    let place = |doc: &mut Document, cat: &str, x: f64, y: f64| {
        let piece = newera_catalog::find(cat)
            .unwrap()
            .instantiate(doc.new_furniture_id(), Point2::new(x, y));
        doc.execute(Command::insert(piece)).unwrap();
    };
    doc.set_active_discipline(Some(Discipline::Plumbing));
    for (cat, x, y) in [
        ("cold-water", 620.0, 60.0),
        ("hot-water", 660.0, 60.0),
        ("sewer", 640.0, 120.0),
        ("floor-drain", 560.0, 420.0),
        ("valve", 600.0, 20.0),
    ] {
        place(&mut doc, cat, x, y);
    }
    let mut pipe = newera_core::Polyline::new(
        doc.new_polyline_id(),
        vec![
            Point2::new(600.0, 20.0),
            Point2::new(620.0, 60.0),
            Point2::new(660.0, 60.0),
        ],
    );
    pipe.color = [30, 110, 200];
    pipe.thickness = 2.0;
    doc.execute(Command::insert(pipe)).unwrap();
    doc.set_active_discipline(Some(Discipline::Electrical));
    for (cat, x, y) in [
        ("outlet-low", 100.0, 20.0),
        ("outlet-mid", 300.0, 20.0),
        ("outlet-high", 500.0, 20.0),
        ("switch", 40.0, 250.0),
        ("switch-double", 40.0, 320.0),
        ("light-ceiling", 350.0, 250.0),
        ("light-wall", 680.0, 250.0),
        ("electrical-panel", 40.0, 450.0),
        ("ac-point", 350.0, 480.0),
        ("data-outlet", 200.0, 480.0),
    ] {
        place(&mut doc, cat, x, y);
    }
    let mut run = newera_core::Polyline::new(
        doc.new_polyline_id(),
        vec![
            Point2::new(40.0, 250.0),
            Point2::new(350.0, 250.0),
            Point2::new(680.0, 250.0),
        ],
    );
    run.color = [214, 96, 20];
    run.thickness = 1.5;
    run.dash = newera_core::DashStyle::Dash;
    run.join = newera_core::LineJoin::Curved;
    doc.execute(Command::insert(run)).unwrap();
    render("disciplines-electrical", SharedDocument::new(doc), |_| {});
}

/// One window per language the app speaks, the same house in all of them:
/// side by side, a label left in Portuguese stands out.
#[test]
#[ignore = "visual review; needs a GPU"]
fn every_interface_language() {
    for lang in crate::i18n::Lang::ALL {
        crate::i18n::set(lang);
        let mut doc = Document::default();
        house(&mut doc, 600.0);
        let name = format!("language-{}", lang.tag());
        render(&name, SharedDocument::new(doc), |app| {
            crate::i18n::set(lang);
            app.open_modify(&[newera_core::WallId(1).into()]);
        });
    }
    crate::i18n::set(crate::i18n::Lang::Pt);
}

/// The panel that tells someone how to point their AI at this window, with an
/// agent already at work in it.
#[test]
#[ignore = "visual review; needs a GPU"]
fn connect_ai() {
    let mut doc = Document::default();
    house(&mut doc, 600.0);
    let now = newera_core::collab::now_ms();
    doc.agents_mut()
        .hello(Some("s1"), "claude-code", Some("2.0.0"), now - 95_000);
    doc.agents_mut().called(Some("s1"), "create", now - 40_000);
    doc.agents_mut().called(Some("s1"), "place", now - 9_000);
    doc.agents_mut()
        .called(Some("s1"), "render_photo", now - 1_000);
    render_in(
        "connect-ai",
        crate::theme::Mode::Night,
        SharedDocument::new(doc),
        |app| {
            app.mcp_url = Some("http://127.0.0.1:7878/mcp".to_owned());
            app.dialog = Some(Dialog::ConnectAi { client: 0 });
        },
    );
}

/// The window as the site shows it: the demo home the browser opens on, in
/// the dark palette, with the plan and the 3D as a visitor first meets them.
#[test]
#[ignore = "visual review; needs a GPU"]
fn site_shot() {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/demo.newera"),
    )
    .expect("the demo home ships with the browser editor");
    crate::i18n::set(crate::i18n::Lang::Pt);
    let mut harness = egui_kittest::Harness::builder()
        .with_size(eframe::egui::vec2(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(|cc| NewEraApp::new(cc, SharedDocument::default(), None));
    crate::theme::set_mode(&harness.ctx, crate::theme::Mode::Night);
    harness.run_steps(5);
    harness.state_mut().open_bytes("demo.newera", &bytes);
    harness.run_steps(24);
    let image = harness.render().expect("render");
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
    std::fs::create_dir_all(&dir).unwrap();
    image.save(dir.join("site-editor.png")).unwrap();
}

/// The same panel before anybody has connected: the state nearly every
/// visitor meets first, and the one the button in the bar is loud about.
#[test]
#[ignore = "visual review; needs a GPU"]
fn connect_ai_waiting() {
    let mut doc = Document::default();
    house(&mut doc, 600.0);
    render_in(
        "connect-ai-waiting",
        crate::theme::Mode::Night,
        SharedDocument::new(doc),
        |app| {
            app.mcp_url = Some("http://127.0.0.1:7878/mcp".to_owned());
            app.dialog = Some(Dialog::ConnectAi { client: 0 });
        },
    );
}

/// The window as the press kit shows it: a furnished home, in English, with
/// nothing open over it.
#[test]
#[ignore = "visual review; needs a GPU"]
fn press_shot() {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../web/demo.newera"),
    )
    .expect("the demo home ships with the browser editor");
    let again = bytes.clone();
    crate::i18n::set(crate::i18n::Lang::En);
    render_in(
        "press-editor",
        crate::theme::Mode::Night,
        SharedDocument::default(),
        move |app| {
            crate::i18n::set(crate::i18n::Lang::En);
            app.open_bytes("demo.newera", &bytes);
            dress(app);
        },
    );
    // And the same home by day, in another language: the palette and the
    // wording both have to hold up.
    let bytes = again;
    crate::i18n::set(crate::i18n::Lang::Es);
    render_in(
        "press-day",
        crate::theme::Mode::Day,
        SharedDocument::default(),
        move |app| {
            crate::i18n::set(crate::i18n::Lang::Es);
            app.open_bytes("demo.newera", &bytes);
            dress(app);
        },
    );
    crate::i18n::set(crate::i18n::Lang::Pt);
}

/// The press shot is the product's face, so the demo gets what a real project
/// would have: floors and walls with a finish, and the 3D standing in the
/// living room instead of hovering over a lawn.
fn dress(app: &mut NewEraApp) {
    use newera_core::{Camera, Element, Material};
    let mut home = app.document.read().home().clone();
    for wall in &mut home.walls {
        wall.left_side = Some("#efe7d8".parse::<Material>().unwrap());
        wall.right_side = Some("#efe7d8".parse::<Material>().unwrap());
    }
    // Standing with your back to the television, looking down the living room
    // at the sofa — eyes at 1.6 m, barely tilted down.
    let eye = Camera {
        x: 230.0,
        y: 95.0,
        z: 185.0,
        yaw: 0.0,
        pitch: 16.0,
        fov: 75.0,
        ..Camera::default()
    };
    let commands = home
        .walls
        .iter()
        .cloned()
        .map(Element::Wall)
        .map(|element| Command::Update { element })
        .collect();
    app.run(|doc| doc.execute(Command::Batch { commands }));
    app.scene.visitor = Some(crate::view::scene::Visitor { camera: eye });
    app.plan.request_fit();
}

/// The same window in daylight: the palette has to hold up on paper too.
#[test]
#[ignore = "visual review; needs a GPU"]
fn the_day_theme() {
    let mut doc = Document::default();
    house(&mut doc, 600.0);
    render_in(
        "theme-day",
        crate::theme::Mode::Day,
        SharedDocument::new(doc),
        |_| {},
    );
}

#[test]
#[ignore = "visual review; needs a GPU"]
fn collaborators() {
    let mut doc = Document::default();
    house(&mut doc, 600.0);
    let now = newera_core::collab::now_ms();
    for (name, at) in [("Ana", (150.0, 120.0)), ("Agente IA", (420.0, 330.0))] {
        let session = doc.sessions_mut().join(name, now);
        doc.sessions_mut().update(
            &session.id,
            newera_core::collab::Presence {
                cursor: Some(Point2::new(at.0, at.1)),
                ..Default::default()
            },
            now,
        );
    }
    render("collaborators", SharedDocument::new(doc), |_| {});
}

#[test]
#[ignore = "visual review; needs a GPU"]
fn references() {
    use newera_core::{PlanAnnotations, Polyline};
    let mut doc = Document::default();
    house(&mut doc, 800.0);
    doc.execute(Command::remove(doc.home().rooms[0].id))
        .unwrap();
    let mut line = Polyline::new(
        doc.new_polyline_id(),
        vec![Point2::new(450.0, 0.0), Point2::new(450.0, 500.0)],
    );
    line.room_divider = true;
    line.dash = newera_core::DashStyle::Dash;
    line.color = [120, 120, 120];
    doc.execute(Command::insert(line)).unwrap();
    for (name, x) in [("Estar", 200.0), ("Jantar", 620.0)] {
        let home = doc.home();
        let dividers: Vec<&Polyline> = home.polylines.iter().collect();
        let points =
            newera_core::detect_room_with_dividers(&home.walls, &dividers, Point2::new(x, 250.0))
                .unwrap();
        let mut room = newera_core::Room::new(doc.new_room_id(), name, points);
        room.auto = true;
        doc.execute(Command::insert(room)).unwrap();
    }
    for (cat, x, y) in [
        ("sofa-3", 200.0, 380.0),
        ("coffee-table", 200.0, 250.0),
        ("armchair", 80.0, 120.0),
        ("dining-table-4", 620.0, 250.0),
        ("plant", 760.0, 60.0),
    ] {
        let piece = newera_catalog::find(cat)
            .unwrap()
            .instantiate(doc.new_furniture_id(), Point2::new(x, y));
        doc.execute(Command::insert(piece)).unwrap();
    }
    doc.execute(Command::SetAnnotations {
        annotations: PlanAnnotations {
            references: true,
            ..PlanAnnotations::default()
        },
    })
    .unwrap();
    let shared = SharedDocument::new(doc);
    render("references", shared.clone(), |app| app.plan.request_fit());
    // Move the divider: rooms, tags and schedule follow.
    let id = shared.read().home().polylines[0].id;
    newera_core::ops::translate(&mut shared.write(), &[id.into()], 120.0, 0.0, true).unwrap();
    render("references-moved", shared, |app| app.plan.request_fit());
}

#[test]
#[ignore = "visual review; needs a GPU"]
fn progress_window() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut doc = Document::default();
    house(&mut doc, 700.0);
    let hold = Arc::new(AtomicBool::new(true));
    let waiting = Arc::clone(&hold);
    render("progress", SharedDocument::new(doc), move |app| {
        crate::jobs::start(
            app,
            "Abrindo projeto",
            "apartamento-eleva.newera".to_owned(),
            move || {
                newera_core::progress::step("Extraindo imagens e modelos", 34, 120);
                while waiting.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Box::new(|_: &mut NewEraApp| {})
            },
        );
    });
    hold.store(false, Ordering::Relaxed);
}

/// A shop finish's texture replaced under the same path — the hosted
/// service swapping in a new swatch — shows in the open 3D view without an
/// edit: red tiles first, blue ones after. Writes both screenshots.
#[test]
#[ignore = "visual review; needs a GPU"]
fn swapped_finish_texture() {
    use newera_core::{Element, Material};
    let picture = |rgb: [u8; 3]| {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::from_fn(64, 32, |x, y| {
            if (x < 32) == (y < 16) {
                image::Rgb(rgb)
            } else {
                image::Rgb([240, 240, 240])
            }
        })
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .unwrap();
        out.into_inner()
    };
    let dir = std::path::PathBuf::from("/memory/swapped-finish-texture");
    let path = "products/p1/finish.jpg";
    newera_core::vfs::mount(&dir, [(path.to_owned(), picture([200, 30, 30]))]);
    let mut doc = Document::default();
    house(&mut doc, 700.0);
    doc.set_asset_dir(Some(dir.clone()));
    let mut home = doc.home().clone();
    for wall in &mut home.walls {
        let look = Material {
            image: Some(path.to_owned()),
            tile: Some([60.0, 30.0]),
            ..Material::default()
        };
        wall.left_side = Some(look.clone());
        wall.right_side = Some(look);
    }
    let commands = home
        .walls
        .iter()
        .cloned()
        .map(Element::Wall)
        .map(|element| Command::Update { element })
        .collect();
    doc.execute(Command::Batch { commands }).unwrap();
    let mut harness = Harness::builder()
        .with_size(eframe::egui::vec2(1400.0, 860.0))
        .with_step_dt(1.0 / 60.0)
        .wgpu()
        .build_eframe(move |cc| NewEraApp::new(cc, SharedDocument::new(doc), None));
    harness.run_steps(17);
    let before = harness.render().expect("render");
    newera_core::vfs::mount(&dir, [(path.to_owned(), picture([30, 30, 200]))]);
    harness.run_steps(12);
    let after = harness.render().expect("render");
    let shots = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/screenshots");
    std::fs::create_dir_all(&shots).unwrap();
    before
        .save(shots.join("swapped-finish-texture-before.png"))
        .unwrap();
    after
        .save(shots.join("swapped-finish-texture-after.png"))
        .unwrap();
    newera_core::vfs::unmount(&dir);
    // The 3D view, below the plan and left of its axes.
    let count = |image: &image::RgbaImage, red: bool| {
        image::imageops::crop_imm(image, 280, 480, 1000, 340)
            .to_image()
            .pixels()
            .filter(|p| {
                let [r, g, b, _] = p.0;
                let (hot, cold) = if red { (r, b.max(g)) } else { (b, r.max(g)) };
                i32::from(hot) - i32::from(cold) > 80
            })
            .count()
    };
    let counts = [
        count(&before, true),
        count(&before, false),
        count(&after, true),
        count(&after, false),
    ];
    assert!(
        counts[0] > 1000 && counts[1] < 10,
        "red tiles first: {counts:?}"
    );
    assert!(
        counts[3] > 1000 && counts[2] < 10,
        "blue ones after: {counts:?}"
    );
}
