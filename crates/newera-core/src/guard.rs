//! Balcony guards and closures, told apart: a bar railing, a glass
//! balustrade under a handrail, the retractable glass closure, and the
//! common São Paulo case of both a railing and a closure.
//!
//! NBR 14718:2019 (guarda-corpos), NBR 7199:2016 (glass in buildings) and
//! NBR 16259:2014 (envidraçamento de sacadas), read in their texts.

use crate::electrical::{Finding, Severity};
use crate::furniture::Furniture;
use crate::geometry::Point2;
use crate::home::Home;
use crate::say;

/// Where a glass piece keeps its glass: `laminated`, `tempered`,
/// `tempered-laminated`, `wired`.
pub const GLASS_KEY: &str = "glass:type";
/// Where a bar railing keeps its clear gap between bars, cm.
pub const GAP_KEY: &str = "guard:gap_cm";

/// What a piece at a balcony's edge is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Guard {
    /// Bars of steel, iron or aluminium.
    Railing,
    /// Glass infill under a handrail: a guard of glass.
    GlassRailing,
    /// Retractable glass leaves: a closure, never a guard.
    Glazing,
}

impl Guard {
    /// Its name as a word of a sentence, translated with it.
    #[must_use]
    pub fn said(self) -> crate::text::Text {
        crate::text::Text::from(self.name())
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Railing => "gradil",
            Self::GlassRailing => "guarda-corpo de vidro",
            Self::Glazing => "fechamento de vidro da sacada",
        }
    }
}

/// What a piece is, by its catalog, else its name.
pub fn guard_of(piece: &Furniture) -> Option<Guard> {
    match piece.catalog.as_str() {
        "railing" => return Some(Guard::Railing),
        "glass-railing" => return Some(Guard::GlassRailing),
        "balcony-glazing" => return Some(Guard::Glazing),
        _ => {}
    }
    // By name, only a piece of a guard's size that no catalog or project
    // says is something else: a grille named after the closure is a grille.
    if piece.discipline.is_some()
        || !matches!(
            piece.catalog.as_str(),
            "imported" | "box" | "group" | "" | "panel" | "solid"
        )
        || piece.width.max(piece.depth) < 60.0
        || piece.height < 60.0
    {
        return None;
    }
    let name = crate::annotations::fold(&piece.name);
    let has = |words: &[&str]| words.iter().any(|w| name.contains(w));
    // Portuguese, then English: a piece imported from an American library
    // is a guard by the same token.
    if has(&[
        "envidracamento",
        "fechamento de vidro",
        "fechamento da varanda",
        "fechamento da sacada",
        "cortina de vidro",
        "balcony glazing",
        "glass enclosure",
    ]) {
        Some(Guard::Glazing)
    } else if has(&[
        "guarda-corpo de vidro",
        "guarda corpo de vidro",
        "glass railing",
        "glass balustrade",
        "glass guardrail",
    ]) {
        Some(Guard::GlassRailing)
    } else if has(&[
        "gradil",
        "guarda-corpo",
        "guarda corpo",
        "grade da sacada",
        "grade da varanda",
        "parapeito metalico",
        "railing",
        "guardrail",
        "balustrade",
    ]) {
        Some(Guard::Railing)
    } else {
        None
    }
}

fn distance_to_segment(p: Point2, a: Point2, b: Point2) -> f64 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = (dx * dx + dy * dy).max(1e-9);
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
    Point2::new(a.x + t * dx, a.y + t * dy).distance(p)
}

/// The line a piece runs along, its two ends.
fn line_of(f: &Furniture) -> (Point2, Point2) {
    (
        f.to_plan((-f.width / 2.0, 0.0)),
        f.to_plan((f.width / 2.0, 0.0)),
    )
}

/// What the guards and closures of the storey shown ask, weighed at the
/// place the project is — see [`crate::electrical::weighed`].
pub fn check(home: &Home) -> Vec<Finding> {
    crate::electrical::weighed(home, found(home))
}

