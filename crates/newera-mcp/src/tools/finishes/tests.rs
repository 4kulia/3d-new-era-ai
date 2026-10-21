//! `finishes`, `edit_finishes` and `takeoff` as an agent calls them, on one
//! room: wall w1 (400 cm, 250 high) with a window f3 (100 cm wide, 90 to
//! 210 cm) in it, room r2 (400 × 300 cm) on its right, a sofa f4.

use newera_core::{
    Document, Furniture, FurnitureId, Opening, Point2, Room, RoomId, SharedDocument, Wall, WallId,
};
use serde_json::{Value, json};

fn room() -> SharedDocument {
    SharedDocument::new(Document::new(home()))
}

fn home() -> newera_core::Home {
    let mut home = newera_core::Home::default();
    let wall = Wall::new(WallId(1), Point2::new(0.0, 0.0), Point2::new(400.0, 0.0));
    let mut window = Furniture {
        id: FurnitureId(3),
        catalog: "window".into(),
        name: "Raam".into(),
        width: 100.0,
        depth: 10.0,
        height: 120.0,
        elevation: 90.0,
        opening: Some(Opening::default()),
        ..Furniture::default()
    };
    newera_core::align_to_wall(&mut window, &wall, 200.0);
    let sofa = Furniture {
        id: FurnitureId(4),
        catalog: "sofa-3".into(),
        name: "Bank".into(),
        position: Point2::new(200.0, 150.0),
        ..Furniture::default()
    };
    home.walls.push(wall);
    home.rooms.push(Room::new(
        RoomId(2),
        "Woonkamer",
        vec![
            Point2::new(0.0, 0.0),
            Point2::new(400.0, 0.0),
            Point2::new(400.0, 300.0),
            Point2::new(0.0, 300.0),
        ],
    ));
    home.furniture = vec![window, sofa];
    home
}

/// What a tool said, as text.
fn said(document: &SharedDocument, tool: &str, args: Value) -> Result<String, String> {
    let result = crate::call(document.clone(), tool, args)?;
    let text = result
        .content
        .iter()
        .filter_map(|b| b.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    if result.is_error == Some(true) {
        return Err(text);
    }
    Ok(text)
}

fn read(document: &SharedDocument, tool: &str, args: Value) -> Value {
    serde_json::from_str(&said(document, tool, args).expect("answers")).expect("json")
}

fn tile_and_paint(document: &SharedDocument) {
    assert_eq!(
        said(
            document,
            "edit_finishes",
            json!({"action": "assign", "surface": "w1:right", "product": "p-tile", "band": [0, 120], "waste": 10, "mat": "tiles #ffffff 30x60"}),
        ),
        Ok("ok rev=1".to_owned())
    );
    assert_eq!(
        said(
            document,
            "edit_finishes",
            json!({"action": "assign", "surface": "w1:right", "product": "p-paint", "band": [120, 250], "coats": 2}),
        ),
        Ok("ok rev=2".to_owned())
    );
}

#[test]
fn a_tile_band_and_paint_above_are_listed_and_measured() {
    let document = room();
    tile_and_paint(&document);
    said(
        &document,
        "update",
        json!({"items": [{"id": "f4", "product": "p-sofa"}]}),
    )
    .expect("the sofa is a product");
    assert_eq!(
        read(&document, "finishes", json!({})),
        json!({
            "columns": ["surface", "room", "product", "band", "coats", "waste"],
            "rows": [
                ["w1:right", "r2", "p-tile", [0.0, 120.0], null, 10.0],
                ["w1:right", "r2", "p-paint", [120.0, 250.0], 2, null],
            ],
        })
    );
    // 400 × 120 cm less the window's bottom 30 cm; 400 × 130 cm less its
    // top 90 cm.
    assert_eq!(
        read(&document, "takeoff", json!({})),
        json!({
            "finishes": [
                {
                    "product": "p-paint", "net_m2": 4.3, "gross_m2": 5.2, "openings_m2": 0.9,
                    "surfaces": [{
                        "surface": "w1:right", "room": "r2", "band": [120.0, 250.0], "coats": 2,
                        "gross_m2": 5.2, "openings_m2": 0.9, "net_m2": 4.3, "openings": ["f3"],
                    }],
                },
                {
                    "product": "p-tile", "net_m2": 4.5, "gross_m2": 4.8, "openings_m2": 0.3,
                    "surfaces": [{
                        "surface": "w1:right", "room": "r2", "band": [0.0, 120.0], "waste": 10.0,
                        "gross_m2": 4.8, "openings_m2": 0.3, "net_m2": 4.5, "openings": ["f3"],
                    }],
                },
            ],
            "furniture": [{"product": "p-sofa", "pieces": 1, "ids": ["f4"]}],
        })
    );
    assert_eq!(
        read(
            &document,
            "takeoff",
            json!({"product": "p-tile", "room": "r2"})
        )["finishes"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let look = document.read().home().walls[0].right_side.clone();
    assert_eq!(look, Some("tiles #ffffff 30x60".parse().unwrap()));
}

#[test]
fn removing_a_surface_clears_every_band_in_one_undo_step() {
    let document = room();
    tile_and_paint(&document);
    let both = document.read().home().clone();
    assert_eq!(
        said(
            &document,
            "edit_finishes",
            json!({"action": "remove", "surface": "w1:right"}),
        ),
        Ok("ok rev=3".to_owned())
    );
    assert_eq!(read(&document, "finishes", json!({}))["rows"], json!([]));
    said(&document, "edit_history", json!({"action": "undo"})).unwrap();
    assert_eq!(document.read().home(), &both);
    said(
        &document,
        "edit_finishes",
        json!({"action": "remove", "surface": "w1:right", "band": [0, 120]}),
    )
    .unwrap();
    assert_eq!(
        read(&document, "finishes", json!({"surface": "w1:right"}))["rows"],
        json!([["w1:right", "r2", "p-paint", [120.0, 250.0], 2, null]])
    );
}

#[test]
fn a_deleted_wall_leaves_its_finish_listed_as_gone() {
    let document = room();
    tile_and_paint(&document);
    said(&document, "delete", json!({"ids": ["w1"]})).unwrap();
    assert_eq!(
        read(&document, "finishes", json!({})),
        json!({
            "columns": ["surface", "room", "product", "band", "coats", "waste"],
            "rows": [
                ["w1:right", null, "p-tile", [0.0, 120.0], null, 10.0],
                ["w1:right", null, "p-paint", [120.0, 250.0], 2, null],
            ],
            "gone": ["w1:right"],
        })
    );
    assert_eq!(
        read(&document, "takeoff", json!({})),
        json!({
            "finishes": [],
            "furniture": [],
            "gone": [
                {"surface": "w1:right", "product": "p-tile"},
                {"surface": "w1:right", "product": "p-paint"},
            ],
        })
    );
}

#[test]
fn a_piece_kept_or_left_out_is_not_bought() {
    let document = room();
    said(
        &document,
        "update",
        json!({"items": [{"id": "f4", "product": "p-sofa", "purchase": "existing"}]}),
    )
    .unwrap();
    assert_eq!(
        document.read().home().furniture[1].info.purchase,
        Some(newera_core::Purchase::Existing)
    );
    assert_eq!(
        read(&document, "takeoff", json!({}))["furniture"],
        json!([])
    );
    said(
        &document,
        "update",
        json!({"items": [{"id": "f4", "purchase": "buy"}]}),
    )
    .unwrap();
    assert_eq!(
        read(&document, "takeoff", json!({}))["furniture"],
        json!([{"product": "p-sofa", "pieces": 1, "ids": ["f4"]}])
    );
    said(
        &document,
        "update",
        json!({"items": [{"id": "f4", "product": ""}]}),
    )
    .unwrap();
    assert_eq!(document.read().home().furniture[1].info.product, None);
    assert_eq!(
        said(
            &document,
            "update",
            json!({"items": [{"id": "f4", "purchase": "maybe"}]}),
        ),
        Err("purchase: buy, existing or excluded (not maybe)".to_owned())
    );
    assert_eq!(
        said(
            &document,
            "update",
            json!({"items": [{"id": "w1", "product": "p"}]}),
        )
        .unwrap_err()
        .split(" (allowed")
        .next(),
        Some("`product` does not apply to w1")
    );
}

#[test]
fn what_cannot_be_done_says_what_to_do_instead() {
    let document = room();
    let refusals = [
        (
            json!({"action": "assign", "surface": "r2:floor", "product": "p", "band": [0, 100]}),
            "a band is a height on a wall side, and r2:floor is not one",
        ),
        (
            json!({"action": "assign", "surface": "w1:top", "product": "p"}),
            "surface `w1:top`: r3:floor, r3:ceiling, w5:left or w5:right",
        ),
        (
            json!({"action": "assign", "surface": "w1:left"}),
            "assign: `product`, the hosted service's product id",
        ),
        (
            json!({"action": "assign", "surface": "w8:left", "product": "p"}),
            "w8 not found",
        ),
        (
            json!({"action": "remove", "surface": "w1:left"}),
            "w1:left has no finish (finishes lists them)",
        ),
        (
            json!({"action": "remove", "surface": "w1:left", "coats": 2}),
            "`coats` is for `assign`, not `remove`",
        ),
        (
            json!({"action": "list"}),
            "unknown action `list`: assign, remove (to read, use finishes)",
        ),
    ];
    for (args, why) in refusals {
        assert_eq!(
            said(&document, "edit_finishes", args.clone()),
            Err(why.to_owned()),
            "{args}"
        );
    }
    assert_eq!(
        said(&document, "takeoff", json!({"room": "r9"})),
        Err("r9 not found (home lists the rooms)".to_owned())
    );
    assert_eq!(document.read().revision(), 0, "nothing was written");
}

#[test]
fn a_floor_and_a_ceiling_take_their_look_and_a_storey_is_named() {
    let mut home = home();
    home.levels.push(newera_core::Level {
        id: newera_core::LevelId(9),
        name: "Begane grond".into(),
        ..newera_core::Level::default()
    });
    // A part of a group is a product of its own.
    let mut group = Furniture {
        id: FurnitureId(5),
        catalog: "group".into(),
        position: Point2::new(100.0, 100.0),
        ..Furniture::default()
    };
    group.children.push(Furniture {
        id: FurnitureId(6),
        catalog: "chair".into(),
        position: Point2::new(100.0, 100.0),
        ..Furniture::default()
    });
    home.furniture.push(group);
    let document = SharedDocument::new(Document::new(home));
    for (surface, mat) in [
        ("r2:floor", "#c8b89a"),
        ("r2:ceiling", "#ffffff"),
        ("w1:left", "#102030"),
    ] {
        said(
            &document,
            "edit_finishes",
            json!({"action": "assign", "surface": surface, "product": "p", "mat": mat}),
        )
        .unwrap();
    }
    said(
        &document,
        "update",
        json!({"items": [{"id": "f6", "product": "p-chair", "purchase": "buy"}]}),
    )
    .unwrap();
    let doc = document.read();
    let (room, wall) = (&doc.home().rooms[0], &doc.home().walls[0]);
    assert_eq!(room.floor_material, Some("#c8b89a".parse().unwrap()));
    assert_eq!(room.ceiling_material, Some("#ffffff".parse().unwrap()));
    assert_eq!(wall.left_side, Some("#102030".parse().unwrap()));
    assert_eq!(wall.right_side, None);
    drop(doc);
    let taken = read(&document, "takeoff", json!({}));
    assert_eq!(
        taken["finishes"][0]["surfaces"][1],
        json!({"surface": "r2:floor", "level": "lv9", "room": "r2", "gross_m2": 12.0, "openings_m2": 0.0, "net_m2": 12.0})
    );
    assert_eq!(
        taken["furniture"],
        json!([{"product": "p-chair", "pieces": 1, "ids": ["f6"]}])
    );
}