fn found(home: &Home) -> Vec<Finding> {
    let view = home.level_view(home.current_level());
    let pieces: Vec<(&Furniture, Guard)> = view
        .furniture
        .iter()
        .flat_map(Furniture::flatten)
        .filter_map(|f| guard_of(f).map(|g| (f, g)))
        .collect();
    let mut out = Vec::new();
    for (f, guard) in &pieces {
        let place = format!("{} {}", f.name, f.id);
        let (_, top) = f.height_range();
        if matches!(guard, Guard::Railing | Guard::GlassRailing) && top + 0.5 < 110.0 {
            out.push(Finding {
                key: format!("guard:height:{}", f.id),
                accepted: None,
                severity: Severity::Erro,
                place: place.clone(),
                message: say!("{} com {} cm: o guarda-corpo tem ao menos 1,10 m do piso ao topo do corrimão; sobre mureta, também 0,90 m acima dela.", guard.said(), top.round()),
                source: "nbr14718",
            });
        }
        if *guard == Guard::Railing
            && let Some(gap) = f
                .properties
                .get(GAP_KEY)
                .and_then(|v| v.replace(',', ".").parse::<f64>().ok())
            && gap > 11.0
        {
            out.push(Finding {
                key: format!("guard:gap:{}", f.id),
                accepted: None,
                severity: Severity::Erro,
                place: place.clone(),
                message: say!("Vão de {} cm entre as barras: o máximo é 11 cm, e até 45 cm do piso nada em que se possa apoiar o pé para escalar.", crate::electrical::decimal(gap)),
                source: "nbr14718",
            });
        }
        let glass = f.properties.get(GLASS_KEY).map(String::as_str);
        if *guard == Guard::GlassRailing {
            match glass {
                Some("laminated" | "tempered-laminated" | "wired") => {}
                Some(other) => out.push(Finding {
                    key: format!("guard:glass:{}", f.id),
                    accepted: None,
                    severity: Severity::Erro,
                    place: place.clone(),
                    message: say!("Guarda-corpo em vidro {}: a norma pede vidro laminado de segurança (classe 1), aramado ou insulado feito deles; temperado sozinho, ao quebrar, deixa o vão aberto.", other),
                    source: "nbr7199",
                }),
                None => out.push(Finding {
                    key: format!("guard:glass:{}", f.id),
                    accepted: None,
                    severity: Severity::Dica,
                    place: place.clone(),
                    message: "Diga o vidro do guarda-corpo (glass:type): ele precisa ser laminado de segurança, em geral 4+4 ou 5+5 mm calculado pela norma.".into(),
                    source: "nbr7199",
                }),
            }
        }
        if *guard == Guard::Glazing {
            if let Some(g) = glass
                && !matches!(g, "laminated" | "tempered" | "tempered-laminated")
            {
                out.push(Finding {
                    key: format!("guard:glazing-glass:{}", f.id),
                    accepted: None,
                    severity: Severity::Erro,
                    place: place.clone(),
                    message: say!("Fechamento de sacada em vidro {}: pede vidro de segurança temperado ou laminado, com espessura calculada pelo vento do local.", g),
                    source: "nbr16259",
                });
            }
            // The closure is no guard: a railing or glass guard must stand along
            // it, or the parapet wall it sits on.
            let (a, b) = line_of(f);
            let guarded = pieces.iter().any(|(g, kind)| {
                matches!(kind, Guard::Railing | Guard::GlassRailing) && {
                    let (c, d) = line_of(g);
                    let mid = Point2::new(f64::midpoint(c.x, d.x), f64::midpoint(c.y, d.y));
                    distance_to_segment(mid, a, b) <= 40.0
                }
            });
            let mid = Point2::new(f64::midpoint(a.x, b.x), f64::midpoint(a.y, b.y));
            let parapet = view
                .walls
                .iter()
                .filter(|w| {
                    !w.is_arc()
                        && distance_to_segment(mid, w.start, w.end) <= w.thickness / 2.0 + 25.0
                })
                .map(|w| w.height.min(w.height_at_end.unwrap_or(w.height)))
                .filter(|h| *h + 5.0 >= f.elevation && f.elevation >= 60.0)
                .reduce(f64::max);
            if let Some(height) = parapet
                && !guarded
                && height + 0.5 < 110.0
            {
                out.push(Finding {
                    key: format!("guard:parapet:{}", f.id),
                    accepted: None,
                    severity: Severity::Erro,
                    place: place.clone(),
                    message: say!("O fechamento de vidro está sobre uma mureta de {} cm: sem gradil, a mureta é o guarda-corpo e pede 1,10 m; o vidro não conta.", height.round()),
                    source: "nbr14718",
                });
            }
            if !guarded && parapet.is_none() {
                out.push(Finding {
                    key: format!("guard:glazing-alone:{}", f.id),
                    accepted: None,
                    severity: Severity::Erro,
                    place: place.clone(),
                    message: "Fechamento de vidro sem guarda-corpo junto: o envidraçamento não faz a função de guarda-corpo; a sacada precisa do gradil ou do guarda-corpo de vidro, com 1,10 m, atrás ou à frente dele.".into(),
                    source: "nbr16259",
                });
            }
        }
    }
    for finding in &mut out {
        finding.accepted = home.accepted.get(&finding.key).cloned();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::FurnitureId;

    fn piece(id: u64, catalog: &str, name: &str, y: f64, height: f64) -> Furniture {
        Furniture {
            id: FurnitureId(id),
            catalog: catalog.into(),
            name: name.into(),
            position: Point2::new(150.0, y),
            width: 300.0,
            depth: 5.0,
            height,
            ..Furniture::default()
        }
    }

    fn keys(home: &Home) -> Vec<String> {
        check(home).into_iter().map(|f| f.key).collect()
    }

    #[test]
    fn the_four_balconies_are_told_apart_and_checked_as_what_they_are() {
        assert_eq!(
            guard_of(&piece(
                1,
                "imported",
                "Gradil de ferro da sacada",
                0.0,
                110.0
            )),
            Some(Guard::Railing)
        );
        assert_eq!(
            guard_of(&piece(2, "imported", "Guarda-corpo de vidro", 0.0, 110.0)),
            Some(Guard::GlassRailing)
        );
        assert_eq!(
            guard_of(&piece(3, "imported", "Fechamento da varanda", 0.0, 240.0)),
            Some(Guard::Glazing)
        );

        // (a) bars only: height and gap.
        let mut bars = Home::default();
        let mut railing = piece(10, "railing", "Gradil", 0.0, 100.0);
        railing.properties.insert(GAP_KEY.into(), "13".into());
        bars.furniture.push(railing);
        let k = keys(&bars);
        assert!(
            k.contains(&"guard:height:f10".to_owned()) && k.contains(&"guard:gap:f10".to_owned()),
            "{k:?}"
        );

        // (b) glass guard: laminated, never plain tempered.
        let mut glass = Home::default();
        let mut balustrade = piece(20, "glass-railing", "Guarda-corpo de vidro", 0.0, 110.0);
        balustrade
            .properties
            .insert(GLASS_KEY.into(), "tempered".into());
        glass.furniture.push(balustrade);
        let findings = check(&glass);
        assert!(
            findings
                .iter()
                .any(|f| f.key == "guard:glass:f20" && f.severity == Severity::Erro),
            "{findings:?}"
        );
        glass.furniture[0]
            .properties
            .insert(GLASS_KEY.into(), "laminated".into());
        assert_eq!(keys(&glass), [] as [std::string::String; 0]);

        // (c) glass closure alone: no guard.
        let mut closed = Home::default();
        closed
            .furniture
            .push(piece(30, "balcony-glazing", "Envidraçamento", 0.0, 240.0));
        assert!(keys(&closed).contains(&"guard:glazing-alone:f30".to_owned()));

        // A grille named after the closure is a grille, not a closure.
        let mut grille = piece(
            40,
            "vent-grille",
            "GV-01 — grelha, fechamento da varanda",
            0.0,
            15.0,
        );
        grille.width = 20.0;
        assert_eq!(guard_of(&grille), None);

        // A closure on a parapet wall: the wall is the guard, and it needs 1,10 m.
        let mut on_wall = Home::default();
        let mut wall = crate::elements::Wall::new(
            crate::ids::WallId(1),
            Point2::new(0.0, 0.0),
            Point2::new(300.0, 0.0),
        );
        wall.height = 105.0;
        on_wall.walls.push(wall);
        let mut glazing = piece(50, "imported", "Fechamento da varanda", 0.0, 155.0);
        glazing.elevation = 105.0;
        on_wall.furniture.push(glazing);
        let k = keys(&on_wall);
        assert!(
            k.contains(&"guard:parapet:f50".to_owned())
                && !k.contains(&"guard:glazing-alone:f50".to_owned()),
            "{k:?}"
        );
        on_wall.walls[0].height = 110.0;
        assert_eq!(keys(&on_wall), [] as [std::string::String; 0]);

        // (d) the common case: a railing and the closure beside it.
        closed
            .furniture
            .push(piece(31, "railing", "Gradil", 12.0, 110.0));
        assert!(keys(&closed).is_empty(), "{:?}", keys(&closed));
    }
}
