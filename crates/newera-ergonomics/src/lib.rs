//! Ergonomics and habitability review.
//!
//! Nothing here blocks anything. A review reads a drawing and says what it
//! finds; the drawing is the user's, and so is the decision. Somebody laying
//! out a house to learn, to try an idea or to see what it would look like is
//! not stopped by a gas standard, and neither is somebody who knows exactly
//! what they are doing and means it. Findings that were looked at and settled
//! can be accepted by key — see [`Finding::accepted`] — so that a plan which
//! is right reaches zero without anything being swept away.
//!
//! Given who lives in a home — how many people, children, elderly, a
//! wheelchair user — this reads the plan the way a careful architect would:
//! is there room to walk beside the bed and in front of the stove, do the
//! beds, seats, bathrooms and wardrobes serve everyone, does the kitchen
//! work, can a wheelchair turn in the bathroom. Each finding says what is
//! wrong with its numbers, what to change, and which reference it follows.
//!
//! References are Brazilian where one exists and they travel as data, not as
//! prose: a finding carries the short code of a [`newera_core::Standard`], and
//! the report resolves the codes it used once. That is what lets the reply say
//! which edition it followed, lets the interface link it, and lets the
//! reliability tier decide how much a finding may accuse — a survey never
//! accuses, and a figure we could not confirm at the source may warn but never
//! errs. Municipal codes vary by city: give [`Profile::city`] and they judge,
//! omit it and they only advise.

// Counts of people and pieces are tiny; cm fit in any float.
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]

mod corners;
mod rule;
mod scene;

use crate::rule::Rule;
use newera_core::say;
use newera_core::standards::{self, Resolved, Standard};
use newera_core::text::Text;
use newera_core::{Home, OpeningKind, Point2};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use scene::{RoomUse, Scene, Side, Space, Unit, Use};

/// Disciplines included in the habitability score. Architecture is always
/// included; other findings remain visible even when outside the score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ReviewScope {
    /// Count electrical findings in the score (default true).
    pub electrical: bool,
    /// Count plumbing findings in the score (default true).
    pub plumbing: bool,
}

impl Default for ReviewScope {
    fn default() -> Self {
        Self {
            electrical: true,
            plumbing: true,
        }
    }
}

impl ReviewScope {
    #[must_use]
    pub fn includes(self, finding: &Finding) -> bool {
        match finding.discipline() {
            "electrical" => self.electrical,
            "plumbing" => self.plumbing,
            _ => true,
        }
    }
}

/// Who lives there.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Profile {
    /// Score scope, persisted with the project; excluded findings stay visible.
    pub scope: ReviewScope,
    /// People living in the home (default 2).
    pub occupants: u32,
    /// Of them, children (sleep in single beds or cribs).
    pub children: u32,
    /// Of them, elderly people.
    pub elderly: u32,
    /// Someone uses a wheelchair: NBR 9050 turning space, doors and reach.
    pub wheelchair: bool,
    /// Height of the main cook, cm, to size the countertop (default 165).
    pub stature: Option<f64>,
    /// City whose building code applies, e.g. `sao-paulo`. Without it,
    /// municipal rules only advise, because their numbers vary by city and,
    /// against a standard, the more restrictive one wins.
    pub city: Option<String>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            scope: ReviewScope::default(),
            occupants: 2,
            children: 0,
            elderly: 0,
            wheelchair: false,
            stature: None,
            city: None,
        }
    }
}

/// Where the project keeps who lives there, as JSON in its properties.
pub const PEOPLE: &str = "ergonomics:people";

impl Profile {
    /// The people the project was last reviewed for, or the defaults.
    ///
    /// A dry run scores a change for someone, and a score for two occupants
    /// cannot be compared with the 91 of a review conducted for three: the
    /// people belong to the project, like its city.
    #[must_use]
    pub fn of(home: &Home) -> Self {
        home.properties
            .get(PEOPLE)
            .and_then(|raw| serde_json::from_str(raw).ok())
            .unwrap_or_default()
    }
}

/// How much a finding matters: the same type every discipline reports with.
pub use newera_core::Severity;

/// One thing to look at.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    /// Room or piece it is about, e.g. `Quarto r5` or `Cama de casal f12`.
    pub place: String,
    /// What is wrong, with the numbers and what to do: a template and its
    /// data, Portuguese when read as a string — see [`newera_core::text::Text`].
    pub message: newera_core::text::Text,
    /// Code of the [`newera_core::Standard`] behind it, resolved once in
    /// [`Report::refs`]. The citation lives here, not inside the sentence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<&'static str>,
    /// A checked change that solves it, as MCP tool arguments:
    /// `{"tool":"move","ids":["f12"],"dx":-20,"dy":0}` or
    /// `{"tool":"update","items":[{"id":"f3","hinge_right":true}]}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<serde_json::Value>,
    /// What the score would gain if this one went away. A score that moves
    /// without saying why is a number nobody can act on.
    #[serde(default)]
    pub weight: u32,
    /// Name to accept it by, stable across runs: the rule and the place.
    pub key: String,
    /// Looked at and accepted, with the reason given. It still shows — a
    /// plan where a real finding is silently dropped is worse than one that
    /// carries it — but it no longer costs anything in the score.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted: Option<String>,
    /// The key the acceptance was written under, when the rule's source
    /// changed its prefix since: same rule, same piece, same reason kept.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_as: Option<String>,
    /// The move the advice implies when no fix is offered ("afaste 14 cm"):
    /// tried on a copy to say whether it opens another finding.
    #[serde(skip)]
    pub probe: Option<serde_json::Value>,
}

impl Finding {
    /// The discipline it is scored under: the tool that made it, or the
    /// rule it answers to — never the code it cites.
    #[must_use]
    pub fn discipline(&self) -> &'static str {
        if self.key.starts_with("elec:") {
            "electrical"
        } else if self.key.starts_with("plumb:") {
            "plumbing"
        } else {
            Rule::of_key(&self.key).map_or("architecture", Rule::discipline)
        }
    }
}

/// What the home offers its people.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Capacity {
    pub beds: u32,
    pub bedrooms: u32,
    pub bathrooms: u32,
    pub dining_seats: u32,
    pub living_seats: u32,
    /// Wardrobe front length in bedrooms and dedicated closets, cm.
    pub wardrobe_cm: f64,
}

impl Capacity {
    fn plus(&self, other: &Self) -> Self {
        Self {
            beds: self.beds + other.beds,
            bedrooms: self.bedrooms + other.bedrooms,
            bathrooms: self.bathrooms + other.bathrooms,
            dining_seats: self.dining_seats + other.dining_seats,
            living_seats: self.living_seats + other.living_seats,
            wardrobe_cm: self.wardrobe_cm + other.wardrobe_cm,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Report {
    pub scope: ReviewScope,
    /// Separate heuristic scores; never a certification of a discipline.
    pub scores: std::collections::BTreeMap<&'static str, u32>,
    /// 100 minus weighted findings.
    pub score: u32,
    pub capacity: Capacity,
    /// Worst first.
    pub findings: Vec<Finding>,
    /// The sources the findings stand on, each one once.
    pub refs: Vec<&'static Standard>,
    /// Where this review was conducted, which is what decides how much each
    /// source in `refs` obliges. It travels with the report because the letter
    /// is not a property of the source: whoever shows a citation has to know
    /// the place it was weighed at, or an ABNT standard reads as an obligation
    /// in Florida.
    pub place: newera_core::Place,
}

fn cm(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 {
        format!("{r:.0}")
    } else {
        format!("{r:.1}").replace('.', ",")
    }
}

fn m2(v: f64) -> String {
    format!("{:.2}", v / 10_000.0).replace('.', ",")
}

struct Review<'s, 'a> {
    scene: &'s Scene<'a>,
    profile: &'s Profile,
    /// Where the project is judged, which decides how much each source may
    /// claim. Resolved once for the whole review: every rule has to weigh the
    /// same law, and a review is re-run per finding that carries a fix and
    /// per storey, so this is not a place to look anything up twice.
    place: newera_core::Place,
    /// Every figure the rules ask for, resolved at `place` once.
    figures: std::collections::BTreeMap<&'static str, Resolved>,
    findings: Vec<Finding>,
    /// Glass that lights and airs each room, shared across rooms open to
    /// each other — see [`glass_by_room`].
    glass: std::collections::BTreeMap<newera_core::RoomId, f64>,
}

/// What the whole home holds — beds, seats, bathrooms — on every storey
/// being designed, not just the one shown (`scene`): the occupancy findings
/// speak of the house.
fn house_capacity(home: &Home, scene: &Scene<'_>) -> Capacity {
    let storeys = home.design_levels();
    // A home drawn without storeys is the scene; with one, it is that storey
    // even when a reference layer is the one shown.
    if storeys.is_empty() {
        return capacity_of(scene);
    }
    storeys.into_iter().fold(Capacity::default(), |sum, level| {
        let view = home.level_view(Some(level));
        sum.plus(&capacity_of(&Scene::new(&view)))
    })
}

/// What one storey holds.
fn capacity_of(scene: &Scene<'_>) -> Capacity {
    let mut c = Capacity::default();
    for (i, u) in scene.units.iter().enumerate() {
        let room = scene.spaces.iter().find(|s| s.units.contains(&i));
        match u.what {
            Use::Bed(n) => c.beds += n,
            Use::Crib => c.beds += 1,
            Use::DiningTable(n) | Use::DiningSet(n) => c.dining_seats += n,
            Use::Stool => c.dining_seats += 1,
            Use::Sofa(n) => c.living_seats += n,
            Use::Armchair => c.living_seats += 1,
            Use::Wardrobe if room.is_none_or(|r| r.what == RoomUse::Bedroom || r.is_closet()) => {
                c.wardrobe_cm += u.piece.width;
            }
            _ => {}
        }
    }
    c.wardrobe_cm = c.wardrobe_cm.round();
    c.bedrooms = scene
        .spaces
        .iter()
        .filter(|s| s.what == RoomUse::Bedroom)
        .count() as u32;
    c.bathrooms = scene
        .spaces
        .iter()
        .filter(|s| s.units.iter().any(|&i| scene.units[i].what == Use::Toilet))
        .count() as u32;
    c
}

impl Review<'_, '_> {
    fn room_program(&mut self) {
        for space in &self.scene.spaces {
            let sanitary: Vec<_> = space
                .units
                .iter()
                .filter_map(|&i| {
                    let u = &self.scene.units[i];
                    matches!(u.what, Use::Toilet | Use::Shower | Use::Bathtub)
                        .then_some(u.piece.id.to_string())
                })
                .collect();
            if !sanitary.is_empty()
                && space.what != RoomUse::Bathroom
                && (space.what != RoomUse::Other
                    || space.room.usage == newera_core::RoomUse::Closet)
            {
                self.findings.push(Finding {
                    severity: Severity::Alerta,
                    place: space.label(),
                    message: say!("Uso do ambiente como {} conflita com equipamentos sanitários ({}). Confirme o programa e declare room_use; o nome e as peças foram preservados.", space.what.said(), sanitary.join(", ")),
                    key: rule::key(Rule::RoomUseConflict, &space.label()),
                    ..Finding::default()
                });
            }
            if space.what == RoomUse::Bathroom {
                for &i in &space.units {
                    let piece = self.scene.units[i].piece;
                    if piece.catalog == "sink-counter" {
                        self.findings.push(Finding {
                            severity: Severity::Dica,
                            place: space.label(),
                            message: say!("Pia de cozinha {} usada como lavatório: renomear não altera o equipamento. Confirme se este catálogo corresponde à peça pretendida para o banheiro.", piece.id),
                            key: rule::key_at(
                                Rule::BathroomFixture,
                                &space.label(),
                                &piece.id.to_string(),
                            ),
                            ..Finding::default()
                        });
                    }
                }
            }
        }
    }

    /// A finding of `rule` about `place`. The rule names it; the sentence is
    /// free to be rewritten without dropping anybody's acceptance.
    fn push(
        &mut self,
        rule: Rule,
        severity: Severity,
        place: impl Into<String>,
        message: impl Into<Text>,
    ) {
        let place = place.into();
        self.findings.push(Finding {
            severity,
            key: rule::key(rule, &place),
            place,
            message: message.into(),
            reference: None,
            fix: None,
            ..Finding::default()
        });
    }

    /// The same, standing on a published source. A tier E survey says what
    /// people do, never what they should do, so it raises nothing; a figure
    /// marked to confirm at the source may warn, never accuse.
    fn push_ref(
        &mut self,
        rule: Rule,
        severity: Severity,
        place: impl Into<String>,
        message: impl Into<Text>,
        code: &'static str,
    ) {
        let Some(source) = standards::standard(code) else {
            debug_assert!(false, "unknown standard `{code}`");
            self.push(rule, severity, place, message);
            return;
        };
        let Some(severity) = self.weigh(severity, Some(code)) else {
            return;
        };
        let place = place.into();
        self.findings.push(Finding {
            severity,
            key: rule::key(rule, &place),
            place,
            message: message.into(),
            reference: Some(source.code),
            fix: None,
            ..Finding::default()
        });
    }

    /// What a finding of `severity` standing on `code` may claim here, or
    /// nothing at all. How much the source obliges is a question about where
    /// the project is, not about the source alone: a tier E survey says what
    /// people do, never what they should do, so it raises nothing; the tier
    /// sets the ceiling; and a figure we could not confirm may warn, never
    /// accuse. Common practice cites nothing and is capped by nothing.
    fn weigh(&self, severity: Severity, code: Option<&str>) -> Option<Severity> {
        standards::weigh(severity, code, &self.place)
    }

    /// The figure called `name` where this review is conducted. Asked of the
    /// table resolved when the review began, never of the registry.
    fn fig(&self, name: &str) -> Option<Resolved> {
        let found = self.figures.get(name).copied();
        debug_assert!(
            found.is_some() || standards::figures().any(|f| f.name == name),
            "no figure is called `{name}`"
        );
        found
    }

    /// A finding standing on a figure: its severity, and its source if it
    /// has one.
    fn push_fig(
        &mut self,
        rule: Rule,
        figure: &Resolved,
        place: impl Into<String>,
        message: impl Into<Text>,
    ) {
        match figure.source {
            Some(code) => self.push_ref(rule, figure.severity, place, message, code),
            None => self.push(rule, figure.severity, place, message),
        }
    }

    /// A rule that speaks more than once about one place says which pair or
    /// piece each finding is about, so the two are accepted apart.
    fn push_fig_at(
        &mut self,
        rule: Rule,
        about: &str,
        figure: &Resolved,
        place: impl Into<String>,
        message: impl Into<Text>,
    ) {
        let place = place.into();
        let key = rule::key_at(rule, &place, about);
        let before = self.findings.len();
        self.push_fig(rule, figure, place, message);
        // A source that raises nothing pushes nothing: only a finding that
        // was actually made is renamed, never the one before it.
        for f in &mut self.findings[before..] {
            f.key.clone_from(&key);
        }
    }

    fn occupancy(&mut self, c: &Capacity) {
        let p = self.profile.clone();
        let people = p.occupants.max(1);
        let home = "Casa";
        if c.beds < people {
            self.push(
                Rule::SleepingPlaces,
                Severity::Erro,
                home,
                say!(
                    "{} moradores e {} lugares para dormir: {} (camas de casal contam 2).",
                    people,
                    c.beds,
                    match people - c.beds {
                        1 => say!("falta 1"),
                        n => say!("faltam {}", n),
                    }
                ),
            );
        }
        if c.bedrooms > 0 {
            let per = f64::from(people) / f64::from(c.bedrooms);
            let crowded = self.fig("occupancy.people_per_bedroom");
            let shared = self.fig("occupancy.people_per_bedroom.sharing");
            if let Some(most) = crowded
                && per > most.bound.value()
            {
                let most_people = most.bound.value() as u32;
                self.push_fig(
                    Rule::BedroomCrowding,
                    &most,
                    home,
                    say!("{} moradores por dormitório: acima de {} é adensamento excessivo; são necessários {} dormitórios.", cm(per), cm(most.bound.value()), people.div_ceil(most_people)),
                );
            } else if let Some(most) = shared
                && per > most.bound.value()
            {
                self.push_fig(
                    Rule::BedroomSharing,
                    &most,
                    home,
                    say!("{} moradores por dormitório; com {} dormitórios ninguém divide com mais de uma pessoa.", cm(per), people.div_ceil(most.bound.value() as u32)),
                );
            }
        } else if people > 1 {
            self.push(
                Rule::NoBedroom,
                Severity::Alerta,
                home,
                "Nenhum dormitório identificado: nomeie os cômodos (Quarto, Suíte) ou coloque camas.",
            );
        }
        if c.bathrooms == 0 {
            self.push(
                Rule::NoToilet,
                Severity::Erro,
                home,
                "Nenhum banheiro com vaso sanitário.",
            );
        } else if let Some(most) = self.fig("occupancy.people_per_bathroom")
            && f64::from(people) > most.bound.value() * f64::from(c.bathrooms)
        {
            self.push_fig(
                Rule::BathroomCount,
                &most,
                home,
                say!("{} moradores para {} banheiro(s): acima de {} por banheiro as filas de manhã são certas; considere um lavabo ou mais um banheiro.", people, c.bathrooms, cm(most.bound.value())),
            );
        }
        if c.dining_seats < people {
            self.push(
                Rule::DiningSeats,
                Severity::Alerta,
                home,
                say!(
                    "{} lugares à mesa para {} moradores: use uma mesa de {} lugares.",
                    c.dining_seats,
                    people,
                    people.max(4).div_ceil(2) * 2
                ),
            );
        }
        if c.living_seats < people {
            self.push(
                Rule::LivingSeats,
                Severity::Dica,
                home,
                say!("{} lugares na sala para {} moradores: todos sentam juntos com mais {} lugar(es) (poltrona ou sofá maior).", c.living_seats, people, people - c.living_seats),
            );
        }
        // Children count half.
        let Some(per_adult) = self.fig("occupancy.wardrobe_per_adult") else {
            return;
        };
        let each = per_adult.bound.value();
        let adults = people.saturating_sub(p.children);
        let needed = each * f64::from(adults) + each / 2.0 * f64::from(p.children.min(people));
        if c.wardrobe_cm + 1.0 < needed {
            self.push_fig(
                Rule::WardrobeCapacity,
                &per_adult,
                home,
                say!("{} cm de guarda-roupa nos dormitórios e closets; a norma de desempenho (anexo F) prevê 1,60 m no dormitório de casal e 1,20 m no de solteiro, uns {} cm por adulto e metade por criança: {} cm no total.", cm(c.wardrobe_cm), cm(each), cm(needed)),
            );
        }
    }

    /// What NBR 5410 asks of each room, brought into the score: a flat with
    /// no outlet anywhere scored the same 99 as one fully wired, and
    /// starting the project made the score drop before it rose. Only rooms of
    /// a plan that is furnished are held to it, so a sketch is not; an
    /// absence on purpose is accepted with its reason, by the same key the
    /// electrical tool uses.
    fn electrical(&mut self) {
        let furnished = self
            .scene
            .home
            .furniture
            .iter()
            .any(|f| !f.is_opening() && f.discipline.is_none());
        if !furnished {
            return;
        }
        for f in newera_core::electrical::check(self.scene.home) {
            self.findings.push(imported(f, true));
        }
    }

    /// What each fixture needs of the plumbing project, brought into the
    /// score like the electrical one: a toilet with no water or no sewer
    /// point is a bathroom that does not work. Only plans with a fixture are
    /// looked at, and an absence on purpose is accepted by the plumbing key.
    /// Balcony guards and closures (NBR 14718, NBR 7199, NBR 16259): what
    /// guards a fall is a safety matter, and weighs as such.
    fn guards(&mut self) {
        for f in newera_core::guard::check(self.scene.home) {
            self.findings.push(imported(f, false));
        }
    }

    fn plumbing(&mut self) {
        for f in newera_core::plumbing::check(self.scene.home) {
            self.findings.push(imported(f, true));
        }
    }

    /// Circulation around pieces: the informative annex of NBR 15575-1 (50 cm
    /// between furniture and walls, 85 cm in front of kitchen equipment) and
    /// common practice for the rest.
    #[allow(clippy::too_many_lines)]
    fn clearances(&mut self) {
        let scene = self.scene;
        let wheel = self.profile.wheelchair;
        for (i, u) in scene.units.iter().enumerate() {
            // Above the head nobody walks in front of it: a crown moulding at
            // 272 cm is passed under, not around.
            if u.piece.height_range().0 >= HEADROOM {
                continue;
            }
            let label = u.label();
            // `at_least` is what the piece asks beyond the figure, when a
            // formula asks more: a hinged leaf swings its own width.
            let need = |side: Side,
                        figure: Option<Resolved>,
                        at_least: f64,
                        span: (f64, f64),
                        reason: &'static str| {
                let figure = figure?;
                let min = figure.bound.value().max(at_least);
                let severity = self.weigh(figure.severity, figure.source)?;
                let (free, blocker, tight) = scene.free_along(i, side, min + 1.0, span, min);
                if free + 0.5 < min {
                    let where_ = match side {
                        Side::Front => say!("à frente"),
                        Side::Left => say!("à esquerda"),
                        Side::Right => say!("à direita"),
                    };
                    let short = min - free;
                    // Sideways, the piece can slide over if the other side keeps its own room.
                    let fix = match side {
                        // In front: push the piece in the way back by the shortfall.
                        Side::Front => blocker.and_then(|j| {
                            let piece = scene.units[i].frame();
                            let way = piece.to_plan((0.0, 1.0));
                            let dir = (way.x - piece.position.x, way.y - piece.position.y);
                            push_away(scene, j, dir, short.ceil())
                        }),
                        Side::Left | Side::Right => {
                            let (other, sign) = if side == Side::Left {
                                (Side::Right, 1.0)
                            } else {
                                (Side::Left, -1.0)
                            };
                            let spare = scene.free(i, other, short + min + 1.0, span);
                            (spare - short + 0.5 >= min).then(|| {
                                let piece = scene.units[i].frame();
                                let to = piece.to_plan((sign * short.ceil(), 0.0));
                                let round = |v: f64| (v * 10.0).round() / 10.0;
                                serde_json::json!({
                                    "tool": "move",
                                    "ids": [piece.id.to_string()],
                                    "dx": round(to.x - piece.position.x),
                                    "dy": round(to.y - piece.position.y),
                                })
                            })
                        }
                    };
                    let advice = if let (Side::Front, Some(f), Some(j)) = (side, &fix, blocker) {
                        say!(
                            "afaste {} {} cm",
                            scene.units[j].label(),
                            cm(f["dx"]
                                .as_f64()
                                .unwrap_or_default()
                                .hypot(f["dy"].as_f64().unwrap_or_default()))
                        )
                    } else if fix.is_some() || side == Side::Front {
                        say!("afaste {} cm", cm(short))
                    } else {
                        say!("não há espaço do outro lado: use peça menor ou reorganize")
                    };
                    // How much of the side is that narrow: a corner taken by
                    // a nightstand is not a wardrobe that does not open.
                    let frame = scene.units[i].frame();
                    let whole = match side {
                        Side::Front => frame.width,
                        Side::Left | Side::Right => frame.depth,
                    } * (span.1 - span.0);
                    let stretch = if tight + 2.0 < whole - 4.0 {
                        say!(" em {} dos {} cm", cm(tight), cm(whole))
                    } else {
                        Text::default()
                    };
                    // With no fix, "afaste" means backing the piece itself away.
                    let probe = if fix.is_none() && side == Side::Front {
                        let piece = scene.units[i].frame();
                        let back = piece.to_plan((0.0, -short.ceil()));
                        Some(serde_json::json!({
                            "tool": "move",
                            "ids": [scene.units[i].piece.id.to_string()],
                            "dx": back.x - piece.position.x,
                            "dy": back.y - piece.position.y,
                        }))
                    } else {
                        None
                    };
                    Some(Finding {
                        severity,
                        // A piece is short of room on one side at a time: the
                        // side belongs in the key, or the front accepted for a
                        // wardrobe would silence its left as well.
                        key: rule::key_at(
                            Rule::Clearance,
                            &label,
                            match side {
                                Side::Front => "front",
                                Side::Left => "left",
                                Side::Right => "right",
                            },
                        ),
                        place: label.clone(),
                        message: say!(
                            "{} cm livres {}{} ({}: mínimo {} cm); {}.",
                            cm(free),
                            where_,
                            stretch,
                            Text::from(reason),
                            cm(min),
                            advice
                        ),
                        reference: figure.source,
                        fix,
                        probe,
                        ..Finding::default()
                    })
                } else {
                    None
                }
            };
            let mut found: Vec<Finding> = Vec::new();
            // Beside a bed, the band past the nightstands.
            let beside = (0.25, 1.0);
            let whole = (0.0, 1.0);
            // A wheelchair asks the accessibility standard instead.
            let fig = |name: &str| {
                if wheel {
                    self.fig(&format!("{name}.wheelchair"))
                        .or_else(|| self.fig(name))
                } else {
                    self.fig(name)
                }
            };
            match u.what {
                Use::Bed(n) => {
                    let why = if wheel {
                        "transferência da cadeira"
                    } else {
                        "circulação ao lado da cama"
                    };
                    let side = fig("clearance.bed.side");
                    let left = need(Side::Left, side, 0.0, beside, why);
                    let right = need(Side::Right, side, 0.0, beside, why);
                    if n >= 2 {
                        // A couple needs both sides.
                        found.extend(left);
                        found.extend(right);
                    } else if let (Some(l), Some(_)) = (left, right) {
                        // A single bed may touch a wall on one side.
                        found.push(l);
                    }
                    found.extend(need(
                        Side::Front,
                        self.fig("clearance.bed.foot"),
                        0.0,
                        whole,
                        "passagem aos pés da cama",
                    ));
                }
                Use::Crib => found.extend(need(
                    Side::Front,
                    self.fig("clearance.crib.front"),
                    0.0,
                    whole,
                    "acesso ao berço",
                )),
                Use::Wardrobe => {
                    // Hinged doors of a joinery build swing their own width.
                    let leaf = u
                        .params
                        .as_ref()
                        .filter(|p| p["door"] != "sliding" && p["door"] != "none")
                        .map_or(0.0, |p| {
                            let w = p["w"].as_f64().unwrap_or(u.piece.width);
                            let doors = p["doors"]
                                .as_f64()
                                .unwrap_or_else(|| (w / 60.0).ceil())
                                .max(1.0);
                            w / doors
                        });
                    let past = self
                        .fig("clearance.wardrobe.past_leaf")
                        .map_or(0.0, |f| f.bound.value());
                    found.extend(need(
                        Side::Front,
                        self.fig("clearance.wardrobe.front"),
                        leaf + past,
                        whole,
                        if leaf > 0.0 {
                            "abrir as portas e circular diante do guarda-roupa; portas de correr pedem menos"
                        } else {
                            "abrir as portas e circular diante do guarda-roupa"
                        },
                    ));
                }
                Use::Dresser => found.extend(need(
                    Side::Front,
                    self.fig("clearance.dresser.front"),
                    0.0,
                    whole,
                    "abrir gavetas e ficar diante delas",
                )),
                // The cabinets under a countertop answer for it and for what is set in it.
                // In a bathroom they are a vanity, used like the basin in it.
                Use::Counter
                    if !scene.countertop(i)
                        && scene
                            .spaces
                            .iter()
                            .any(|s| s.what == RoomUse::Bathroom && s.units.contains(&i)) =>
                {
                    found.extend(need(
                        Side::Front,
                        fig("clearance.basin.front"),
                        0.0,
                        (0.05, 0.95),
                        if wheel {
                            "aproximação frontal ao lavatório"
                        } else {
                            "uso do lavatório"
                        },
                    ));
                }
                Use::Fridge | Use::Stove | Use::Sink | Use::Counter | Use::Appliance
                    if !scene.countertop(i) && !scene.embedded_item(i) =>
                {
                    // A blind corner's panel sits behind the other run: only its door counts.
                    let blind = |key: &str| {
                        u.params
                            .as_ref()
                            .and_then(|p| p[key].as_f64())
                            .unwrap_or(0.0)
                            / u.piece.width.max(1.0)
                    };
                    let span = if u.piece.mirrored {
                        (0.05 + blind("blind_right"), 0.95 - blind("blind_left"))
                    } else {
                        (0.05 + blind("blind_left"), 0.95 - blind("blind_right"))
                    };
                    if span.1 > span.0 {
                        found.extend(need(
                            Side::Front,
                            fig("clearance.kitchen.front"),
                            0.0,
                            span,
                            if wheel {
                                "giro de cadeira de rodas"
                            } else {
                                "circulação diante de bancada e equipamentos"
                            },
                        ));
                    }
                }
                Use::Island => {
                    found.extend(need(
                        Side::Front,
                        self.fig("clearance.island.around"),
                        0.0,
                        whole,
                        "circulação em volta da ilha",
                    ));
                }
                // NBR 15575-1 annex F: 40 cm in front of basin, toilet and
                // bidet; NBR 9050 a 0,80 × 1,20 m transfer module.
                Use::Toilet => found.extend(need(
                    Side::Front,
                    fig("clearance.toilet.front"),
                    0.0,
                    whole,
                    if wheel {
                        "área de transferência"
                    } else {
                        "uso do vaso"
                    },
                )),
                Use::Basin if !scene.countertop(i) && !scene.embedded_item(i) => {
                    found.extend(need(
                        Side::Front,
                        fig("clearance.basin.front"),
                        0.0,
                        whole,
                        if wheel {
                            "aproximação frontal ao lavatório"
                        } else {
                            "uso do lavatório"
                        },
                    ));
                }
                Use::Shower | Use::Bathtub => found.extend(need(
                    Side::Front,
                    self.fig("clearance.shower.front"),
                    0.0,
                    (0.2, 0.8),
                    "entrar e sair do box",
                )),
                Use::Washer | Use::LaundrySink => found.extend(need(
                    Side::Front,
                    self.fig("clearance.laundry.front"),
                    0.0,
                    whole,
                    "uso do tanque e da máquina de lavar",
                )),
                Use::Desk => found.extend(need(
                    Side::Front,
                    self.fig("clearance.desk.front"),
                    0.0,
                    (0.2, 0.8),
                    "cadeira e levantar-se da mesa",
                )),
                Use::DiningTable(_) => {
                    // The sides people sit on: where chairs are, or the front
                    // and both ends of a table drawn without chairs.
                    let piece = u.piece;
                    let chairs: Vec<(f64, f64)> = scene
                        .units
                        .iter()
                        .filter(|c| matches!(c.what, Use::Chair | Use::Stool))
                        .map(|c| piece.to_local(c.piece.position))
                        .filter(|(x, y)| {
                            x.abs() < piece.width / 2.0 + 70.0 && y.abs() < piece.depth / 2.0 + 70.0
                        })
                        .collect();
                    let (hw, hd) = (piece.width / 2.0, piece.depth / 2.0);
                    for side in [Side::Front, Side::Left, Side::Right] {
                        let seated = chairs.is_empty()
                            || chairs.iter().any(|&(x, y)| match side {
                                Side::Front => y > hd - 10.0,
                                Side::Left => x < -hw + 10.0,
                                Side::Right => x > hw - 10.0,
                            });
                        if seated {
                            found.extend(need(
                                side,
                                self.fig("clearance.table.seat"),
                                0.0,
                                (0.2, 0.8),
                                "puxar a cadeira e sentar",
                            ));
                        }
                    }
                }
                Use::DiningSet(_) => {
                    for side in [Side::Front, Side::Left, Side::Right] {
                        found.extend(need(
                            side,
                            self.fig("clearance.dining_set.behind"),
                            0.0,
                            (0.2, 0.8),
                            "passar atrás das cadeiras",
                        ));
                    }
                }
                Use::Sofa(_) | Use::Armchair => {
                    found.extend(need(
                        Side::Front,
                        self.fig("clearance.seat.front"),
                        0.0,
                        (0.2, 0.8),
                        "sentar, levantar e circular diante do assento",
                    ));
                }
                _ => {}
            }
            self.findings.extend(found);
        }
    }

    fn doors(&mut self) {
        let home = self.scene.home;
        let wheel = self.profile.wheelchair || self.profile.elderly > 0;
        let narrow = self.fig("door.clear_width");
        let rolling = self.fig("door.clear_width.wheelchair");
        for door in home.furniture.iter().filter(|f| f.visible) {
            let Some(opening) = &door.opening else {
                continue;
            };
            if opening.kind == OpeningKind::Window {
                continue;
            }
            let label = format!("{} {}", door.name, door.id);
            // Frame stops take about 4 cm of the nominal width.
            let clear = door.width - 4.0;
            if let Some(min) = narrow
                && clear < min.bound.value()
            {
                self.push_fig(
                    Rule::DoorClearWidth,
                    &min,
                    label,
                    say!("Vão livre de cerca de {} cm: abaixo de {} cm não passa um móvel nem uma pessoa com volumes; use porta de 70 cm ou mais.", cm(clear), cm(min.bound.value())),
                );
            } else if let Some(min) = rolling
                && wheel
                && clear < min.bound.value()
            {
                self.push_fig(
                    Rule::DoorWidthWheelchair,
                    &min,
                    label,
                    say!("Vão livre de cerca de {} cm: cadeira de rodas e andador pedem {} cm livres; use porta de 90 cm.", cm(clear), cm(min.bound.value())),
                );
            }
        }
        // A bedroom or a bathroom reached only through an open passage has
        // no privacy, and nothing else says so: the passage and a panel drawn
        // beside it as the open leaf are each valid on their own — which is
        // how four rooms of an imported plan went without a door.
        for space in self
            .scene
            .spaces
            .iter()
            .filter(|s| matches!(s.what, RoomUse::Bedroom | RoomUse::Bathroom))
        {
            let near_edge = |p: Point2| outline_distance(&space.room.points, p) <= 25.0;
            let ways_in: Vec<&newera_core::Furniture> = home
                .furniture
                .iter()
                .filter(|f| f.visible)
                .filter(|f| {
                    f.opening
                        .as_ref()
                        .is_some_and(|o| o.kind != OpeningKind::Window)
                })
                .filter(|f| near_edge(f.position))
                .collect();
            let passages: Vec<&&newera_core::Furniture> = ways_in
                .iter()
                .filter(|f| {
                    f.opening
                        .as_ref()
                        .is_some_and(|o| o.kind == OpeningKind::Passage)
                })
                .collect();
            let what = if space.what == RoomUse::Bathroom {
                say!("banheiro")
            } else {
                say!("dormitório")
            };
            // No way in at all: a sealed room. Only said once the plan has
            // doors somewhere — a sketch of rooms with no openings yet is a
            // sketch, not a sealed flat.
            if ways_in.is_empty() {
                let has_openings = home.furniture.iter().any(|f| {
                    f.visible
                        && f.opening
                            .as_ref()
                            .is_some_and(|o| o.kind != OpeningKind::Window)
                });
                if has_openings {
                    self.push(
                        Rule::RoomWithoutAccess,
                        Severity::Erro,
                        space.label(),
                        say!("Sem acesso: nenhuma porta nem vão chega ao {}; sem uma porta, ele não pode ser usado.", what),
                    );
                }
                continue;
            }
            if passages.len() < ways_in.len() {
                continue;
            }
            // A panel named as a door, standing beside the passage, is the
            // leaf somebody drew instead of placing one.
            let drawn_leaf = home
                .furniture
                .iter()
                .filter(|f| f.opening.is_none())
                .find(|f| {
                    let name = newera_core::fold(&f.name);
                    (name.contains("porta") || name.contains("door"))
                        && passages
                            .iter()
                            .any(|p| p.position.distance(f.position) <= 120.0)
                });
            let mut message = say!(
                "Sem porta: o único acesso ao {} é um vão livre ({}), que não fecha nem dá privacidade; troque por uma porta.",
                what,
                passages
                    .iter()
                    .map(|p| p.id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if let Some(leaf) = drawn_leaf {
                message = message.then(say!(
                    "{} {} parece a folha desenhada ao lado, sem ser uma abertura.",
                    leaf.name,
                    leaf.id
                ));
            }
            self.push(
                Rule::RoomWithoutDoor,
                Severity::Alerta,
                space.label(),
                message,
            );
        }
        let scene = self.scene;
        for (door, unit) in scene.door_hits() {
            let door_name = format!("{} {}", door.name, door.id);
            let by = scene.units[unit].label();
            // One door can be blocked by more than one piece, and the leaf
            // freed of one still meets the other: each is its own finding,
            // however the advice reads — flipping the hinge or moving it.
            let blocked = rule::key_at(
                Rule::DoorLeafBlocked,
                &door_name,
                &scene.units[unit].piece.id.to_string(),
            );
            // Would the leaf clear it swinging from the other jamb?
            let flip = door
                .opening
                .as_ref()
                .filter(|o| o.leaves < 2 && !o.sliding && o.sashes.is_empty())
                .and_then(|o| {
                    let mut flipped = door.clone();
                    let right = !o.hinge_right;
                    flipped.opening.as_mut()?.hinge_right = right;
                    let swing = newera_core::door_swing(&flipped)?;
                    // Clear by this review's reading and by the rule
                    // check_layout reports blocks_door with: a basin the
                    // review reads as built in still stops a real leaf.
                    (scene.swing_clear(&swing)
                        && newera_core::door_blocked_by(scene.home, &flipped).is_empty())
                    .then_some(right)
                });
            if let Some(right) = flip {
                self.findings.push(Finding {
                    severity: Severity::Erro,
                    key: blocked,
                    place: door_name,
                    message: say!("A folha da porta bate em {}: com a dobradiça do outro lado ela abre livre (hinge_right hoje {}, use {}).", by, !right, right),
                    reference: None,
                    fix: Some(serde_json::json!({
                        "tool": "update",
                        "items": [{"id": door.id.to_string(), "hinge_right": right}],
                    })),
                    ..Finding::default()
                });
                continue;
            }
            let moved = nudge(scene, unit);
            self.findings.push(Finding {
                severity: Severity::Erro,
                key: blocked,
                place: door_name,
                message: if let Some(m) = &moved {
                    say!(
                        "A folha da porta bate em {}: movendo a peça {} cm a porta abre livre.",
                        by,
                        cm(m["d"].as_f64().unwrap_or_default())
                    )
                } else {
                    say!(
                        "A folha da porta bate em {}: mova a peça ou use porta de correr.",
                        by
                    )
                },
                reference: None,
                fix: moved.map(without_distance),
                ..Finding::default()
            });
        }
        for (a, b) in scene.overlaps() {
            let fix = nudge(scene, b).or_else(|| nudge(scene, a));
            self.findings.push(Finding {
                severity: Severity::Erro,
                // The pair, not the piece: a wardrobe accepted over the rug
                // has not been accepted over the bed.
                key: rule::key_at(
                    Rule::SamePlace,
                    &scene.units[a].label(),
                    &scene.units[b].piece.id.to_string(),
                ),
                place: scene.units[a].label(),
                message: if let Some(f) = &fix {
                    say!(
                        "Ocupa o mesmo lugar que {}: movendo {} {} cm fica livre.",
                        scene.units[b].label(),
                        f["ids"][0].as_str().unwrap_or_default(),
                        cm(f["d"].as_f64().unwrap_or_default())
                    )
                } else {
                    say!(
                        "Ocupa o mesmo lugar que {}: não há lugar livre por perto, reorganize.",
                        scene.units[b].label()
                    )
                },
                reference: None,
                fix: fix.map(without_distance),
                ..Finding::default()
            });
        }
    }

    #[allow(clippy::too_many_lines)]
    fn rooms(&mut self) {
        let scene = self.scene;
        let home = scene.home;
        let wheel = self.profile.wheelchair;
        let spaces: Vec<RoomFacts> = scene
            .spaces
            .iter()
            .enumerate()
            .map(|(k, s)| {
                let area = s.room.area();
                let short = short_side(&s.room.points);
                // Ceiling: the lowest wall around the room.
                let ceiling = home
                    .walls
                    .iter()
                    .filter(|w| {
                        near_outline(&s.room.points, w.start.midpoint_with(w.end), w.thickness)
                    })
                    .map(|w| w.height.min(w.height_at_end.unwrap_or(w.height)))
                    .reduce(f64::min)
                    .unwrap_or(home.wall_height);
                let glass: f64 = self.glass.get(&s.room.id).copied().unwrap_or_default();
                let uses = s.units.iter().map(|&i| scene.units[i].what).collect();
                RoomFacts {
                    label: s.label(),
                    what: s.what,
                    area,
                    short,
                    ceiling,
                    glass,
                    uses,
                    index: k,
                }
            })
            .collect();
        for RoomFacts {
            label,
            what,
            area,
            short,
            ceiling,
            glass,
            uses,
            index: k,
        } in spaces
        {
            // Minimum areas and narrow sides now come from the figure table,
            // one row per authority: São Paulo's decree 57.776, the state
            // sanitary code, NBR 15575-1 annex F, and NBR 9050 6.11.1 for a
            // corridor somebody rolls through. A figure is asked for, not
            // decided here, so a decree written for one city stops judging
            // projects in the next one.
            let long_side = if short > 0.0 { area / short } else { 0.0 };
            let slug = match what {
                RoomUse::Bedroom => Some("bedroom"),
                RoomUse::Living => Some("living"),
                RoomUse::Kitchen => Some("kitchen"),
                RoomUse::Bathroom => Some("bathroom"),
                RoomUse::Laundry => Some("laundry"),
                RoomUse::Corridor => Some("corridor"),
                _ => None,
            };
            if let Some(slug) = slug {
                // Not every use has every figure — nobody sets a minimum area
                // for a bathroom — so an absent row here is not a typo.
                let ask = |what: &str| {
                    self.figures
                        .get(format!("room.{slug}.{what}").as_str())
                        .copied()
                };
                // A wheelchair asks the accessibility standard instead of the
                // city, and it is the longer run that needs the extra width.
                let side = if wheel && slug == "corridor" {
                    ask(if long_side > 400.0 {
                        "min_side.wheelchair.long"
                    } else {
                        "min_side.wheelchair"
                    })
                } else {
                    ask("min_side")
                };
                if let Some(min) = ask("min_area")
                    && area + 1.0 < min.bound.value()
                {
                    self.push_fig(
                        Rule::RoomArea,
                        &min,
                        &label,
                        say!("{} m² para {}: códigos de obras costumam pedir ao menos {} m² (confira o do seu município).", m2(area), what.said(), m2(min.bound.value())),
                    );
                }
                if let Some(min) = side
                    && short + 0.5 < min.bound.value()
                {
                    self.push_fig(
                        Rule::RoomNarrowSide,
                        &min,
                        &label,
                        say!(
                            "Menor lado de {} cm; para {} a referência é pelo menos {} cm.",
                            cm(short),
                            what.said(),
                            cm(min.bound.value())
                        ),
                    );
                }
            }
            // A use may be allowed lower — a bathroom, a corridor — and which
            // ones is the code's to say, so the use asks first.
            let min_ceiling = slug
                .and_then(|slug| {
                    self.figures
                        .get(format!("room.{slug}.min_ceiling").as_str())
                        .copied()
                })
                .or_else(|| self.fig("room.min_ceiling"));
            if let Some(min) = min_ceiling
                && what != RoomUse::Other
                && ceiling + 0.5 < min.bound.value()
            {
                self.push_fig(
                    Rule::CeilingHeight,
                    &min,
                    &label,
                    say!(
                        "Pé-direito de {} cm; o mínimo é {} cm em {}.",
                        cm(ceiling),
                        cm(min.bound.value()),
                        what.said()
                    ),
                );
            }
            // Daylight: a share of the floor in glass, larger to work or
            // study, smaller in a service area.
            if what != RoomUse::Other && what != RoomUse::Corridor {
                let per_glass = self.fig(match what {
                    RoomUse::Office => "room.floor_per_glass.work",
                    RoomUse::Laundry => "room.floor_per_glass.service",
                    _ => "room.floor_per_glass",
                });
                if glass <= 0.0 {
                    let severity = if what.long_stay() {
                        Severity::Alerta
                    } else {
                        Severity::Dica
                    };
                    let what_to_do = if what == RoomUse::Bathroom {
                        say!("use janela basculante ou ventilação mecânica")
                    } else {
                        say!("precisa de janela para luz e ventilação")
                    };
                    self.push(
                        Rule::RoomWithoutWindow,
                        severity,
                        &label,
                        say!("Sem janela: {}.", what_to_do),
                    );
                } else if let Some(most) = per_glass
                    && glass * most.bound.value() + 1.0 < area
                {
                    let ratio = most.bound.value();
                    self.push_fig(
                        Rule::WindowArea,
                        &most,
                        &label,
                        // São Paulo's sanitary code is the one the sentence
                        // was written for; any other code is named by its note.
                        if most.source == Some("coe-municipal") {
                            say!(
                                "Janelas somam {} m² para {} m² de piso; o Código Sanitário de SP pede 1/{} do piso: {} m², com metade abrindo para ventilar.",
                                m2(glass),
                                m2(area),
                                cm(ratio),
                                m2(area / ratio)
                            )
                        } else {
                            say!(
                                "Janelas somam {} m² para {} m² de piso; {} pede 1/{} do piso: {} m², com metade abrindo para ventilar.",
                                m2(glass),
                                m2(area),
                                most.note,
                                cm(ratio),
                                m2(area / ratio)
                            )
                        },
                    );
                }
            }
            // Minimum furniture, NBR 15575-1 informative annex.
            let has = |f: &dyn Fn(&Use) -> bool| uses.iter().any(f);
            let missing: Vec<&str> = match what {
                RoomUse::Bedroom => [
                    (has(&|u| matches!(u, Use::Bed(_) | Use::Crib)), "cama"),
                    (self.scene.wardrobe_serves(&scene.spaces[k]), "guarda-roupa"),
                ]
                .into_iter()
                .filter(|(ok, _)| !ok)
                .map(|(_, n)| n)
                .collect(),
                RoomUse::Kitchen => [
                    (has(&|u| matches!(u, Use::Stove)), "fogão ou cooktop"),
                    (has(&|u| matches!(u, Use::Fridge)), "geladeira"),
                    (has(&|u| matches!(u, Use::Sink)), "pia"),
                ]
                .into_iter()
                .filter(|(ok, _)| !ok)
                .map(|(_, n)| n)
                .collect(),
                RoomUse::Bathroom => [
                    (has(&|u| matches!(u, Use::Toilet)), "vaso sanitário"),
                    (has(&|u| matches!(u, Use::Basin)), "lavatório"),
                    (
                        // A lavabo takes no shower.
                        newera_core::vocabulary::mentions(&label)
                            .has(newera_core::vocabulary::Mention::Lavatory)
                            || has(&|u| matches!(u, Use::Shower | Use::Bathtub)),
                        "box ou banheira",
                    ),
                ]
                .into_iter()
                .filter(|(ok, _)| !ok)
                .map(|(_, n)| n)
                .collect(),
                _ => Vec::new(),
            };
            if !missing.is_empty() {
                self.push_ref(
                    Rule::MissingFixtures,
                    Severity::Dica,
                    &label,
                    say!(
                        "Falta {} para testar o uso de {}.",
                        Text::joined(missing.iter().map(|&w| Text::from(w)).collect(), "{}, {}"),
                        what.said()
                    ),
                    "nbr15575g",
                );
            }
            if wheel && matches!(what, RoomUse::Bathroom | RoomUse::Kitchen) {
                let space = &scene.spaces[k];
                let turn = scene.turning_diameter(space);
                if let Some(min) = self.fig("room.wheelchair_turn")
                    && turn + 1.0 < min.bound.value()
                {
                    self.push_fig(
                        Rule::WheelchairTurn,
                        &min,
                        &label,
                        say!("Cabe um giro de {} cm; são necessários {} cm livres para girar a cadeira de rodas.", cm(turn), cm(min.bound.value())),
                    );
                }
            }
            if self.profile.elderly > 0 && what == RoomUse::Bathroom {
                self.push_ref(
                    Rule::ElderlyBathroom,
                    Severity::Dica,
                    &label,
                    "Morador idoso: barras de apoio junto ao vaso e no box, piso antiderrapante e box sem degrau.",
                    "nbr9050",
                );
            }
        }
    }

    /// The kitchen, read against the sources that founded the subject: the
    /// work triangle, Alexander's counter lengths, Blum's five zones, the
    /// electrical and gas standards that make it legal, and what the
    /// laboratory knows about pulling cooking pollutants out of the air.
    #[allow(clippy::too_many_lines)]
    fn kitchen(&mut self) {
        let scene = self.scene;
        let stature = self.profile.stature.unwrap_or(165.0);
        let city = self.profile.city.as_deref().and_then(standards::municipal);
        // Only judge the electrical layout of a plan that has one.
        let has_electrical = scene
            .units
            .iter()
            .any(|u| matches!(u.what, Use::Outlet | Use::Switch));
        // Kitchens, and living rooms with a kitchen in them — never a
        // bathroom or a laundry, whatever stands in them.
        let cooks = |s: &&Space<'_>| {
            s.what == RoomUse::Kitchen
                || !matches!(s.what, RoomUse::Bathroom | RoomUse::Laundry)
                    && s.units
                        .iter()
                        .filter(|&&i| {
                            matches!(scene.units[i].what, Use::Fridge | Use::Stove | Use::Sink)
                        })
                        .count()
                        >= 2
        };
        let cooking: Vec<usize> = scene
            .spaces
            .iter()
            .enumerate()
            .filter(|(_, s)| cooks(s))
            .map(|(k, _)| k)
            .collect();
        for k in cooking {
            let space = &scene.spaces[k];
            let label = space.label();
            let find = |w: fn(&Use) -> bool| {
                space
                    .units
                    .iter()
                    .map(|&i| &scene.units[i])
                    .find(|u| w(&u.what))
            };
            let any = |w: fn(&Use) -> bool| find(w).is_some();
            let points: Vec<Option<Point2>> = [
                find(|u| matches!(u, Use::Fridge)),
                find(|u| matches!(u, Use::Sink)),
                find(|u| matches!(u, Use::Stove)),
            ]
            .into_iter()
            .map(|u| u.map(|u| u.piece.position))
            .collect();
            if let [Some(fridge), Some(sink), Some(stove)] = points[..] {
                let legs = [
                    fridge.distance(sink),
                    sink.distance(stove),
                    stove.distance(fridge),
                ];
                let total: f64 = legs.iter().sum();
                // The three legs sum to no more than a total, each between a
                // shortest and a longest.
                let short_leg = legs.iter().copied().fold(f64::MAX, f64::min);
                let long_leg = legs.iter().copied().fold(0.0, f64::max);
                let (most, longest, shortest) = (
                    self.fig("kitchen.triangle.total"),
                    self.fig("kitchen.triangle.leg"),
                    self.fig("kitchen.triangle.leg.min"),
                );
                if let (Some(most), Some(longest)) = (most, longest)
                    && (total > most.bound.value() || long_leg > longest.bound.value())
                {
                    self.push_fig(
                        Rule::WorkTriangleLong,
                        &most,
                        &label,
                        say!("Triângulo geladeira–pia–fogão de {} cm, lado maior {} cm: a referência é até {} cm no total e {} cm por lado; aproxime os três.", cm(total), cm(long_leg), cm(most.bound.value()), cm(longest.bound.value())),
                    );
                } else if let Some(shortest) = shortest
                    && short_leg < shortest.bound.value()
                {
                    self.push_fig(
                        Rule::WorkTriangleShort,
                        &shortest,
                        &label,
                        say!("Triângulo geladeira–pia–fogão com um lado de só {} cm: a referência é ao menos {} cm, para haver bancada de apoio entre eles.", cm(short_leg), cm(shortest.bound.value())),
                    );
                }
                if let Some(apart) = self.fig("kitchen.sink_to_stove")
                    && sink.distance(stove) < apart.bound.value()
                {
                    self.push_fig(
                        Rule::SinkNextToStove,
                        &apart,
                        &label,
                        "Pia e fogão colados: deixe bancada entre eles para preparo e segurança (a NKBA soma 90 cm de apoio entre os dois).",
                    );
                }
            }
            // Worktop height: about 10 to 15 cm below the elbow (63 % of
            // stature). The kitchen has one counter height, and it is the one
            // most of the stone is at — an appliance sitting 6 cm lower under
            // the same stone is not a counter to measure, and reading it as
            // one accused a correct kitchen in a real session.
            let ideal = match (
                self.fig("kitchen.counter.elbow_share"),
                self.fig("kitchen.counter.below_elbow"),
            ) {
                (Some(share), Some(below)) => stature * share.bound.value() - below.bound.value(),
                _ => f64::NAN,
            };
            let mut tops: Vec<(f64, usize)> = space
                .units
                .iter()
                .filter(|&&i| {
                    let u = &scene.units[i];
                    matches!(u.what, Use::Sink | Use::Counter)
                        && !scene.embedded(i)
                        && !scene.thin(i)
                        && u.piece.height_range().1 >= 70.0
                })
                .map(|&i| (scene.units[i].piece.height_range().1, i))
                .collect();
            tops.sort_by(|a, b| a.0.total_cmp(&b.0));
            if let Some(&(top, which)) = tops.get(tops.len() / 2)
                && let Some(tolerance) = self.fig("kitchen.counter.tolerance")
                && (top - ideal).abs() > tolerance.bound.value()
            {
                self.push_fig(
                    Rule::CounterHeight,
                    &tolerance,
                    scene.units[which].label(),
                    say!("Bancada a {} cm; para quem tem {} cm de altura o conforto fica perto de {} cm.", cm(top), cm(stature), cm(ideal.round())),
                );
            }
            // Wall cabinets: above the head at the counter, within reach —
            // a share of the stature, or a height from a wheelchair.
            let reaching = if self.profile.wheelchair {
                self.fig("kitchen.wall_cabinet.reach.wheelchair")
                    .map(|f| (f.bound.value(), f))
            } else {
                self.fig("kitchen.wall_cabinet.reach_share")
                    .map(|f| (stature * f.bound.value(), f))
            };
            let over = self
                .fig("kitchen.wall_cabinet.over_reach")
                .map_or(0.0, |f| f.bound.value());
            for &i in &space.units {
                let u = &scene.units[i];
                if u.what != Use::WallCabinet {
                    continue;
                }
                let (lo, hi) = u.piece.height_range();
                if let Some(low) = self.fig("kitchen.wall_cabinet.min_elevation")
                    && lo < low.bound.value()
                {
                    self.push_fig(
                        Rule::WallCabinetLow,
                        &low,
                        u.label(),
                        say!("Aéreo a {} cm do chão: abaixo de ~{} cm (45 cm sobre a bancada, prática de marcenaria) a cabeça bate ao trabalhar.", cm(lo), cm(low.bound.value())),
                    );
                }
                if let Some((reach, source)) = reaching
                    && hi > reach + over
                {
                    self.push_fig(
                        Rule::WallCabinetHigh,
                        &source,
                        u.label(),
                        say!("Topo do aéreo a {} cm: a prateleira de cima fica fora do alcance (~{} cm); guarde ali o que se usa pouco.", cm(hi), cm(reach.round())),
                    );
                }
                break;
            }
            // --- Alexander, pattern 184: is there counter, and is it whole? ---
            let runs = counter_runs(scene, space);
            let total: f64 = runs.iter().sum();
            if !runs.is_empty() {
                if let Some(least) = self.fig("kitchen.counter.total")
                    && total + 1.0 < least.bound.value()
                {
                    self.push_fig(
                        Rule::CounterTotal,
                        &least,
                        &label,
                        say!("{} cm de bancada livre fora de pia, fogão e geladeira; abaixo de {} cm falta onde pousar as coisas do preparo (mesa solta também conta).", cm(total), cm(least.bound.value())),
                    );
                }
                if let Some(least) = self.fig("kitchen.counter.run")
                    && let Some(shortest) = runs
                        .iter()
                        .copied()
                        .filter(|r| *r + 1.0 < least.bound.value())
                        .reduce(f64::min)
                    && runs.len() > 1
                {
                    self.push_fig(
                        Rule::CounterRunShort,
                        &least,
                        &label,
                        say!("Um trecho de bancada de só {} cm: abaixo de {} cm o pedaço não serve para preparar nada; junte-o a outro trecho.", cm(shortest), cm(least.bound.value())),
                    );
                }
            }
            // The four things you walk between, none of them far from another.
            let corners: Vec<(&str, Point2)> = [
                ("fogão", find(|u| matches!(u, Use::Stove))),
                ("pia", find(|u| matches!(u, Use::Sink))),
                (
                    "armazenagem",
                    find(|u| matches!(u, Use::Fridge | Use::Storage | Use::WallCabinet)),
                ),
                ("bancada", find(|u| matches!(u, Use::Counter | Use::Island))),
            ]
            .into_iter()
            .filter_map(|(name, u)| u.map(|u| (name, u.piece.position)))
            .collect();
            let farthest = corners
                .iter()
                .enumerate()
                .flat_map(|(i, a)| corners[i + 1..].iter().map(move |b| (a, b)))
                .map(|(a, b)| (a.1.distance(b.1), a.0, b.0))
                .max_by(|x, y| x.0.total_cmp(&y.0));
            if let Some((far, a, b)) = farthest
                && let Some(most) = self.fig("kitchen.spread")
                && far > most.bound.value()
            {
                self.push_fig(
                    Rule::KitchenSpread,
                    &most,
                    &label,
                    say!(
                        "{} cm entre {} e {}: acima de {} cm cada par vira travessia, e o preparo se desfaz em idas e vindas.",
                        cm(far),
                        Text::from(a),
                        Text::from(b),
                        cm(most.bound.value())
                    ),
                );
            }
            // --- Blum: the five zones, in the order of the work ---
            let zones = [
                ("mantimentos", any(|u| matches!(u, Use::Fridge))),
                // A tall cabinet in a kitchen — read as a wardrobe for its
                // doors — is its pantry.
                (
                    "armazenagem",
                    any(|u| matches!(u, Use::WallCabinet | Use::Storage | Use::Wardrobe)),
                ),
                ("lavagem", any(|u| matches!(u, Use::Sink))),
                ("preparo", any(|u| matches!(u, Use::Counter | Use::Island))),
                ("cocção", any(|u| matches!(u, Use::Stove))),
            ];
            let missing: Vec<&str> = zones
                .iter()
                .filter(|(_, there)| !there)
                .map(|(name, _)| *name)
                .collect();
            if !missing.is_empty() && missing.len() < zones.len() {
                self.push_ref(
                    Rule::MissingWorkZone,
                    Severity::Dica,
                    &label,
                    say!(
                        "Das cinco zonas de trabalho falta {}: sem ela o fluxo mantimentos → armazenagem → lavagem → preparo → cocção se quebra.",
                        Text::joined(missing.iter().map(|&w| Text::from(w)).collect(), "{} e {}")
                    ),
                    "blum-zonas",
                );
            }
            // --- NBR 5410: sockets by perimeter, and two above the counter ---
            if has_electrical {
                let perimeter = outline_length(&space.room.points);
                let sockets = space
                    .units
                    .iter()
                    .filter(|&&i| scene.units[i].what == Use::Outlet)
                    .count();
                if let Some(each) = self.fig("kitchen.sockets.perimeter_each") {
                    let needed = (perimeter / each.bound.value()).ceil().max(1.0) as usize;
                    if sockets < needed {
                        self.push_fig(
                            Rule::KitchenSockets,
                            &each,
                            &label,
                            say!("{} ponto(s) de tomada para {} m de perímetro: são necessários {} (um a cada {} m ou fração).", sockets, m2(perimeter * 100.0), needed, cm(each.bound.value() / 100.0)),
                        );
                    }
                }
                // 9.5.2.2.1 b): two sockets over the sink worktop, "no mesmo
                // ponto ou em pontos distintos" — a double outlet is two.
                let above: u32 = space
                    .units
                    .iter()
                    .filter(|&&i| {
                        let u = &scene.units[i];
                        u.what == Use::Outlet && u.piece.elevation >= 90.0
                    })
                    .map(|&i| {
                        scene.units[i]
                            .piece
                            .properties
                            .get("elec:sockets")
                            .and_then(|v| v.parse::<u32>().ok())
                            .unwrap_or(1)
                    })
                    .sum();
                if let Some(least) = self.fig("kitchen.sockets.over_counter")
                    && f64::from(above) < least.bound.value()
                {
                    self.push_fig(
                        Rule::CounterSockets,
                        &least,
                        &label,
                        say!("{} tomada(s) acima da bancada: são exigidas pelo menos {}, no mesmo ponto (tomada dupla: elec:sockets = 2) ou em pontos distintos; forno, cooktop elétrico e lava-louças acima de 10 A pedem circuito próprio.", above, cm(least.bound.value())),
                    );
                }
            }
            // --- NBR 13103: a gas appliance needs permanent ventilation ---
            if let Some(stove) = find(|u| matches!(u, Use::Stove)) {
                let gas = !electric(&stove.piece.name);
                if gas {
                    let glass = self.glass.get(&space.room.id).copied().unwrap_or_default();
                    // A permanent grille to the outside, low and high.
                    let grilles = scene
                        .home
                        .furniture
                        .iter()
                        .flat_map(newera_core::Furniture::flatten)
                        .filter(|f| {
                            f.catalog == "vent-grille"
                                && space.room.points.len() >= 3
                                && newera_core::electrical::inside_room(
                                    &space.room.points,
                                    f.position,
                                    20.0,
                                )
                        })
                        .count();
                    if grilles > 0 {
                        if grilles < 2 {
                            self.push_ref(
                                Rule::GasVentPartial,
                                Severity::Dica,
                                &label,
                                "Uma grelha de ventilação permanente: a ventilação de aparelho a gás pede abertura inferior e superior; confira a área útil na edição vigente da norma.",
                                "nbr13103",
                            );
                        }
                    } else if glass <= 0.0 {
                        self.push_ref(
                            Rule::GasWithoutVent,
                            Severity::Erro,
                            &label,
                            "Aparelho a gás em ambiente sem janela nem abertura permanente: a ventilação permanente para o exterior é obrigatória (em São Paulo, Decreto 57.776, 3.M), e aqui é segurança, não conforto.",
                            "coe-municipal",
                        );
                    } else {
                        self.push_ref(
                            Rule::GasWindowNotVent,
                            Severity::Alerta,
                            &label,
                            "Aparelho a gás: janela que fecha não é ventilação permanente. Preveja abertura permanente direta para o exterior (veneziana ou grelha, inferior e superior), como pedem a norma de aparelhos a gás e, em São Paulo, o Decreto 57.776 (3.M).",
                            "nbr13103",
                        );
                    }
                }
                // --- LBNL: pulling the pollutants out before they spread ---
                match find(|u| matches!(u, Use::Hood)) {
                    None => self.push_ref(
                        Rule::StoveWithoutHood,
                        Severity::Alerta,
                        &label,
                        "Cocção sem coifa: o cozimento gera material particulado e NO₂ dentro de casa, e sem captura eles ficam no ar da sala junto.",
                        "lbnl-coifa",
                    ),
                    Some(hood) if hood.piece.width + 1.0 < stove.piece.width => self.push_ref(
                        Rule::HoodNarrowerThanStove,
                        Severity::Dica,
                        &label,
                        say!("Coifa de {} cm sobre cocção de {} cm: a captura já cai à metade nas bocas da frente, e vazão alta não compensa coifa estreita.", cm(hood.piece.width), cm(stove.piece.width)),
                        "lbnl-coifa",
                    ),
                    Some(_) => {}
                }
            }
            // --- What is built at scale: the floor of the Brazilian market ---
            if space.what == RoomUse::Kitchen {
                let short = short_side(&space.room.points);
                if let Some(least) = self.fig("kitchen.width")
                    && short + 0.5 < least.bound.value()
                {
                    self.push_fig(
                        Rule::KitchenWidth,
                        &least,
                        &label,
                        say!("Cozinha de {} cm de largura; para comparar, a unidade do Minha Casa Minha Vida (regra só para ela) entrega {} cm, com previsão de pia 120×50, fogão 55×60 e geladeira 70×70 cm.", cm(short), cm(least.bound.value())),
                    );
                }
                // --- The city has the last word, and only where it was named:
                // a circle heard only because the city was left unsaid would
                // accuse on a guess. ---
                match self.fig("kitchen.free_circle").filter(|f| f.named) {
                    Some(circle) => {
                        let turn = scene.turning_diameter(space);
                        let diameter = circle.bound.value();
                        if turn + 1.0 < diameter {
                            let who = city.map_or_else(
                                || self.place.city.clone().unwrap_or_default(),
                                |c| c.label.to_owned(),
                            );
                            self.push_fig(
                                Rule::FreeCircle,
                                &circle,
                                &label,
                                say!("Cabe um círculo de {} cm no piso; {} pede {} cm ({}). Entre norma e lei local prevalece o mais restritivo.", cm(turn), who, cm(diameter), circle.note),
                            );
                        }
                    }
                    None => self.push_ref(
                        Rule::CityNotDeclared,
                        Severity::Dica,
                        &label,
                        "Código de obras não informado: o círculo livre no piso e as áreas mínimas da cozinha variam por município — informe a cidade para que sejam verificados.",
                        "coe-municipal",
                    ),
                }
            }
        }
    }

    /// Screens: the sofa far enough to see the whole picture, close enough
    /// to read it (about 1,2 to 2,5 times the diagonal).
    fn screens(&mut self) {
        let scene = self.scene;
        for space in &scene.spaces {
            let tvs = space
                .units
                .iter()
                .filter(|&&i| scene.units[i].what == Use::Tv);
            for &tv in tvs {
                let screen = scene.units[tv].piece;
                // Embedded TVs sit inside a panel group: use the group position.
                let diagonal = screen.width.hypot(screen.height);
                // The seats that can watch it: not turned away from the
                // screen — a reading chair beside the TV, its back to it, is
                // not where anyone watches from.
                let watches = |i: usize| {
                    let seat = scene.units[i].frame();
                    let ahead = seat.to_plan((0.0, 1.0));
                    let front = (ahead.x - seat.position.x, ahead.y - seat.position.y);
                    let to = (
                        screen.position.x - seat.position.x,
                        screen.position.y - seat.position.y,
                    );
                    let length = to.0.hypot(to.1);
                    length > 1e-6 && front.0 * to.0 + front.1 * to.1 >= 0.0
                };
                let Some(sofa) = space
                    .units
                    .iter()
                    .filter(|&&i| matches!(scene.units[i].what, Use::Sofa(_) | Use::Armchair))
                    .filter(|&&i| watches(i))
                    .min_by(|a, b| {
                        let d = |i: usize| scene.units[i].piece.position.distance(screen.position);
                        d(**a).total_cmp(&d(**b))
                    })
                else {
                    continue;
                };
                let distance = scene.units[*sofa].piece.position.distance(screen.position);
                let (Some(nearest), Some(farthest)) = (
                    self.fig("screen.distance.near"),
                    self.fig("screen.distance.far"),
                ) else {
                    continue;
                };
                let (near, far) = (
                    diagonal * nearest.bound.value(),
                    diagonal * farthest.bound.value(),
                );
                let inches = (diagonal / 2.54).round();
                if distance < near || distance > far {
                    self.push_fig(
                        Rule::TvDistance,
                        if distance < near { &nearest } else { &farthest },
                        scene.units[tv].label(),
                        say!("{} a {} cm da TV de {}\"; para essa tela o conforto fica entre {} e {} cm.", scene.units[*sofa].label(), cm(distance.round()), format!("{inches:.0}"), cm(near.round()), cm(far.round())),
                    );
                }
            }
        }
    }

    /// What the other rules leave out: the walk between two single beds
    /// (NBR 15575-1 annex F, 60 cm), an accessible basin or sink and bed
    /// (NBR 9050: top at 85 cm at most, bed at 46 cm), a gas heater in a
    /// bathroom (NBR 13103: type C, sealed, only) and gas cooking in a room
    /// people also sleep in (NBR 13103 6.2.2.3).
    fn gaps(&mut self) {
        let scene = self.scene;
        for space in &scene.spaces {
            let label = space.label();
            let units: Vec<&crate::scene::Unit> =
                space.units.iter().map(|&i| &scene.units[i]).collect();
            // Two single beds side by side.
            let singles: Vec<&&crate::scene::Unit> =
                units.iter().filter(|u| u.what == Use::Bed(1)).collect();
            for (k, a) in singles.iter().enumerate() {
                for b in singles.iter().skip(k + 1) {
                    let (x, y) = a.piece.to_local(b.piece.position);
                    let across = f64::midpoint(a.piece.depth, b.piece.depth);
                    let gap = x.abs() - f64::midpoint(a.piece.width, b.piece.width);
                    if let Some(least) = self.fig("clearance.between_beds")
                        && y.abs() < across * 0.5
                        && (0.0..least.bound.value()).contains(&gap)
                    {
                        self.push_fig_at(
                            Rule::BetweenBeds,
                            // One finding per pair: accepting the gap between
                            // two beds never accepts it between two others.
                            &format!("{}+{}", a.piece.id, b.piece.id),
                            &least,
                            &label,
                            say!("{} cm entre {} e {}: entre duas camas de solteiro a referência é {} cm.", cm(gap), a.label(), b.label(), cm(least.bound.value())),
                        );
                    }
                }
            }
            if self.profile.wheelchair {
                let highest = self.fig("reach.top.wheelchair");
                let bed = self.fig("reach.bed.wheelchair");
                let off = self.fig("reach.bed.wheelchair.tolerance");
                for u in &units {
                    let (_, top) = u.piece.height_range();
                    match (u.what, highest, bed.zip(off)) {
                        (Use::Basin | Use::Sink, Some(most), _)
                            if top > most.bound.value() + 0.5 =>
                        {
                            self.push_fig(
                                Rule::BasinHeightWheelchair,
                                &most,
                                u.label(),
                                say!("Tampo a {} cm: para cadeira de rodas, até {} cm, com vão livre embaixo.", cm(top), cm(most.bound.value())),
                            );
                        }
                        (Use::Bed(_), _, Some((aim, off)))
                            if (top - aim.bound.value()).abs() > off.bound.value() =>
                        {
                            self.push_fig(
                                Rule::BedHeightWheelchair,
                                &off,
                                u.label(),
                                say!("Cama a {} cm do chão: a transferência da cadeira pede uns {} cm.", cm(top), cm(aim.bound.value())),
                            );
                        }
                        _ => {}
                    }
                }
            }
            let gas_named = |u: &crate::scene::Unit| {
                let n = newera_core::fold(&u.piece.name);
                n.contains("gas") && (n.contains("aquecedor") || n.contains("boiler"))
            };
            if space.what == RoomUse::Bathroom {
                for u in units.iter().filter(|u| gas_named(u)) {
                    self.push_ref(
                        Rule::GasHeaterInBathroom,
                        Severity::Erro,
                        u.label(),
                        "Aquecedor a gás no banheiro: ali só é admitido aparelho tipo C (câmara de combustão estanque); do contrário, leve-o para fora, para a área de serviço ventilada.",
                        "nbr13103",
                    );
                }
            }
            let sleeps = units.iter().any(|u| matches!(u.what, Use::Bed(_)));
            let gas_cooking = units
                .iter()
                .any(|u| u.what == Use::Stove && !electric(&u.piece.name));
            if sleeps && gas_cooking {
                self.push_ref(
                    Rule::GasCookingInBedroom,
                    Severity::Dica,
                    &label,
                    "Cocção a gás no mesmo ambiente em que se dorme: a norma limita a 8,14 kW, com válvula de segurança em todos os queimadores e coifa com saída para o exterior.",
                    "nbr13103",
                );
            }
        }
    }

    fn reach(&mut self) {
        if !self.profile.wheelchair {
            return;
        }
        let band = |low: &str, high: &str| Some((self.fig(low)?, self.fig(high)?));
        let (Some(switches), Some(outlets)) = (
            band("reach.switch.min", "reach.switch.max"),
            band("reach.outlet.min", "reach.outlet.max"),
        ) else {
            return;
        };
        let mut wrong = 0;
        for u in &self.scene.units {
            if !matches!(u.what, Use::Switch | Use::Outlet) {
                continue;
            }
            let center = u.piece.elevation + u.piece.height / 2.0;
            let (low, high) = if u.what == Use::Switch {
                switches
            } else {
                outlets
            };
            if !(low.bound.value()..=high.bound.value()).contains(&center) {
                wrong += 1;
            }
        }
        if wrong > 0 {
            let value = |f: Resolved| cm(f.bound.value());
            self.push_fig(
                Rule::SwitchOutletReach,
                &switches.1,
                "Casa",
                say!("{} interruptor(es)/tomada(s) fora do alcance de quem usa cadeira de rodas: interruptores entre {} e {} cm, tomadas entre {} e {} cm.", wrong, value(switches.0), value(switches.1), value(outlets.0), value(outlets.1)),
            );
        }
    }
}

/// What a room offers, measured once.
struct RoomFacts {
    label: String,
    what: RoomUse,
    /// cm².
    area: f64,
    short: f64,
    ceiling: f64,
    /// Window area, cm².
    glass: f64,
    uses: Vec<Use>,
    index: usize,
}

/// A finding another discipline made, brought into the review — the one
/// policy for all three.
///
/// It arrives already weighed at the project's place by the discipline
/// itself (see [`newera_core::electrical::weighed`]), so the discipline tool
/// and the review agree on what a source may claim there. What the review adds
/// is how much the finding counts *here*: an electrical or plumbing project is
/// judged by its own tool and scored as its own discipline, and in the
/// habitability review a missing socket is one step lighter than in the
/// installation project — an error there is a warning here. A guard is not an
/// installation project: a fall from a balcony is habitability itself, and it
/// comes in as heavy as it left.
fn imported(f: newera_core::electrical::Finding, lighter: bool) -> Finding {
    let severity = match (lighter, f.severity) {
        (false, severity) => severity,
        (true, Severity::Erro) => Severity::Alerta,
        (true, Severity::Alerta | Severity::Dica) => Severity::Dica,
    };
    Finding {
        severity,
        place: f.place,
        message: f.message,
        reference: Some(f.source),
        key: f.key,
        ..Finding::default()
    }
}

trait Midpoint {
    fn midpoint_with(self, other: Self) -> Self;
}

impl Midpoint for Point2 {
    fn midpoint_with(self, other: Self) -> Self {
        Self::new(self.x.midpoint(other.x), self.y.midpoint(other.y))
    }
}

/// The shortest slide (along the piece's own axes, up to 1,5 m) that leaves
/// unit `i` clear of walls, pieces and door swings, in its room and still on
/// the walls it rests against. Returns move arguments plus the distance as `d`.
fn nudge(scene: &Scene<'_>, i: usize) -> Option<serde_json::Value> {
    let piece = scene.units[i].piece;
    if piece.is_opening() || !piece.locks.movable {
        return None;
    }
    let room = scene.room_at(piece.position);
    let on_walls = scene.contacts(i, 0.0, 0.0);
    let held = scene.held(i, 0.0, 0.0);
    let mut step = 5.0;
    while step <= 150.0 {
        for (lx, ly) in [(step, 0.0), (-step, 0.0), (0.0, step), (0.0, -step)] {
            let to = piece.to_plan((lx, ly));
            let (dx, dy) = (to.x - piece.position.x, to.y - piece.position.y);
            if scene.room_at(to) == room
                && scene.contacts(i, dx, dy) >= on_walls
                && scene.held(i, dx, dy) + 1.0 >= held
                && scene.conflicts(i, dx, dy) == 0
            {
                let round = |v: f64| (v * 10.0).round() / 10.0;
                return Some(serde_json::json!({
                    "tool": "move",
                    "ids": [piece.id.to_string()],
                    "dx": round(dx),
                    "dy": round(dy),
                    "d": step,
                }));
            }
        }
        step += 5.0;
    }
    None
}

/// Moves unit `i` `dist` cm along `dir` if that adds no problem, keeps it in
/// its room, on its walls and in its niche: move arguments.
fn push_away(scene: &Scene<'_>, i: usize, dir: (f64, f64), dist: f64) -> Option<serde_json::Value> {
    let piece = scene.units[i].piece;
    if piece.is_opening() || !piece.locks.movable {
        return None;
    }
    let len = dir.0.hypot(dir.1).max(1e-9);
    let (dx, dy) = (dir.0 / len * dist, dir.1 / len * dist);
    let to = Point2::new(piece.position.x + dx, piece.position.y + dy);
    let round = |v: f64| (v * 10.0).round() / 10.0;
    (scene.room_at(to) == scene.room_at(piece.position)
        && scene.contacts(i, dx, dy) >= scene.contacts(i, 0.0, 0.0)
        // Not out of the niche it is built into.
        && scene.held(i, dx, dy) + 1.0 >= scene.held(i, 0.0, 0.0)
        && scene.conflicts(i, dx, dy) <= scene.conflicts(i, 0.0, 0.0))
    .then(|| {
        serde_json::json!({
            "tool": "move",
            "ids": [piece.id.to_string()],
            "dx": round(dx),
            "dy": round(dy),
        })
    })
}

fn without_distance(mut fix: serde_json::Value) -> serde_json::Value {
    if let Some(map) = fix.as_object_mut() {
        map.remove("d");
    }
    fix
}

/// Whether `p` lies on the outline of a polygon, within `tolerance` cm.
/// Does the name say the cooktop burns nothing?
fn electric(name: &str) -> bool {
    let n = name.to_lowercase();
    [
        "induç",
        "induc",
        "induction",
        "elétric",
        "eletric",
        "electric",
    ]
    .iter()
    .any(|w| n.contains(w))
}

/// Distance from a point to the outline of a room, cm.
fn outline_distance(points: &[Point2], p: Point2) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let len2 = (dx * dx + dy * dy).max(1e-9);
            let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0);
            Point2::new(a.x + t * dx, a.y + t * dy).distance(p)
        })
        .fold(f64::MAX, f64::min)
}

/// Perimeter of a room outline, cm.
fn outline_length(points: &[Point2]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a.distance(*b))
        .sum()
}

/// Glazed area on a room's outline, cm².
/// Glass that lights and airs each room, by room id.
///
/// A room is not a sealed box. Where two rooms run into each other with no
/// wall between — a living room open to its balcony, a kitchen open to the
/// living room — the light and the air of one are the light and the air of
/// the other, and a check that reads room by room accuses a plan that is
/// right. Which it did, three sessions running, on a kitchen whose balcony
/// carries a window 4,5 m wide.
///
/// Translucent pieces count too: a fixed pane, a fluted glass panel, a glass
/// door. They are drawn as furniture with `opacity`, not as openings, and
/// they are how a room with no facade of its own gets daylight.
fn glass_by_room(scene: &Scene<'_>) -> std::collections::BTreeMap<newera_core::RoomId, f64> {
    let home = scene.home;
    let own: Vec<(newera_core::RoomId, f64)> = home
        .rooms
        .iter()
        .map(|room| {
            let windows = window_area(scene, &room.points);
            let panes: f64 = home
                .furniture
                .iter()
                .flat_map(newera_core::Furniture::flatten)
                .filter(|f| {
                    f.visible
                        && f.opacity.is_some_and(|o| o < 1.0)
                        // Already counted as a window.
                        && !glazed(f)
                        && near_outline(&room.points, f.position, f.depth.max(15.0) + 5.0)
                })
                .map(|f| f.width * f.height)
                .sum();
            (room.id, windows + panes)
        })
        .collect();

    // Rooms open to each other share what they have, however many hops away:
    // light crosses two open thresholds as readily as one.
    let mut group: Vec<usize> = (0..home.rooms.len()).collect();
    for i in 0..home.rooms.len() {
        for j in (i + 1)..home.rooms.len() {
            if open_between(home, &home.rooms[i], &home.rooms[j]) {
                let (a, b) = (group[i], group[j]);
                let root = a.min(b);
                for g in &mut group {
                    if *g == a || *g == b {
                        *g = root;
                    }
                }
            }
        }
    }
    let mut shared: std::collections::BTreeMap<usize, f64> = std::collections::BTreeMap::new();
    for (i, (_, glass)) in own.iter().enumerate() {
        *shared.entry(group[i]).or_default() += glass;
    }
    own.iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, shared.get(&group[i]).copied().unwrap_or_default()))
        .collect()
}

/// How wide an opening between two rooms has to be for one to air the other.
const OPEN_ENOUGH: f64 = 80.0;

/// Whether two rooms run into each other with no wall between them.
///
/// Their outlines touch along a stretch — that is what "no wall" looks like
/// in a drawing — and no wall stands on it.
fn open_between(home: &Home, a: &newera_core::Room, b: &newera_core::Room) -> bool {
    const STEP: f64 = 10.0;
    let walls: Vec<geo::Polygon<f64>> = home
        .wall_outlines()
        .iter()
        .filter(|o| o.len() >= 3)
        .map(|o| newera_core::to_polygon(o))
        .collect();
    let in_wall = |p: Point2| {
        use geo::Contains;
        walls.iter().any(|w| w.contains(&geo::Point::new(p.x, p.y)))
    };
    let mut open: f64 = 0.0;
    for k in 0..a.points.len() {
        let (from, to) = (a.points[k], a.points[(k + 1) % a.points.len()]);
        let length = from.distance(to);
        if length < STEP {
            continue;
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let steps = (length / STEP) as usize;
        let mut run: f64 = 0.0;
        for step in 0..=steps {
            #[allow(clippy::cast_precision_loss)]
            let t = (step as f64 * STEP / length).min(1.0);
            let p = Point2::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
            if near_outline(&b.points, p, 5.0) && !in_wall(p) {
                run += STEP;
                open = open.max(run);
            } else {
                run = 0.0;
            }
        }
    }
    open >= OPEN_ENOUGH
}

fn window_area(scene: &Scene<'_>, points: &[Point2]) -> f64 {
    scene
        .home
        .furniture
        .iter()
        .filter(|f| f.visible && glazed(f) && near_outline(points, f.position, f.depth.max(15.0)))
        .map(|f| f.width * f.height)
        .sum()
}

/// An opening that is glass from side to side: a window, a French window,
/// or a sliding door, whose sliding leaves the catalog makes of glass (made
/// to swing, the same door draws solid leaves).
fn glazed(f: &newera_core::Furniture) -> bool {
    f.opening.as_ref().is_some_and(|o| {
        o.kind == OpeningKind::Window
            || f.catalog == "french-window"
            || (f.catalog == "door-sliding" && o.sliding)
    })
}

/// Lengths of the continuous worktop runs in a space, cm. Cabinets that
/// touch are one run, because what matters is the stretch you can work on,
/// not how many boxes it was built from.
fn counter_runs(scene: &Scene<'_>, space: &Space<'_>) -> Vec<f64> {
    let tops: Vec<(Point2, f64)> = space
        .units
        .iter()
        .map(|&i| &scene.units[i])
        .filter(|u| matches!(u.what, Use::Counter | Use::Island))
        .map(|u| (u.piece.position, u.piece.width))
        .collect();
    let mut group: Vec<usize> = (0..tops.len()).collect();
    // Union-find, flattened by a few passes: the sets are tiny.
    for _ in 0..tops.len() {
        for i in 0..tops.len() {
            for j in i + 1..tops.len() {
                let apart = tops[i].0.distance(tops[j].0);
                if apart < f64::midpoint(tops[i].1, tops[j].1) + 15.0 {
                    let root = group[i].min(group[j]);
                    let (a, b) = (group[i], group[j]);
                    for g in &mut group {
                        if *g == a || *g == b {
                            *g = root;
                        }
                    }
                }
            }
        }
    }
    let mut runs: Vec<f64> = Vec::new();
    for root in 0..tops.len() {
        let length: f64 = group
            .iter()
            .enumerate()
            .filter(|(_, g)| **g == root)
            .map(|(i, _)| tops[i].1)
            .sum();
        if length > 0.0 {
            runs.push(length);
        }
    }
    runs
}

fn near_outline(points: &[Point2], p: Point2, tolerance: f64) -> bool {
    (0..points.len())
        .any(|k| p.distance_to_segment(points[k], points[(k + 1) % points.len()]) <= tolerance)
}

/// Shorter side of the smallest rectangle around a polygon, cm.
fn short_side(points: &[Point2]) -> f64 {
    use geo::MinimumRotatedRect;
    let Some(rect) = scene::polygon(points).minimum_rotated_rect() else {
        return 0.0;
    };
    let c: Vec<_> = rect.exterior().coords().copied().collect();
    if c.len() < 3 {
        return 0.0;
    }
    let a = (c[1].x - c[0].x).hypot(c[1].y - c[0].y);
    let b = (c[2].x - c[1].x).hypot(c[2].y - c[1].y);
    a.min(b)
}

/// Height above the floor from which a piece is over a person's head, cm:
/// what `measure` leaves out of the band a walking person meets.
const HEADROOM: f64 = 190.0;

/// The name a finding used to be accepted by, before rules were named:
/// the standard it cited, its place, and the first words of its sentence.
///
/// Projects saved then carry these keys, so an acceptance written under one
/// is still honoured — [`Finding::accepted_as`] says so — and is still found
/// by [`orphaned`]. Nothing is written under this shape any more: a rewording
/// moved it, which is the whole reason rules are named now. Kept for the
/// projects that already have them, and read-only.
#[doc(hidden)]
#[must_use]
pub fn legacy_key_of(finding: &Finding) -> String {
    let rule = finding.reference.unwrap_or("-");
    let words: String = newera_core::fold(&finding.message)
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| w.len() > 3)
        .take(4)
        .collect::<Vec<_>>()
        .join("-");
    let place = newera_core::fold(&finding.place)
        .split_whitespace()
        .last()
        .unwrap_or_default()
        .to_owned();
    format!("{rule}:{place}:{words}")
}

/// Acceptances of ergonomics findings that no current finding answers to,
/// on any storey, for these people.
///
/// A finding accepted with "corridor of 69 cm" as its reason, and then fixed
/// for real, leaves the reason behind; if the counter ever comes back, the
/// finding returns already silenced. Listing them is what lets them go.
pub fn orphaned(home: &Home, profile: &Profile) -> Vec<(String, String)> {
    let mine: Vec<(&String, &String)> = home
        .accepted
        .iter()
        .filter(|(key, _)| {
            !newera_core::Issue::is_layout_key(key)
                && !key.starts_with("elec:")
                && !key.starts_with("plumb:")
        })
        .collect();
    if mine.is_empty() {
        return Vec::new();
    }
    let storeys: Vec<Option<newera_core::LevelId>> = if home.levels.is_empty() {
        vec![None]
    } else {
        home.levels.iter().map(|l| Some(l.id)).collect()
    };
    let mut live = std::collections::BTreeSet::new();
    for storey in storeys {
        let mut shown = home.clone();
        shown.selected_level = storey;
        for f in review(&shown, profile).findings {
            live.extend(f.accepted_as);
            live.insert(f.key);
        }
    }
    mine.into_iter()
        .filter(|(key, _)| !live.contains(*key))
        .map(|(key, why)| (key.clone(), why.clone()))
        .collect()
}

pub fn review(home: &Home, profile: &Profile) -> Report {
    review_with(home, profile, true)
}

/// Whether moving as `fix` says creates a finding at least as heavy as the
/// one it solves: the two cannot both fit, and the fix would walk in circles.
fn fix_creates(
    home: &Home,
    profile: &Profile,
    before: &[Finding],
    finding: &Finding,
) -> Option<Finding> {
    let fix = finding.fix.as_ref().or(finding.probe.as_ref())?;
    if fix["tool"] != "move" {
        return None;
    }
    let ids: Vec<newera_core::ElementId> = fix["ids"]
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str()?.parse().ok())
        .collect();
    let (dx, dy) = (fix["dx"].as_f64()?, fix["dy"].as_f64()?);
    let mut doc = newera_core::Document::new(home.clone());
    newera_core::ops::translate(&mut doc, &ids, dx, dy, false).ok()?;
    let after = review_with(doc.home(), profile, false);
    let rank = |s: Severity| match s {
        Severity::Erro => 3,
        Severity::Alerta => 2,
        Severity::Dica => 1,
    };
    let moved: Vec<String> = ids.iter().map(ToString::to_string).collect();
    after
        .findings
        .into_iter()
        .filter(|f| !before.iter().any(|b| b.key == f.key))
        .filter(|f| {
            moved
                .iter()
                .any(|id| f.place.contains(id.as_str()) || f.message.contains(id.as_str()))
        })
        .max_by_key(|f| (f.accepted.is_none(), rank(f.severity)))
        .map(|f| {
            let heavier = rank(f.severity) >= rank(finding.severity);
            (f, heavier)
        })
        .map(|(f, _)| f)
}

fn severity_rank(s: Severity) -> u8 {
    match s {
        Severity::Erro => 3,
        Severity::Alerta => 2,
        Severity::Dica => 1,
    }
}

fn review_with(home: &Home, profile: &Profile, weigh_fixes: bool) -> Report {
    let view = home.level_view(home.current_level());
    let scene = Scene::new(&view);
    // The city is the project's, so every caller weighs the same rules; one
    // passed in the call still wins, for asking "and under this code?".
    let profile = &Profile {
        city: profile.city.clone().or_else(|| home.compass.city.clone()),
        ..profile.clone()
    };
    // The place is the project's, so every caller weighs the same law; a city
    // named in the call still wins, for asking "and under this code?" — but
    // only the city: the country and the state the project declares still
    // stand, or a project in Miami would be judged in Brazil the moment it
    // named its city.
    let place = {
        let mut compass = home.compass.clone();
        compass.city.clone_from(&profile.city);
        compass.place()
    };
    let mut review = Review {
        glass: glass_by_room(&scene),
        scene: &scene,
        figures: standards::figures_at(&place),
        place: place.clone(),
        profile,
        findings: Vec::new(),
    };
    let capacity = house_capacity(home, &scene);
    review.occupancy(&capacity);
    review.clearances();
    review.doors();
    review.rooms();
    review.room_program();
    review.kitchen();
    review.findings.extend(corners::review(&scene));
    review.screens();
    review.reach();
    review.gaps();
    review.electrical();
    review.plumbing();
    review.guards();
    // Establish identity before presentation grouping: accepting a warning
    // for one room or piece must never accept the same text elsewhere.
    let mut findings: Vec<Finding> = Vec::new();
    for f in review.findings {
        debug_assert!(!f.key.is_empty(), "a finding names its rule: {f:?}");
        match findings.iter_mut().find(|g| {
            g.severity == f.severity
                // All findings, including architecture, keep independent keys.
                && g.key == f.key
                && g.message == f.message
                && g.reference == f.reference
                && g.fix.is_none()
                && f.fix.is_none()
        }) {
            Some(same) => {
                same.place.push_str(", ");
                same.place.push_str(&f.place);
            }
            None => findings.push(f),
        }
    }
    findings.sort_by_key(|f| f.severity);
    // Each finding gets the name it is accepted by, and the ones already
    // looked at carry the reason instead of the cost.
    for finding in &mut findings {
        finding.accepted = home.accepted.get(&finding.key).cloned();
        // Accepted before rules were named, in a project saved back then: the
        // acceptance follows this very finding, by the name it had — and only
        // this one, since the old name is rebuilt from the finding itself.
        if finding.accepted.is_none() {
            let old = legacy_key_of(finding);
            if let Some(why) = home.accepted.get(&old) {
                finding.accepted = Some(why.clone());
                finding.accepted_as = Some(old);
            }
        }
    }
    // A fix that would open another finding as heavy is no fix: the two do
    // not fit together, and the message says so instead of walking in circles.
    if weigh_fixes {
        let snapshot = findings.clone();
        for finding in &mut findings {
            if finding.accepted.is_some() {
                continue;
            }
            if let Some(other) = fix_creates(home, profile, &snapshot, finding) {
                if other.accepted.is_none()
                    && severity_rank(other.severity) >= severity_rank(finding.severity)
                {
                    finding.fix = None;
                    finding.message = finding.message.clone().then(say!(
                        "Não cabem os dois: afastar isso cria «{}» em {}; a posição atual é a melhor das duas — aceite o que ficar com o motivo.",
                        other.message.quoted(),
                        other.place
                    ));
                } else {
                    finding.message = finding.message.clone().then(say!(
                        "Isso deixa «{}» em {}{}.",
                        other.message.quoted(),
                        other.place,
                        if other.accepted.is_some() {
                            say!(", já aceito")
                        } else {
                            say!(", mais leve")
                        }
                    ));
                }
            }
        }
    }
    // Errors weigh fully; many alerts or tips of a crowded plan level off.
    let penalty_of = |list: &[Finding], scope: ReviewScope| {
        let count = |sev: Severity| {
            list.iter()
                .filter(|f| {
                    scope.includes(f)
                        && f.severity == sev
                        && f.accepted.is_none()
                        && !f.key.starts_with("unused_corner:")
                })
                .count() as u32
        };
        let (errors, alerts, tips) = (
            count(Severity::Erro),
            count(Severity::Alerta),
            count(Severity::Dica),
        );
        errors * 12 + alerts.min(6) * 5 + alerts.saturating_sub(6) + tips.min(10)
    };
    let scores = ["architecture", "electrical", "plumbing"]
        .into_iter()
        .map(|discipline| {
            let selected: Vec<_> = findings
                .iter()
                .filter(|f| f.discipline() == discipline)
                .cloned()
                .collect();
            (
                discipline,
                100u32.saturating_sub(penalty_of(&selected, ReviewScope::default())),
            )
        })
        .collect();
    let penalty = penalty_of(&findings, profile.scope);
    // What each one costs: the score without it, minus the score with it.
    // A number that moves without saying why is a number nobody can act on.
    for at in 0..findings.len() {
        let mut without = findings.clone();
        without.remove(at);
        findings[at].weight = penalty.saturating_sub(penalty_of(&without, profile.scope));
    }
    // Each source once, in the order the findings first lean on it.
    let mut refs: Vec<&'static Standard> = Vec::new();
    for code in findings.iter().filter_map(|f| f.reference) {
        if let Some(source) = standards::standard(code)
            && !refs.iter().any(|s| s.code == code)
        {
            refs.push(source);
        }
    }
    Report {
        scope: profile.scope,
        scores,
        score: 100u32.saturating_sub(penalty),
        capacity,
        findings,
        refs,
        place,
    }
}

#[cfg(test)]
mod tests {
    use newera_core::{Furniture, FurnitureId, Room, RoomId, Wall, WallId};

    use super::*;

    #[test]
    fn a_guard_acceptance_is_orphaned_when_its_finding_is_gone() {
        // The balcony's guards come into the review, so an acceptance of one
        // whose finding went away is listed with the review's orphans and can
        // be pruned — no guard key is left without a check that answers for it.
        let mut home = Home::default();
        square(&mut home, "Varanda", 300.0, 150.0);
        let mut rail = piece(30, "railing", (150.0, 10.0), (280.0, 5.0, 90.0), 0.0);
        rail.name = "Gradil".into();
        home.furniture.push(rail);
        let live = review(&home, &Profile::default());
        let key = live
            .findings
            .iter()
            .find(|f| f.key.starts_with("guard:"))
            .map_or_else(|| panic!("{live:#?}"), |f| f.key.clone());
        home.accepted
            .insert(key.clone(), "baixo de propósito".into());
        assert_eq!(
            orphaned(&home, &Profile::default()),
            [] as [(std::string::String, std::string::String); 0]
        );
        home.furniture.clear();
        assert_eq!(
            orphaned(&home, &Profile::default()),
            vec![(key, "baixo de propósito".to_owned())]
        );
    }

    #[test]
    fn a_finding_is_scored_under_its_rule_not_under_the_code_it_cites() {
        let finding = |key: &str, code: &'static str| Finding {
            key: key.to_owned(),
            reference: Some(code),
            ..Finding::default()
        };
        // A socket count is electrical whichever electrical code it stands on.
        assert_eq!(
            finding("kitchen_sockets:r1", "nbr5410").discipline(),
            "electrical"
        );
        assert_eq!(
            finding("counter_sockets:r1", "irc2024").discipline(),
            "electrical"
        );
        // And a rule of the room stays architecture, whatever it cites.
        assert_eq!(
            finding("room_area:r1", "nbr5410").discipline(),
            "architecture"
        );
        assert_eq!(
            finding("elec:light:r1", "nbr5410").discipline(),
            "electrical"
        );
        assert_eq!(finding("plumb:cold:f1", "nbr5626").discipline(), "plumbing");
        assert_eq!(
            finding("guard:height:f1", "nbr14718").discipline(),
            "architecture"
        );
    }

    #[test]
    fn score_scope_keeps_unmodeled_installations_visible_without_penalizing_architecture() {
        let mut home = Home::default();
        square(&mut home, "Banho", 400.0, 400.0);
        home.furniture
            .push(piece(20, "toilet", (100.0, 100.0), (40.0, 68.0, 80.0), 0.0));
        let full = review(&home, &Profile::default());
        let profile = Profile {
            scope: ReviewScope {
                electrical: false,
                plumbing: false,
            },
            ..Profile::default()
        };
        let architecture = review(&home, &profile);
        assert_eq!(architecture.findings.len(), full.findings.len());
        assert_eq!(architecture.scores, full.scores);
        assert_eq!(architecture.score, architecture.scores["architecture"]);
        assert!(architecture.score > full.score);
        for discipline in ["electrical", "plumbing"] {
            let findings: Vec<_> = architecture
                .findings
                .iter()
                .filter(|f| f.discipline() == discipline)
                .collect();
            assert_ne!(findings, [] as [&Finding; 0]);
            assert!(
                findings
                    .iter()
                    .all(|f| f.weight == 0 && f.accepted.is_none())
            );
        }
        assert!(
            architecture
                .findings
                .iter()
                .any(|f| f.discipline() == "architecture" && f.weight > 0)
        );
        assert!(home.accepted.is_empty());
        let old: Profile = serde_json::from_str(r#"{"occupants":3}"#).unwrap();
        assert_eq!(old.scope, ReviewScope::default());
    }

    #[test]
    fn declared_room_use_survives_names_and_conflicts_are_feedback() {
        let mut home = Home::default();
        square(&mut home, "Copa", 400.0, 400.0);
        home.furniture
            .push(piece(20, "toilet", (100.0, 100.0), (40.0, 68.0, 80.0), 0.0));
        let conflict = review(&home, &Profile::default());
        let finding = conflict
            .findings
            .iter()
            .find(|f| f.key == "room_use:r10")
            .unwrap();
        assert!(finding.message.contains("f20"));
        assert!(finding.fix.is_none());
        assert_eq!(home.rooms[0].name, "Copa");
        home.rooms[0].usage = newera_core::RoomUse::Bathroom;
        assert_eq!(Scene::new(&home).spaces[0].what, RoomUse::Bathroom);
        assert!(
            !review(&home, &Profile::default())
                .findings
                .iter()
                .any(|f| f.key == "room_use:r10")
        );
        home.rooms[0].name = "Jardim azul".into();
        home.furniture.push(piece(
            21,
            "sink-counter",
            (250.0, 100.0),
            (100.0, 50.0, 85.0),
            0.0,
        ));
        home.furniture[1].name = "Lavatório".into();
        assert!(
            review(&home, &Profile::default())
                .findings
                .iter()
                .any(|f| f.key == "bathroom_fixture:r10:f21")
        );
        home.furniture[1].catalog = "basin-cabinet".into();
        assert!(
            !review(&home, &Profile::default())
                .findings
                .iter()
                .any(|f| f.key.starts_with("bathroom_fixture:"))
        );
        home.rooms[0].usage = newera_core::RoomUse::Other;
        assert_eq!(Scene::new(&home).spaces[0].what, RoomUse::Other);
        home.rooms[0].usage = newera_core::RoomUse::Auto;
        assert_eq!(Scene::new(&home).spaces[0].what, RoomUse::Bathroom);
    }

    #[test]
    fn a_vanity_with_two_bowls_is_a_bathroom_not_a_kitchen() {
        let mut home = Home::default();
        square(&mut home, "Banho master", 250.0, 130.0);
        home.rooms[0].usage = newera_core::RoomUse::Bathroom;
        home.furniture = vec![
            piece(20, "toilet", (35.0, 88.0), (40.0, 68.0, 40.0), 180.0),
            piece(21, "shower", (217.0, 65.0), (50.0, 100.0, 200.0), 0.0),
            // A vanity against the wall, 65 cm clear in front, two bowls in it.
            piece(22, "", (125.0, 32.5), (120.0, 50.0, 80.0), 0.0),
            piece(23, "", (100.0, 32.5), (40.0, 35.0, 16.0), 0.0),
            piece(24, "", (150.0, 32.5), (40.0, 35.0, 16.0), 0.0),
        ];
        home.furniture[2].name = "Gabinete suspenso".into();
        for bowl in &mut home.furniture[3..] {
            bowl.name = "Cuba de embutir".into();
            bowl.elevation = 70.0;
        }
        // Deep bowls, sunk into the vanity they are set in.
        for bowl in &mut home.furniture[3..] {
            bowl.height = 24.0;
            bowl.elevation = 62.0;
        }
        let report = review(&home, &Profile::default());
        let said = |text: &str| report.findings.iter().any(|f| f.message.contains(text));
        assert!(!said("Ocupa o mesmo lugar"), "{:#?}", report.findings);
        assert!(!said("zonas de trabalho"), "{:#?}", report.findings);
        assert!(!said("lavatório para testar"), "{:#?}", report.findings);
        assert!(!said("bancada e equipamentos"), "{:#?}", report.findings);
        // The basin's own 40 cm still holds it to account: a bench 30 cm away.
        home.furniture
            .push(piece(25, "", (125.0, 97.5), (100.0, 20.0, 45.0), 0.0));
        home.furniture[5].name = "Banco".into();
        let report = review(&home, &Profile::default());
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.message.contains("uso do lavatório")),
            "{:#?}",
            report.findings
        );
    }

    fn square(home: &mut Home, name: &str, w: f64, d: f64) {
        let corners = [(0.0, 0.0), (w, 0.0), (w, d), (0.0, d)];
        for k in 0..4 {
            let (a, b) = (corners[k], corners[(k + 1) % 4]);
            let mut wall = Wall::new(
                WallId(k as u64 + 1),
                Point2::new(a.0, a.1),
                Point2::new(b.0, b.1),
            );
            wall.thickness = 15.0;
            wall.height = 260.0;
            home.walls.push(wall);
        }
        let inner = [
            (7.5, 7.5),
            (w - 7.5, 7.5),
            (w - 7.5, d - 7.5),
            (7.5, d - 7.5),
        ];
        home.rooms.push(Room::new(
            RoomId(10),
            name,
            inner.iter().map(|p| Point2::new(p.0, p.1)).collect(),
        ));
    }

    fn piece(
        id: u64,
        catalog: &str,
        at: (f64, f64),
        size: (f64, f64, f64),
        angle: f64,
    ) -> Furniture {
        Furniture {
            id: FurnitureId(id),
            catalog: catalog.into(),
            name: catalog.into(),
            position: Point2::new(at.0, at.1),
            angle,
            width: size.0,
            depth: size.1,
            height: size.2,
            ..Furniture::default()
        }
    }

    fn says(report: &Report, severity: Severity, text: &str) -> bool {
        report
            .findings
            .iter()
            .any(|f| f.severity == severity && f.message.contains(text))
    }

    /// Is there a finding of this severity standing on this source?
    fn cites(report: &Report, severity: Severity, code: &str) -> bool {
        report
            .findings
            .iter()
            .any(|f| f.severity == severity && f.reference == Some(code))
    }

    #[test]
    fn identical_architecture_warnings_remain_independent_after_acceptance_and_reordering() {
        let mut home = Home::default();
        for (id, x) in [(1, 0.0), (2, 400.0)] {
            home.rooms.push(Room::new(
                RoomId(id),
                "Quarto",
                vec![
                    Point2::new(x, 0.0),
                    Point2::new(x + 300.0, 0.0),
                    Point2::new(x + 300.0, 300.0),
                    Point2::new(x, 300.0),
                ],
            ));
        }
        let warnings = |home: &Home| {
            review(home, &Profile::default())
                .findings
                .into_iter()
                .filter(|f| f.message.starts_with("Sem janela:"))
                .collect::<Vec<_>>()
        };
        let before = warnings(&home);
        assert_eq!(
            before.len(),
            2,
            "each room must retain its own warning: {before:#?}"
        );
        assert_ne!(before[0].key, before[1].key);
        let first = before
            .iter()
            .find(|f| f.place.ends_with("r1"))
            .unwrap()
            .key
            .clone();
        let second = before
            .iter()
            .find(|f| f.place.ends_with("r2"))
            .unwrap()
            .key
            .clone();
        home.accepted
            .insert(first.clone(), "Only the first room reviewed".into());
        home.rooms.reverse();
        home.rooms
            .iter_mut()
            .find(|r| r.id == RoomId(1))
            .unwrap()
            .name = "Quarto renomeado".into();
        let after = warnings(&home);
        assert_eq!(after.len(), 2);
        assert!(
            after
                .iter()
                .find(|f| f.key == first)
                .unwrap()
                .accepted
                .is_some()
        );
        assert!(
            after
                .iter()
                .find(|f| f.key == second)
                .unwrap()
                .accepted
                .is_none()
        );
        assert!(
            !orphaned(&home, &Profile::default())
                .iter()
                .any(|(key, _)| key == &first)
        );
        // Removing the accepted room must not transfer its reason to the other room.
        home.rooms.retain(|r| r.id != RoomId(1));
        let remaining = warnings(&home);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].key, second);
        assert!(remaining[0].accepted.is_none());
        assert!(
            orphaned(&home, &Profile::default())
                .iter()
                .any(|(key, _)| key == &first)
        );
    }

    #[test]
    fn identical_piece_warnings_keep_individual_acceptances_when_dimensions_change() {
        let mut home = Home::default();
        square(&mut home, "Quarto", 600.0, 500.0);
        for (id, x) in [(20, 150.0), (21, 450.0)] {
            home.furniture.push(piece(
                id,
                "bed-single",
                (x, 200.0),
                (90.0, 200.0, 60.0),
                0.0,
            ));
        }
        let profile = Profile {
            wheelchair: true,
            ..Profile::default()
        };
        let warnings = |home: &Home| {
            review(home, &profile)
                .findings
                .into_iter()
                .filter(|f| f.message.starts_with("Cama a "))
                .collect::<Vec<_>>()
        };
        let before = warnings(&home);
        assert_eq!(before.len(), 2, "{before:#?}");
        let first = before
            .iter()
            .find(|f| f.place.ends_with("f20"))
            .unwrap()
            .key
            .clone();
        let second = before
            .iter()
            .find(|f| f.place.ends_with("f21"))
            .unwrap()
            .key
            .clone();
        assert_ne!(first, second);
        home.accepted
            .insert(first.clone(), "Reviewed bed 20 only".into());
        home.furniture.reverse();
        let bed = home
            .furniture
            .iter_mut()
            .find(|f| f.id == FurnitureId(20))
            .unwrap();
        bed.name = "Cama renomeada".into();
        bed.height = 65.0;
        let after = warnings(&home);
        assert_eq!(after.len(), 2);
        assert!(
            after
                .iter()
                .find(|f| f.key == first)
                .unwrap()
                .accepted
                .is_some()
        );
        assert!(
            after
                .iter()
                .find(|f| f.key == second)
                .unwrap()
                .accepted
                .is_none()
        );
    }

    #[test]
    fn identical_discipline_messages_keep_each_rooms_acceptance_key() {
        let mut home = Home::default();
        for (id, x) in [(1, 0.0), (2, 400.0)] {
            home.rooms.push(Room::new(
                RoomId(id),
                format!("Banho {id}"),
                vec![
                    Point2::new(x, 0.0),
                    Point2::new(x + 300.0, 0.0),
                    Point2::new(x + 300.0, 300.0),
                    Point2::new(x, 300.0),
                ],
            ));
        }
        home.furniture
            .push(piece(3, "toilet", (100.0, 100.0), (40.0, 65.0, 80.0), 0.0));
        let before = review(&home, &Profile::default());
        let keys = ["elec:outlets:r1", "elec:outlets:r2"];
        for key in keys {
            assert!(
                before.findings.iter().any(|f| f.key == key),
                "{key} must survive grouping"
            );
        }
        home.accepted
            .insert(keys[0].into(), "First bathroom reviewed".into());
        let after = review(&home, &Profile::default());
        assert!(
            after
                .findings
                .iter()
                .find(|f| f.key == keys[0])
                .unwrap()
                .accepted
                .is_some()
        );
        assert!(
            after
                .findings
                .iter()
                .find(|f| f.key == keys[1])
                .unwrap()
                .accepted
                .is_none()
        );
    }

    #[test]
    fn a_connected_closet_serves_its_bedroom_without_counting_laundry_storage() {
        let mut home = Home::default();
        for (id, name, x0, x1) in [
            (1, "Suíte", 0.0, 300.0),
            (2, "Closet", 312.0, 512.0),
            (3, "Lavanderia", 524.0, 724.0),
        ] {
            home.rooms.push(Room::new(
                RoomId(id),
                name,
                vec![
                    Point2::new(x0, 0.0),
                    Point2::new(x1, 0.0),
                    Point2::new(x1, 300.0),
                    Point2::new(x0, 300.0),
                ],
            ));
        }
        let mut wall = Wall::new(
            WallId(10),
            Point2::new(306.0, 0.0),
            Point2::new(306.0, 300.0),
        );
        wall.thickness = 12.0;
        home.walls.push(wall);
        home.furniture = vec![
            piece(
                20,
                "bed-double",
                (150.0, 180.0),
                (158.0, 208.0, 55.0),
                180.0,
            ),
            piece(21, "wardrobe", (412.0, 270.0), (200.0, 50.0, 220.0), 180.0),
            piece(22, "wardrobe", (412.0, 30.0), (140.0, 50.0, 220.0), 0.0),
            piece(23, "wardrobe", (624.0, 270.0), (200.0, 50.0, 220.0), 180.0),
        ];
        let missing = |h: &Home| {
            review(h, &Profile::default())
                .findings
                .iter()
                .any(|f| f.message.contains("Falta guarda-roupa"))
        };
        let report = review(&home, &Profile::default());
        assert!((report.capacity.wardrobe_cm - 340.0).abs() < 0.01);
        assert!(
            missing(&home),
            "an adjacent but sealed closet is not access"
        );
        let mut door = piece(24, "door", (306.0, 150.0), (80.0, 12.0, 210.0), 90.0);
        door.opening = Some(newera_core::Opening::default());
        home.furniture.push(door);
        assert!(!missing(&home));
        home.rooms[1].name = "Closet da suíte".into();
        assert_eq!(review(&home, &Profile::default()).capacity.bedrooms, 1);
        assert!(!missing(&home));
        home.furniture[4].opening.as_mut().unwrap().kind = OpeningKind::Window;
        assert!(missing(&home), "a window does not connect clothing storage");
        home.furniture[4].opening.as_mut().unwrap().kind = OpeningKind::Passage;
        assert!(!missing(&home));
        home.furniture[4].visible = false;
        assert!(missing(&home));
        home.furniture[4].visible = true;
        home.walls.clear();
        assert!(missing(&home), "a loose opening must not invent an access");
    }

    #[test]
    fn unused_kitchen_corner_is_feedback_without_fix_or_score_penalty() {
        let mut home = Home::default();
        square(&mut home, "Cozinha", 500.0, 400.0);
        home.furniture = vec![
            piece(20, "base-cabinet", (263.5, 37.5), (300.0, 60.0, 90.0), 0.0),
            piece(21, "fridge", (456.5, 120.0), (70.0, 72.0, 180.0), 90.0),
        ];
        let report = review(&home, &Profile::default());
        let finding = report
            .findings
            .iter()
            .find(|f| f.key.starts_with("unused_corner:"))
            .unwrap();
        assert!(finding.message.contains("0.61 m²"), "{finding:?}");
        assert!(finding.message.contains("f20") && finding.message.contains("f21"));
        assert_eq!(finding.severity, Severity::Dica);
        assert!(finding.fix.is_none() && finding.probe.is_none());
        assert_eq!(finding.weight, 0);
        let key = finding.key.clone();
        home.accepted
            .insert(key.clone(), "Reserva de manutenção confirmada".into());
        let accepted = review(&home, &Profile::default());
        assert!(
            accepted
                .findings
                .iter()
                .any(|f| f.key == key && f.accepted.is_some())
        );
        let mut open = home.clone();
        open.walls.clear();
        assert_eq!(corners::review(&Scene::new(&open)), [] as [Finding; 0]);
        let mut aisle = home.clone();
        aisle.furniture[1].position.y = 200.0;
        assert_eq!(corners::review(&Scene::new(&aisle)), [] as [Finding; 0]);
        let mut filled = home.clone();
        filled
            .furniture
            .push(piece(22, "box", (453.0, 46.25), (79.0, 77.5, 80.0), 0.0));
        assert_eq!(corners::review(&Scene::new(&filled)), [] as [Finding; 0]);
        let mut raised = home.clone();
        raised.furniture[0].elevation = 150.0;
        assert_eq!(corners::review(&Scene::new(&raised)), [] as [Finding; 0]);
        let mut access = home.clone();
        let mut door = piece(22, "door", (455.0, 0.0), (70.0, 15.0, 210.0), 0.0);
        door.opening = Some(newera_core::Opening::default());
        access.furniture.push(door);
        assert_eq!(corners::review(&Scene::new(&access)), [] as [Finding; 0]);
        for angle in [90.0_f64, 33.0] {
            let (sin, cos) = angle.to_radians().sin_cos();
            let rotate = |p: Point2| Point2::new(p.x * cos - p.y * sin, p.x * sin + p.y * cos);
            let mut rotated = home.clone();
            for wall in &mut rotated.walls {
                wall.start = rotate(wall.start);
                wall.end = rotate(wall.end);
            }
            for room in &mut rotated.rooms {
                for point in &mut room.points {
                    *point = rotate(*point);
                }
            }
            for item in &mut rotated.furniture {
                item.position = rotate(item.position);
                item.angle += angle;
            }
            let findings = corners::review(&Scene::new(&rotated));
            assert_eq!(findings.len(), 1, "{findings:?}");
            assert_eq!(findings[0].key, key);
            assert!(findings[0].message.contains("0.61 m²"));
        }
    }

    #[test]
    fn a_bedroom_is_checked_for_its_people_and_the_walk_around_the_bed() {
        let mut home = Home::default();
        square(&mut home, "Quarto", 330.0, 330.0);
        // Double bed, head on the top wall (front toward +y), centered.
        home.furniture.push(piece(
            20,
            "bed-double",
            (165.0, 7.5 + 104.0),
            (158.0, 208.0, 55.0),
            0.0,
        ));
        home.furniture.push(piece(
            21,
            "wardrobe",
            (165.0, 322.5 - 30.0),
            (180.0, 60.0, 220.0),
            180.0,
        ));
        let report = review(&home, &Profile::default());
        // 315 − 158 = 157 cm → 78,5 cm each side; foot: 322,5 − 60 − 215,5 = 47 cm.
        assert!(
            !says(&report, Severity::Alerta, "à esquerda"),
            "{report:#?}"
        );
        assert!(
            says(&report, Severity::Dica, "passagem aos pés"),
            "{report:#?}"
        );
        assert_eq!(report.capacity.beds, 2);
        // Four people, one double bed and no bathroom.
        let crowded = review(
            &home,
            &Profile {
                occupants: 4,
                ..Profile::default()
            },
        );
        assert!(says(&crowded, Severity::Erro, "faltam 2"), "{crowded:#?}");
        assert!(says(&crowded, Severity::Erro, "Nenhum banheiro"));
        assert!(crowded.score < report.score);
        // Pushed against the left wall, the couple loses a side.
        home.furniture[0].position.x = 7.5 + 79.0 + 20.0;
        let pushed = review(&home, &Profile::default());
        assert!(
            says(&pushed, Severity::Alerta, "20 cm livres à"),
            "{pushed:#?}"
        );
        assert!(says(&pushed, Severity::Alerta, "afaste 30 cm"));
        let fix = pushed
            .findings
            .iter()
            .find_map(|f| f.fix.clone())
            .expect("a move that frees the side");
        assert_eq!(fix["tool"], "move");
        assert_eq!(fix["dx"], 30.0, "{fix}");
    }

    #[test]
    fn the_house_holds_what_every_storey_holds() {
        use newera_core::{Level, LevelId};
        let mut home = Home::default();
        home.levels = vec![
            Level {
                id: LevelId(1),
                name: "Térreo".into(),
                elevation: 0.0,
                height: 280.0,
                ..Level::default()
            },
            Level {
                id: LevelId(2),
                name: "Superior".into(),
                elevation: 292.0,
                height: 280.0,
                elevation_index: 1,
                ..Level::default()
            },
        ];
        let room = |id: u64, name: &str, x: f64, level: u64| {
            let mut room = Room::new(
                RoomId(id),
                name,
                [(x, 0.0), (x + 400.0, 0.0), (x + 400.0, 400.0), (x, 400.0)]
                    .iter()
                    .map(|p| Point2::new(p.0, p.1))
                    .collect(),
            );
            room.level = Some(LevelId(level));
            room
        };
        home.rooms = vec![
            room(10, "Sala", 0.0, 1),
            room(11, "Banheiro", 400.0, 1),
            room(12, "Quarto", 0.0, 2),
            room(13, "Quarto 2", 400.0, 2),
        ];
        let on = |mut f: Furniture, level: u64| {
            f.level = Some(LevelId(level));
            f
        };
        home.furniture = vec![
            on(
                piece(20, "sofa-3", (200.0, 100.0), (210.0, 90.0, 85.0), 0.0),
                1,
            ),
            on(
                piece(
                    21,
                    "dining-table-6",
                    (200.0, 300.0),
                    (180.0, 90.0, 75.0),
                    0.0,
                ),
                1,
            ),
            on(
                piece(22, "toilet", (600.0, 100.0), (40.0, 68.0, 40.0), 0.0),
                1,
            ),
            on(
                piece(23, "bed-double", (200.0, 200.0), (160.0, 200.0, 50.0), 0.0),
                2,
            ),
            on(
                piece(24, "bed-double", (600.0, 200.0), (160.0, 200.0, 50.0), 0.0),
                2,
            ),
        ];
        let profile: Profile = serde_json::from_str(r#"{"occupants":4}"#).unwrap();
        for shown in [1, 2] {
            home.selected_level = Some(LevelId(shown));
            let report = review(&home, &profile);
            assert_eq!(report.capacity.beds, 4, "shown lv{shown}");
            assert_eq!(report.capacity.bedrooms, 2, "shown lv{shown}");
            assert_eq!(report.capacity.bathrooms, 1, "shown lv{shown}");
            assert_eq!(report.capacity.dining_seats, 6, "shown lv{shown}");
            assert_eq!(report.capacity.living_seats, 3, "shown lv{shown}");
            let said = |text: &str| report.findings.iter().any(|f| f.message.contains(text));
            assert!(!said("Nenhum banheiro"), "shown lv{shown}");
            assert!(!said("Nenhum dormitório"), "shown lv{shown}");
            assert!(!said("lugares à mesa"), "shown lv{shown}");
        }
        // One storey designed, and a traced plan shown: the house is the design.
        home.levels[1].set_reference(true);
        for f in &mut home.furniture {
            if f.level == Some(LevelId(2)) {
                f.position.y += 400.0;
            }
            f.level = Some(LevelId(1));
        }
        for r in &mut home.rooms {
            if r.level == Some(LevelId(2)) {
                for p in &mut r.points {
                    p.y += 400.0;
                }
            }
            r.level = Some(LevelId(1));
        }
        home.selected_level = Some(LevelId(2));
        let report = review(&home, &profile);
        assert_eq!(report.capacity.beds, 4, "the design, not the tracing");
        assert_eq!(report.capacity.bathrooms, 1);
    }

    fn named_again(home: &mut Home) {
        let mut shower = piece(30, "shower", (40.0, 200.0), (90.0, 90.0, 200.0), 0.0);
        shower.name = "Box".into();
        home.furniture.push(shower);
    }

    #[test]
    fn imported_pieces_are_read_by_name_and_built_in_ones_do_not_collide() {
        let mut home = Home::default();
        square(&mut home, "Banho suíte", 300.0, 240.0);
        let mut named = |id: u64, name: &str, at: (f64, f64), size: (f64, f64, f64), elev: f64| {
            let mut f = piece(id, "imported", at, size, 0.0);
            f.name = name.into();
            f.elevation = elev;
            home.furniture.push(f);
        };
        named(
            20,
            "Bancada contínua junto à geladeira",
            (150.0, 40.0),
            (212.0, 64.0, 4.0),
            87.0,
        );
        named(
            21,
            "Cooktop Brastemp BDS62AE — 4 bocas",
            (150.0, 40.0),
            (59.0, 48.5, 8.6),
            87.0,
        );
        named(
            22,
            "10 — Gavetões sob cooktop — 65 cm",
            (150.0, 40.0),
            (65.0, 60.0, 87.0),
            0.0,
        );
        named(
            23,
            "Mesa Dover — eucalipto",
            (150.0, 170.0),
            (120.0, 80.0, 77.0),
            0.0,
        );
        named(24, "Cadeira Dover", (150.0, 140.0), (51.0, 59.0, 85.0), 0.0);
        named(25, "Vaso suíte", (40.0, 200.0), (40.0, 63.0, 62.0), 0.0);
        named(
            26,
            "Cama suíte",
            (1000.0, 1000.0),
            (140.0, 200.0, 70.0),
            0.0,
        );
        let scene = Scene::new(&home);
        let what = |id: u64| {
            scene
                .units
                .iter()
                .find(|u| u.piece.id == FurnitureId(id))
                .unwrap()
                .what
        };
        assert_eq!(what(21), Use::Stove);
        assert_eq!(what(22), Use::Counter);
        assert_eq!(what(23), Use::DiningTable(4));
        assert_eq!(what(24), Use::Chair);
        assert_eq!(what(25), Use::Toilet);
        assert_eq!(what(26), Use::Bed(2));
        assert_eq!(scene.spaces[0].what, RoomUse::Bathroom);
        // The cooktop sits in its countertop and the chair under the table: no overlap.
        assert!(scene.overlaps().is_empty(), "{:?}", scene.overlaps());
        // A toilet standing inside the shower is not built in.
        named_again(&mut home);
        let scene = Scene::new(&home);
        assert_eq!(scene.overlaps().len(), 1);
    }

    #[test]
    fn a_fix_that_opens_a_finding_as_heavy_says_the_two_do_not_fit() {
        let mut home = Home::default();
        square(&mut home, "Sala e cozinha", 600.0, 326.0);
        // A counter facing the room, a sofa with its back to it and a rack in
        // front of the sofa: 71 cm behind the sofa, 50 in front.
        home.furniture.push(piece(
            20,
            "base-cabinet",
            (300.0, 7.5 + 30.0),
            (240.0, 60.0, 90.0),
            0.0,
        ));
        home.furniture.push(piece(
            21,
            "sofa-3",
            (300.0, 67.5 + 71.0 + 45.0),
            (210.0, 90.0, 85.0),
            0.0,
        ));
        home.furniture.push(piece(
            22,
            "tv-stand",
            (300.0, 228.5 + 50.0 + 20.0),
            (180.0, 40.0, 50.0),
            180.0,
        ));
        let report = review(&home, &Profile::default());
        // The counter's fix pushes the sofa into the rack's clearance: it
        // says what it leaves behind, instead of the agent finding out.
        let counter = report
            .findings
            .iter()
            .find(|f| f.place.contains("f20") && f.message.contains("livres à frente"))
            .unwrap_or_else(|| panic!("{report:#?}"));
        assert!(counter.message.contains("f21"), "{counter:#?}");
        assert!(counter.message.contains("Isso deixa"), "{counter:#?}");
    }

    /// A project saved before rules were named carries keys built from the
    /// standard, the place and the first words of the sentence. They still
    /// answer for the finding they were written about.
    #[test]
    fn an_acceptance_written_under_the_old_prose_key_still_follows_its_finding() {
        let mut home = Home::default();
        square(&mut home, "Cozinha", 300.0, 300.0);
        for w in &mut home.walls {
            w.height = 240.0;
        }
        let report = review(&home, &Profile::default());
        let finding = report
            .findings
            .iter()
            .find(|f| f.key.starts_with("ceiling_height:"))
            .unwrap_or_else(|| panic!("{report:#?}"))
            .clone();
        let old = legacy_key_of(&finding);
        assert!(
            old.starts_with("nbr15575:") && old.split(':').count() == 3,
            "the shape projects were saved with: {old}"
        );
        home.accepted
            .insert(old.clone(), "laje existente, não há como subir".into());
        let again = review(&home, &Profile::default());
        let same = again
            .findings
            .iter()
            .find(|f| f.key == finding.key)
            .unwrap();
        assert_eq!(
            same.accepted.as_deref(),
            Some("laje existente, não há como subir")
        );
        assert_eq!(same.accepted_as.as_deref(), Some(old.as_str()));
        assert!(again.score > report.score);
        assert!(
            orphaned(&home, &Profile::default()).is_empty(),
            "not an orphan: it is this finding's own old name"
        );
    }

    /// The old shape was matched by its tail, ignoring which rule wrote it,
    /// so an acceptance could land on a rule it was never given to. A key
    /// names one rule now, and nothing else answers to it.
    #[test]
    fn an_acceptance_never_crosses_to_another_rule_about_the_same_place() {
        let mut home = Home::default();
        square(&mut home, "Cozinha", 300.0, 300.0);
        for w in &mut home.walls {
            w.height = 240.0;
        }
        let report = review(&home, &Profile::default());
        let low = report
            .findings
            .iter()
            .find(|f| f.key.starts_with("ceiling_height:"))
            .unwrap_or_else(|| panic!("{report:#?}"))
            .clone();
        let others: Vec<String> = report
            .findings
            .iter()
            .filter(|f| f.place == low.place && f.key != low.key)
            .map(|f| f.key.clone())
            .collect();
        assert!(!others.is_empty(), "{report:#?}");
        home.accepted
            .insert(low.key.clone(), "laje existente".into());
        let again = review(&home, &Profile::default());
        for key in others {
            assert!(
                again
                    .findings
                    .iter()
                    .find(|f| f.key == key)
                    .is_some_and(|f| f.accepted.is_none()),
                "{key} was not the one accepted"
            );
        }
    }

    /// Wording is not identity: the same rule about the same place keeps its
    /// acceptance when the sentence is rewritten — or translated.
    #[test]
    fn rewording_a_finding_does_not_drop_the_acceptance_written_for_it() {
        let mut home = Home::default();
        square(&mut home, "Quarto", 300.0, 300.0);
        let reworded = |f: &Finding| {
            let mut f = f.clone();
            f.message = "Ceiling at 240 cm, below the 250 cm this room needs.".into();
            f
        };
        let report = review(&home, &Profile::default());
        let finding = report
            .findings
            .iter()
            .find(|f| f.key.starts_with("room_without_window:"))
            .unwrap_or_else(|| panic!("{report:#?}"));
        assert_eq!(
            rule::key(Rule::RoomWithoutWindow, &finding.place),
            finding.key,
            "the key is the rule and the place, nothing of the sentence"
        );
        assert_eq!(
            reworded(finding).key,
            finding.key,
            "a rewritten sentence answers to the same key"
        );
        assert_ne!(
            legacy_key_of(&reworded(finding)),
            legacy_key_of(finding),
            "which is exactly what the old prose key could not do"
        );
    }

    #[test]
    fn beds_side_by_side_accessible_tops_and_gas_where_people_wash_or_sleep() {
        let mut bedroom = Home::default();
        square(&mut bedroom, "Quarto", 400.0, 400.0);
        bedroom.furniture.push(piece(
            20,
            "bed-single",
            (100.0, 107.5),
            (90.0, 200.0, 50.0),
            0.0,
        ));
        bedroom.furniture.push(piece(
            21,
            "bed-single",
            (230.0, 107.5),
            (90.0, 200.0, 50.0),
            0.0,
        ));
        let r = review(&bedroom, &Profile::default());
        assert!(says(&r, Severity::Alerta, "40 cm entre"), "{r:#?}");

        let mut bath = Home::default();
        square(&mut bath, "Banheiro", 200.0, 250.0);
        let mut basin = piece(30, "basin-cabinet", (100.0, 30.0), (60.0, 45.0, 90.0), 0.0);
        basin.name = "Lavatório".into();
        bath.furniture.push(basin);
        let mut heater = piece(31, "imported", (30.0, 200.0), (40.0, 20.0, 60.0), 0.0);
        heater.name = "Aquecedor a gás".into();
        heater.elevation = 150.0;
        bath.furniture.push(heater);
        let wheel = Profile {
            wheelchair: true,
            ..Profile::default()
        };
        let r = review(&bath, &wheel);
        assert!(says(&r, Severity::Alerta, "Tampo a 90 cm"), "{r:#?}");
        assert!(
            r.findings.iter().any(|f| f.message.contains("tipo C")),
            "{r:#?}"
        );

        let mut studio = Home::default();
        square(&mut studio, "Studio", 500.0, 400.0);
        studio.furniture.push(piece(
            40,
            "bed-double",
            (100.0, 107.5),
            (140.0, 200.0, 50.0),
            0.0,
        ));
        studio
            .furniture
            .push(piece(41, "stove", (400.0, 37.5), (60.0, 60.0, 90.0), 0.0));
        let r = review(&studio, &Profile::default());
        assert!(
            r.findings.iter().any(|f| f.message.contains("8,14 kW")),
            "{r:#?}"
        );
    }

    #[test]
    fn heights_and_clearances_follow_the_norm_texts() {
        // NBR 15575-1 16.1.1: a kitchen takes 2,50 m, a bathroom 2,30 m.
        let mut kitchen = Home::default();
        square(&mut kitchen, "Cozinha", 300.0, 300.0);
        for w in &mut kitchen.walls {
            w.height = 240.0;
        }
        let r = review(&kitchen, &Profile::default());
        assert!(says(&r, Severity::Alerta, "o mínimo é 250 cm"), "{r:#?}");
        let mut bath = Home::default();
        square(&mut bath, "Banheiro", 200.0, 250.0);
        for w in &mut bath.walls {
            w.height = 240.0;
        }
        let r = review(&bath, &Profile::default());
        assert!(!says(&r, Severity::Alerta, "Pé-direito"), "{r:#?}");

        // NBR 9050 fig. 26: a switch at 110 cm is out of reach, an outlet at 90 is not.
        let mut room = Home::default();
        square(&mut room, "Sala", 400.0, 400.0);
        let mut switch = piece(30, "switch", (10.0, 100.0), (8.0, 4.0, 12.0), 0.0);
        switch.elevation = 104.0;
        let mut outlet = piece(31, "outlet-mid", (10.0, 200.0), (8.0, 4.0, 12.0), 0.0);
        outlet.elevation = 84.0;
        room.furniture = vec![switch, outlet];
        let wheel = Profile {
            wheelchair: true,
            ..Profile::default()
        };
        let r = review(&room, &wheel);
        assert!(
            says(&r, Severity::Alerta, "1 interruptor(es)/tomada(s)"),
            "{r:#?}"
        );

        // NBR 15575-1 annex F: 40 cm in front of a toilet is enough.
        let mut wc = Home::default();
        square(&mut wc, "Banheiro", 200.0, 250.0);
        wc.furniture.push(piece(
            40,
            "toilet",
            (100.0, 7.5 + 31.5),
            (40.0, 63.0, 40.0),
            0.0,
        ));
        wc.furniture.push(piece(
            41,
            "base-cabinet",
            (100.0, 7.5 + 63.0 + 45.0 + 30.0),
            (60.0, 60.0, 90.0),
            180.0,
        ));
        let r = review(&wc, &Profile::default());
        assert!(
            !r.findings.iter().any(|f| f.message.contains("uso do vaso")),
            "45 cm free: {r:#?}"
        );
    }

    #[test]
    fn a_project_point_named_after_the_fixture_it_serves_is_not_that_fixture() {
        let mut home = Home::default();
        square(&mut home, "Banho social", 200.0, 250.0);
        let mut toilet = piece(20, "toilet", (60.0, 40.0), (40.0, 63.0, 62.0), 0.0);
        toilet.name = "Vaso".into();
        home.furniture.push(toilet);
        let before = review(&home, &Profile::default());
        for (id, catalog, name, at) in [
            (21, "sewer", "Esgoto — vaso banho social", (60.0, 12.0)),
            (
                22,
                "cold-water",
                "Água fria — vaso banho social",
                (90.0, 10.0),
            ),
            (23, "cold-water", "Água fria — chuveiro", (180.0, 200.0)),
        ] {
            let mut point = piece(id, catalog, at, (10.0, 10.0, 5.0), 0.0);
            point.name = name.into();
            point.discipline = Some(newera_core::Discipline::Plumbing);
            home.furniture.push(point);
        }
        let scene = Scene::new(&home);
        for id in 21..=23 {
            let unit = scene.units.iter().find(|u| u.piece.id == FurnitureId(id));
            assert!(
                unit.is_none_or(|u| u.what == Use::Other),
                "f{id}: {:?}",
                unit.map(|u| u.what)
            );
        }
        let after = review(&home, &Profile::default());
        assert!(
            !after.findings.iter().any(|f| !f.key.starts_with("plumb:")
                && (f.place.contains("Esgoto") || f.place.contains("Água fria"))),
            "{:#?}",
            after.findings
        );
        // Its points only ever take plumbing findings away.
        assert!(after.score >= before.score, "{:#?}", after.findings);
        assert!(
            before.findings.iter().any(|f| f.key == "plumb:sewer:f20")
                && !after.findings.iter().any(|f| f.key == "plumb:sewer:f20"),
            "the toilet had no sewer and now has: {:#?}",
            after.findings
        );
    }

    #[test]
    fn the_sofa_sits_at_a_distance_that_suits_the_screen() {
        let mut home = Home::default();
        square(&mut home, "Sala", 400.0, 400.0);
        let mut tv = piece(20, "tv", (200.0, 12.0), (124.0, 8.0, 72.0), 0.0);
        tv.elevation = 70.0;
        home.furniture.push(tv);
        home.furniture.push(piece(
            21,
            "sofa-3",
            (200.0, 160.0),
            (210.0, 90.0, 85.0),
            180.0,
        ));
        let report = review(&home, &Profile::default());
        // 148 cm from a 56" screen (143 cm diagonal): closer than 172 cm.
        assert!(
            says(&report, Severity::Dica, "entre 172 e 358 cm"),
            "{report:#?}"
        );
        home.furniture[1].position.y = 230.0;
        let report = review(&home, &Profile::default());
        assert!(!says(&report, Severity::Dica, "da TV"), "{report:#?}");
    }

    #[test]
    fn the_tv_is_judged_from_the_seats_that_face_it() {
        let mut home = Home::default();
        square(&mut home, "Estar", 500.0, 500.0);
        let mut tv = piece(20, "tv", (250.0, 450.0), (124.0, 8.0, 72.0), 180.0);
        tv.elevation = 100.0;
        home.furniture = vec![
            tv,
            // The sofa watches it from 3 m.
            piece(21, "sofa-3", (250.0, 150.0), (210.0, 90.0, 85.0), 0.0),
            // A reading chair beside it, turned away from the screen.
            piece(22, "armchair", (130.0, 400.0), (65.0, 65.0, 85.0), 180.0),
        ];
        let tip = |home: &Home| {
            review(home, &Profile::default())
                .findings
                .iter()
                .find(|f| f.key.starts_with("tv_distance"))
                .map(|f| f.message.to_string())
        };
        assert_eq!(tip(&home), None, "the sofa is where it is watched from");
        // Turned to the screen, the chair is the nearest seat watching it.
        home.furniture[2].angle = 270.0;
        assert!(
            tip(&home).is_some_and(|t| t.contains("armchair")),
            "{:?}",
            tip(&home)
        );
    }

    /// The false positive with a standard at its root: a dishwasher is an
    /// appliance in a niche (EN 1116), so its lid is never read as the height
    /// of the worktop it hides under.
    #[test]
    fn a_dishwasher_is_an_appliance_not_a_worktop() {
        let mut home = Home::default();
        square(&mut home, "Cozinha", 400.0, 300.0);
        home.furniture.push(piece(
            20,
            "sink-counter",
            (100.0, 7.5 + 30.0),
            (120.0, 60.0, 92.0),
            0.0,
        ));
        // A 84,5 cm dishwasher beside a 92 cm counter, for a 146 cm cook whose
        // comfortable height is exactly 84 cm: if the dishwasher counted as
        // worktop, the counter would be the one reported as wrong.
        home.furniture.push(piece(
            21,
            "dishwasher",
            (220.0, 7.5 + 30.0),
            (60.0, 60.0, 84.5),
            0.0,
        ));
        home.furniture.push(piece(
            22,
            "stove",
            (300.0, 7.5 + 31.0),
            (60.0, 62.0, 90.0),
            0.0,
        ));
        let tall = Profile {
            stature: 165.0.into(),
            ..Profile::default()
        };
        let report = review(&home, &tall);
        assert!(
            !says(&report, Severity::Dica, "84,5 cm"),
            "the dishwasher's lid is not a countertop: {report:#?}"
        );

        // The reported false positive came the other way: a designer's
        // module, numbered and named, with no catalog id to go by.
        let mut named = home.clone();
        named.furniture.retain(|f| f.id != FurnitureId(21));
        let mut module = piece(24, "", (220.0, 7.5 + 30.0), (60.0, 60.0, 84.5), 0.0);
        module.name = "10 — Lava-louças de embutir".to_owned();
        named.furniture.push(module);
        let report = review(&named, &tall);
        assert!(
            !says(&report, Severity::Dica, "84,5 cm"),
            "read by name, it is still an appliance: {report:#?}"
        );
    }

    #[test]
    fn a_washer_stands_under_a_countertop_with_a_sink() {
        let mut home = Home::default();
        square(&mut home, "Lavanderia", 300.0, 200.0);
        let mut top = piece(20, "group", (150.0, 41.0), (264.0, 67.0, 90.0), 0.0);
        top.name = "Bancada 264 × 67 cm".into();
        top.properties.insert(
            "joinery:params".into(),
            r#"{"kind":"countertop","length":264,"depth":67,"height":90,"thickness":3,"cutouts":[{"kind":"sink","x":200}]}"#.into(),
        );
        home.furniture = vec![
            top,
            piece(21, "washer", (60.0, 39.0), (60.0, 60.0, 85.0), 0.0),
        ];
        let clash = |home: &Home| {
            review(home, &Profile::default())
                .findings
                .iter()
                .any(|f| f.message.contains("Ocupa o mesmo lugar"))
        };
        assert!(!clash(&home), "under the slab");
        // Taller than the space under the slab, it is in the countertop.
        home.furniture[1].height = 95.0;
        assert!(clash(&home), "through the slab");
    }

    #[test]
    fn the_kitchen_is_read_against_alexander_the_gas_standard_and_the_city() {
        let mut home = Home::default();
        // 170 cm wide: narrower than the most modest financed unit.
        square(&mut home, "Cozinha", 380.0, 185.0);
        home.furniture.push(piece(
            20,
            "sink-counter",
            (80.0, 7.5 + 30.0),
            (120.0, 60.0, 90.0),
            0.0,
        ));
        home.furniture.push(piece(
            21,
            "stove",
            (200.0, 7.5 + 31.0),
            (60.0, 62.0, 90.0),
            0.0,
        ));
        home.furniture.push(piece(
            22,
            "fridge",
            (330.0, 7.5 + 35.0),
            (70.0, 70.0, 180.0),
            0.0,
        ));
        // One short stretch of worktop, and nothing else.
        home.furniture.push(piece(
            23,
            "base-cabinet",
            (280.0, 7.5 + 30.0),
            (60.0, 60.0, 90.0),
            0.0,
        ));
        let report = review(&home, &Profile::default());
        // Alexander 184: 60 cm of free counter is not 366.
        assert!(
            cites(&report, Severity::Dica, "alexander184"),
            "{report:#?}"
        );
        // A gas appliance with no window at all is a safety problem.
        assert!(
            cites(&report, Severity::Erro, "coe-municipal"),
            "{report:#?}"
        );
        // Cooking with nothing to capture what it releases.
        assert!(
            cites(&report, Severity::Alerta, "lbnl-coifa"),
            "{report:#?}"
        );
        // The narrow kitchen: MCMV binds only its own units, so it compares.
        assert!(cites(&report, Severity::Dica, "caixa-mcmv"), "{report:#?}");
        // No city: the municipal circle is advice, not a verdict.
        assert!(
            cites(&report, Severity::Dica, "coe-municipal"),
            "{report:#?}"
        );

        let paulista = Profile {
            city: Some("sao-paulo".to_owned()),
            ..Profile::default()
        };
        let judged = review(&home, &paulista);
        assert!(
            judged.findings.iter().any(|f| f.severity == Severity::Erro
                && f.reference == Some("coe-municipal")
                && f.message.contains("pede 150 cm")),
            "a kitchen 170 cm wide holds no 150 cm circle: {judged:#?}"
        );
        assert!(
            judged.refs.iter().any(|r| r.code == "coe-municipal"),
            "{:?}",
            judged.refs
        );

        // Induction burns nothing: the gas standard stops applying.
        let mut electric = home.clone();
        electric
            .furniture
            .iter_mut()
            .find(|f| f.id == FurnitureId(21))
            .unwrap()
            .name = "Cooktop de indução".to_owned();
        let clean = review(&electric, &Profile::default());
        assert!(
            !clean
                .findings
                .iter()
                .any(|f| f.reference == Some("nbr13103")),
            "{clean:#?}"
        );
    }

    /// Sockets are only judged where someone drew an electrical project: a
    /// plan without one is unfinished, not wrong.
    #[test]
    fn sockets_are_counted_only_against_an_electrical_project() {
        let mut home = Home::default();
        square(&mut home, "Cozinha", 400.0, 300.0);
        home.furniture.push(piece(
            20,
            "sink-counter",
            (100.0, 7.5 + 30.0),
            (120.0, 60.0, 90.0),
            0.0,
        ));
        home.furniture.push(piece(
            21,
            "stove",
            (250.0, 7.5 + 31.0),
            (60.0, 62.0, 90.0),
            0.0,
        ));
        // No outlet at all is the worst case, not a quiet one: the norm's
        // outlets per metre are asked of a furnished kitchen anyway.
        let quiet = review(&home, &Profile::default());
        let missing = quiet
            .findings
            .iter()
            .find(|f| f.key.starts_with("elec:outlets:"))
            .unwrap_or_else(|| panic!("{quiet:#?}"));
        assert!(
            missing.message.starts_with("0 de 4 tomadas"),
            "{missing:#?}"
        );
        // Accepted with its reason, it stops costing score.
        let mut accepted = home.clone();
        accepted.accepted.insert(
            missing.key.clone(),
            "projeto elétrico a cargo do condomínio".into(),
        );
        assert!(review(&accepted, &Profile::default()).score > quiet.score);

        let mut wired = home.clone();
        let mut socket = piece(22, "outlet-low", (40.0, 7.5), (10.0, 5.0, 10.0), 0.0);
        socket.elevation = 30.0;
        wired.furniture.push(socket);
        let report = review(&wired, &Profile::default());
        // Perimeter of 13,7 m needs four points; one is drawn, none of them
        // above the counter.
        assert!(cites(&report, Severity::Erro, "nbr5410"), "{report:#?}");
        assert!(
            says(&report, Severity::Erro, "0 tomada(s) acima da bancada"),
            "{report:#?}"
        );
    }

    /// Every argument of a call, split at the top-level commas, from just
    /// after its opening parenthesis.
    fn arguments(text: &str) -> Vec<&str> {
        let (mut depth, mut start, mut out) = (0i32, 0, Vec::new());
        let mut quoted = false;
        for (i, c) in text.char_indices() {
            match c {
                '"' => quoted = !quoted,
                _ if quoted => {}
                '(' | '{' | '[' => depth += 1,
                ')' | '}' | ']' if depth == 0 => {
                    out.push(&text[start..i]);
                    break;
                }
                ')' | '}' | ']' => depth -= 1,
                ',' if depth == 0 => {
                    out.push(&text[start..i]);
                    start = i + 1;
                }
                _ => {}
            }
        }
        out.retain(|a| !a.trim().is_empty());
        out
    }

    fn literals(text: &str) -> Vec<&str> {
        text.split('"').skip(1).step_by(2).collect()
    }

    #[test]
    fn every_source_a_rule_cites_is_in_the_registry() {
        // `push_ref` only `debug_assert!`s an unknown code, and only when the
        // rule fires: a citation nobody's plan trips is never looked up. Most
        // citations live on figures now, and newera-core checks those; this
        // reads the ones the rules still write themselves.
        let mut cited = Vec::new();
        for file in [include_str!("lib.rs"), include_str!("corners.rs")] {
            let body = file.split("\nmod tests {").next().unwrap_or(file);
            for (at, _) in body.match_indices("push_ref(") {
                if body[..at].ends_with("fn ") {
                    continue;
                }
                let args = arguments(&body[at + "push_ref(".len()..]);
                cited.extend(literals(args.last().copied().unwrap_or_default()));
            }
        }
        assert!(
            cited.len() > 5,
            "only {} citations found: {cited:?}",
            cited.len()
        );
        for code in cited {
            assert!(
                standards::standard(code).is_some(),
                "a rule cites `{code}`, which is not in the registry"
            );
        }
    }

    #[test]
    fn a_finding_is_said_in_english_and_accepted_in_portuguese() {
        use newera_core::vocabulary::Language::English;
        let mut home = Home::default();
        square(&mut home, "Bedroom", 400.0, 400.0);
        home.furniture.push(piece(
            20,
            "bed-double",
            (40.0 + 80.0, 107.5),
            (160.0, 200.0, 50.0),
            0.0,
        ));
        let report = review(&home, &Profile::default());
        let side = report
            .findings
            .iter()
            .find(|f| f.key == "clearance:f20:left")
            .unwrap_or_else(|| panic!("{report:#?}"));
        // The canonical sentence stays Portuguese: it is what acceptances
        // were written under before rules were named.
        assert!(
            side.message.contains("livres à esquerda"),
            "{}",
            side.message.as_str()
        );
        assert!(legacy_key_of(side).contains("livres-esquerda"));
        // Said in English, every word of it is — the side, the reason and
        // the advice passed into the sentence as well.
        let english = side.message.in_language(English);
        assert!(
            english.contains("cm free on the left (circulation beside the bed: at least 50 cm)"),
            "{english}"
        );
        assert!(
            !english.contains("livres") && !english.contains("afaste"),
            "{english}"
        );
        let toilet = report
            .findings
            .iter()
            .find(|f| f.key == "no_toilet:casa")
            .unwrap();
        assert_eq!(
            toilet.message.in_language(English),
            "No bathroom with a toilet."
        );
    }

    #[test]
    fn every_sentence_the_review_says_has_its_english() {
        // Every template the rules can say, read from the source: `say!`
        // literals, fixed messages passed to `push`, and the reason a
        // clearance gives, which is a sentence of its own inside another.
        let mut said = 0;
        for file in [
            include_str!("lib.rs"),
            include_str!("corners.rs"),
            include_str!("scene.rs"),
        ] {
            let body = file.split("\nmod tests {").next().unwrap_or(file);
            let mut templates: Vec<String> = Vec::new();
            // The first string literal, escapes and all: the TV's inches are
            // written `{}\"`.
            let first = |text: &str| -> Option<String> {
                let start = text.find('"')? + 1;
                let mut out = String::new();
                let mut chars = text[start..].chars();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => out.push(chars.next()?),
                        '"' => return Some(out),
                        c => out.push(c),
                    }
                }
                None
            };
            for (at, _) in body.match_indices("say!(") {
                templates.extend(first(&body[at..]));
            }
            for call in ["self.push(", "self.push_ref(", "self.push_fig("] {
                for (at, _) in body.match_indices(call) {
                    let args = arguments(&body[at + call.len()..]);
                    let message = args.get(3).copied().unwrap_or_default().trim();
                    if let Some(text) = message.strip_prefix('"').and_then(|m| m.strip_suffix('"'))
                    {
                        templates.push(text.to_owned());
                    }
                }
            }
            for (at, _) in body.match_indices("need(") {
                if let Some(reason) = arguments(&body[at + 5..]).get(4) {
                    templates.extend(literals(reason).into_iter().map(str::to_owned));
                }
            }
            for template in templates {
                said += 1;
                assert!(
                    newera_core::text::english(&template).is_some(),
                    "no English for: {template}"
                );
            }
        }
        assert!(said > 60, "only {said} sentences found: is it reading?");
    }

    /// Every figure a rule asks for by name is in the table: a name nobody
    /// wrote resolves to nothing, and a rule with no figure does not apply —
    /// which is how acceptances end up pruned as orphans.
    #[test]
    fn every_figure_a_rule_asks_for_is_in_the_table() {
        let body = include_str!("lib.rs");
        let body = body.split("\nmod tests {").next().unwrap_or(body);
        let mut asked = 0;
        for (at, _) in body.match_indices("fig(\"") {
            let name = literals(&body[at + 4..])
                .first()
                .copied()
                .unwrap_or_default();
            asked += 1;
            assert!(
                standards::figures().any(|f| f.name == name)
                    // A wheelchair asks `<name>.wheelchair` first, then `<name>`.
                    || standards::figures().any(|f| f.name == format!("{name}.wheelchair")),
                "a rule asks for `{name}`, and no figure is called that"
            );
        }
        assert!(asked > 40, "only {asked} figures asked for: is it reading?");
    }

    /// The severity policy is the whole point of the ladder, so it is tested
    /// on its own rather than only through the rules that lean on it.
    #[test]
    fn a_source_never_claims_more_than_its_tier_allows() {
        let mut home = Home::default();
        square(&mut home, "Sala", 300.0, 300.0);
        let view = home.level_view(home.current_level());
        let scene = Scene::new(&view);
        let profile = Profile::default();
        let place = newera_core::Place::from_city(profile.city.as_deref());
        let mut review = Review {
            scene: &scene,
            figures: standards::figures_at(&place),
            place,
            profile: &profile,
            findings: Vec::new(),
            glass: std::collections::BTreeMap::new(),
        };
        // Tier A, checked at the source: an error stays an error.
        review.push_ref(
            Rule::GasWithoutVent,
            Severity::Erro,
            "Casa",
            "gás sem abertura",
            "nbr13103",
        );
        // Tier A, figure not confirmed at the source: it may warn, never accuse.
        review.push_ref(
            Rule::KitchenWidth,
            Severity::Erro,
            "Casa",
            "cozinha estreita",
            "caixa-mcmv",
        );
        // Tier C doctrine cannot raise an alert, however the rule asked.
        review.push_ref(
            Rule::CounterTotal,
            Severity::Erro,
            "Casa",
            "bancada curta",
            "alexander184",
        );
        // Tier E describes what people do: it raises nothing at all.
        review.push_ref(
            Rule::CounterHeight,
            Severity::Erro,
            "Casa",
            "tendência",
            "houzz2026",
        );
        let got: Vec<(Severity, Option<&str>)> = review
            .findings
            .iter()
            .map(|f| (f.severity, f.reference))
            .collect();
        assert_eq!(
            got,
            vec![
                (Severity::Alerta, Some("nbr13103")),
                (Severity::Alerta, Some("caixa-mcmv")),
                (Severity::Dica, Some("alexander184")),
            ],
            "the survey was dropped and the rest was capped"
        );
    }

    /// Alexander counts the stretch you can work on, not the boxes it was
    /// built from: cabinets that touch are one run.
    #[test]
    fn touching_cabinets_are_one_worktop_run() {
        let mut home = Home::default();
        square(&mut home, "Cozinha", 600.0, 300.0);
        // Three 60 cm cabinets side by side: one run of 180 cm.
        for (n, x) in [(30_u64, 40.0), (31, 100.0), (32, 160.0)] {
            home.furniture.push(piece(
                n,
                "base-cabinet",
                (x, 7.5 + 30.0),
                (60.0, 60.0, 90.0),
                0.0,
            ));
        }
        // One on its own at the far end: a 60 cm stretch nobody can use.
        home.furniture.push(piece(
            33,
            "base-cabinet",
            (540.0, 7.5 + 30.0),
            (60.0, 60.0, 90.0),
            0.0,
        ));
        home.furniture.push(piece(
            34,
            "sink-counter",
            (300.0, 7.5 + 30.0),
            (120.0, 60.0, 90.0),
            0.0,
        ));
        home.furniture.push(piece(
            35,
            "stove",
            (400.0, 7.5 + 31.0),
            (60.0, 62.0, 90.0),
            0.0,
        ));
        let report = review(&home, &Profile::default());
        assert!(
            says(&report, Severity::Dica, "trecho de bancada de só 60 cm"),
            "the lone cabinet is reported, the row of three is not: {report:#?}"
        );
        assert!(
            !says(&report, Severity::Dica, "trecho de bancada de só 180 cm"),
            "{report:#?}"
        );
        // 180 + 60 = 240 cm of free counter, still short of Alexander's 366.
        assert!(
            says(&report, Severity::Dica, "240 cm de bancada livre"),
            "{report:#?}"
        );
    }

    #[test]
    fn a_pantry_tower_is_the_kitchens_storage() {
        let mut home = Home::default();
        square(&mut home, "Cozinha", 500.0, 300.0);
        let mut tower = piece(24, "group", (460.0, 37.5), (70.0, 60.0, 235.0), 0.0);
        tower.name = "Armário 70 × 60 × 235 cm".into();
        tower.properties.insert(
            "joinery:params".into(),
            r#"{"kind":"cabinet","w":70,"h":235,"d":60}"#.into(),
        );
        home.furniture = vec![
            piece(20, "fridge", (50.0, 43.5), (70.0, 72.0, 180.0), 0.0),
            piece(21, "sink-counter", (160.0, 37.5), (120.0, 60.0, 90.0), 0.0),
            piece(22, "base-cabinet", (270.0, 37.5), (100.0, 60.0, 90.0), 0.0),
            piece(23, "stove", (360.0, 37.5), (60.0, 60.0, 90.0), 0.0),
        ];
        let lacks_storage = |home: &Home| {
            review(home, &Profile::default())
                .findings
                .iter()
                .any(|f| f.message.contains("zonas de trabalho falta armazenagem"))
        };
        assert!(lacks_storage(&home), "no storage at all");
        home.furniture.push(tower);
        assert!(!lacks_storage(&home), "the pantry tower is storage");
    }

    #[test]
    fn a_kitchen_spread_too_wide_and_missing_a_zone_is_reported() {
        let mut home = Home::default();
        square(&mut home, "Cozinha", 600.0, 300.0);
        home.furniture.push(piece(
            40,
            "sink-counter",
            (60.0, 7.5 + 30.0),
            (120.0, 60.0, 90.0),
            0.0,
        ));
        // The stove 4,4 m away from the sink: every trip is a crossing.
        home.furniture.push(piece(
            41,
            "stove",
            (500.0, 7.5 + 31.0),
            (60.0, 62.0, 90.0),
            0.0,
        ));
        home.furniture.push(piece(
            42,
            "fridge",
            (300.0, 7.5 + 35.0),
            (70.0, 70.0, 180.0),
            0.0,
        ));
        let report = review(&home, &Profile::default());
        assert!(
            says(&report, Severity::Dica, "entre fogão e pia")
                && cites(&report, Severity::Dica, "alexander184"),
            "{report:#?}"
        );
        // Nowhere to put the plates and nothing to prepare on.
        assert!(
            says(&report, Severity::Dica, "falta armazenagem e preparo")
                && cites(&report, Severity::Dica, "blum-zonas"),
            "{report:#?}"
        );
    }

    /// The plan that was right and kept being accused: a kitchen open to a
    /// living room, open in turn to a balcony whose facade is one long
    /// window. Nothing between them but air, and every round of review said
    /// "gas appliance with no window" and "living room with no window".
    #[test]
    fn light_and_air_cross_a_room_that_is_open_to_the_next() {
        let mut home = Home::default();
        // Three rooms in a row, sharing their boundaries with no wall on them:
        // kitchen 0..300, living 300..600, balcony 600..800 along x.
        let outer = [(0.0, 0.0), (800.0, 0.0), (800.0, 400.0), (0.0, 400.0)];
        for k in 0..4 {
            let (a, b) = (outer[k], outer[(k + 1) % 4]);
            let mut wall = Wall::new(
                WallId(k as u64 + 1),
                Point2::new(a.0, a.1),
                Point2::new(b.0, b.1),
            );
            wall.thickness = 15.0;
            wall.height = 260.0;
            home.walls.push(wall);
        }
        let room = |id: u64, name: &str, x0: f64, x1: f64| {
            Room::new(
                RoomId(id),
                name,
                vec![
                    Point2::new(x0, 7.5),
                    Point2::new(x1, 7.5),
                    Point2::new(x1, 392.5),
                    Point2::new(x0, 392.5),
                ],
            )
        };
        home.rooms.push(room(1, "Cozinha", 7.5, 300.0));
        home.rooms.push(room(2, "Sala", 300.0, 600.0));
        home.rooms.push(room(3, "Varanda", 600.0, 792.5));
        // The only window in the home is on the balcony's facade.
        let mut window = piece(70, "window", (700.0, 392.5), (452.0, 15.0, 155.0), 0.0);
        window.opening = Some(newera_core::Opening {
            kind: OpeningKind::Window,
            ..newera_core::Opening::default()
        });
        window.elevation = 100.0;
        home.furniture.push(window);
        // A gas cooktop in the kitchen, and the rest of a kitchen around it.
        let mut stove = piece(71, "stove", (150.0, 40.0), (75.0, 62.0, 91.0), 0.0);
        stove.name = "Fogão 5 bocas a gás".to_owned();
        home.furniture.push(stove);
        home.furniture.push(piece(
            72,
            "sink-counter",
            (60.0, 40.0),
            (120.0, 60.0, 91.0),
            0.0,
        ));
        home.furniture
            .push(piece(73, "fridge", (260.0, 45.0), (70.0, 70.0, 180.0), 0.0));

        let report = review(&home, &Profile::default());
        assert!(
            !says(
                &report,
                Severity::Erro,
                "sem janela nem abertura permanente"
            ),
            "the balcony airs the kitchen through the living room: {report:#?}"
        );
        assert!(
            !says(&report, Severity::Alerta, "Sem janela"),
            "and lights it: {report:#?}"
        );

        // Close the kitchen off with a wall and the finding comes back, as
        // it should: then there really is nowhere for the gas to go.
        let mut divider = Wall::new(
            WallId(9),
            Point2::new(300.0, 7.5),
            Point2::new(300.0, 392.5),
        );
        divider.thickness = 15.0;
        divider.height = 260.0;
        home.walls.push(divider);
        home.rooms[0] = room(1, "Cozinha", 7.5, 292.5);
        home.rooms[1] = room(2, "Sala", 307.5, 600.0);
        let closed = review(&home, &Profile::default());
        assert!(
            says(
                &closed,
                Severity::Erro,
                "sem janela nem abertura permanente"
            ),
            "{closed:#?}"
        );
    }

    #[test]
    fn a_glass_sliding_door_lights_the_room_like_a_window() {
        let dark = |catalog: &str| {
            let mut home = Home::default();
            square(&mut home, "Sala", 400.0, 400.0);
            let swung = catalog == "door-sliding-swung";
            let catalog = if swung { "door-sliding" } else { catalog };
            let mut door = piece(20, catalog, (200.0, 0.0), (220.0, 15.0, 220.0), 0.0);
            door.opening = Some(newera_core::Opening {
                kind: if catalog == "french-window" {
                    OpeningKind::Window
                } else {
                    OpeningKind::Door
                },
                leaves: 2,
                sliding: catalog != "door" && !swung,
                ..newera_core::Opening::default()
            });
            home.furniture.push(door);
            review(&home, &Profile::default())
                .findings
                .iter()
                .filter(|f| {
                    f.key.starts_with("room_without_window") || f.key.starts_with("window_area")
                })
                .map(|f| f.message.to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(dark("door-sliding"), Vec::<String>::new());
        assert!(
            !dark("door-sliding-swung").is_empty(),
            "made to swing, its leaves are solid"
        );
        assert_eq!(dark("french-window"), Vec::<String>::new());
        assert!(!dark("door").is_empty(), "a wooden door is not a window");
    }

    /// A dishwasher under the stone is 6 cm lower than the stone, by design.
    /// Reading its top as the counter accused a kitchen whose stone was right
    /// — the height of a kitchen is where most of its stone is.
    #[test]
    fn an_appliance_lower_than_the_stone_is_not_read_as_the_counter() {
        let mut home = Home::default();
        square(&mut home, "Cozinha", 400.0, 300.0);
        for (id, x) in [(60_u64, 80.0), (61, 260.0)] {
            home.furniture.push(piece(
                id,
                "sink-counter",
                (x, 7.5 + 30.0),
                (120.0, 60.0, 91.0),
                0.0,
            ));
        }
        home.furniture.push(piece(
            62,
            "stove",
            (180.0, 7.5 + 31.0),
            (60.0, 62.0, 91.0),
            0.0,
        ));
        home.furniture.push(piece(
            63,
            "fridge",
            (350.0, 7.5 + 35.0),
            (70.0, 70.0, 180.0),
            0.0,
        ));
        // The dishwasher: its own 84,5 cm, tucked under the same stone.
        let mut washer = piece(
            64,
            "dishwasher",
            (150.0, 7.5 + 30.0),
            (60.0, 58.0, 84.5),
            0.0,
        );
        washer.name = "LP14V lava-louças".to_owned();
        home.furniture.push(washer);

        // 91 cm is right for someone 165 cm tall, so nothing is said.
        let report = review(&home, &Profile::default());
        assert!(!says(&report, Severity::Dica, "Bancada a"), "{report:#?}");

        // Lower the stone itself and the kitchen is reported, as it should be.
        for piece in &mut home.furniture {
            if piece.catalog == "sink-counter" {
                piece.height = 78.0;
            }
        }
        let report = review(&home, &Profile::default());
        assert!(says(&report, Severity::Dica, "Bancada a 78"), "{report:#?}");
    }

    /// What the laboratory knows: capture is the canopy's job, and a hood
    /// narrower than the burners loses the front ones whatever its airflow.
    #[test]
    fn installed_cooktop_defines_cooking_width_instead_of_its_cabinet() {
        let mut home = Home::default();
        square(&mut home, "Cozinha", 400.0, 300.0);
        let mut host = piece(50, "group", (250.0, 40.0), (70.0, 60.0, 87.0), 0.0);
        host.properties.insert(
            "joinery:params".into(),
            r#"{"kind":"cabinet","cooktop":true}"#.into(),
        );
        let mut cooktop = piece(51, "box", (250.0, 40.0), (60.0, 50.0, 6.0), 0.0);
        cooktop.name = "Cooktop de indução".into();
        cooktop.elevation = 84.6;
        let mut hood = piece(52, "hood", (250.0, 40.0), (60.0, 50.0, 60.0), 0.0);
        hood.elevation = 180.0;
        home.furniture = vec![host, cooktop, hood];
        for reverse in [false, true] {
            if reverse {
                home.furniture.reverse();
            }
            let report = review(&home, &Profile::default());
            assert!(
                !report
                    .findings
                    .iter()
                    .any(|f| f.message.starts_with("Coifa de")),
                "{report:#?}"
            );
            assert!(
                !report
                    .findings
                    .iter()
                    .any(|f| f.message.starts_with("Aparelho a gás")),
                "{report:#?}"
            );
        }
        home.furniture.reverse();
        for piece in &mut home.furniture {
            piece.angle = 90.0;
        }
        assert!(
            !review(&home, &Profile::default())
                .findings
                .iter()
                .any(|f| f.message.starts_with("Coifa de"))
        );
        // A genuinely narrower hood still receives feedback.
        home.furniture[2].width = 50.0;
        assert!(says(
            &review(&home, &Profile::default()),
            Severity::Dica,
            "Coifa de 50 cm sobre cocção de 60 cm"
        ));
        // Missing, hidden, nearby and vertically unrelated appliances do not
        // replace the cooking zone declared in the cabinet.
        home.furniture[2].width = 60.0;
        for (visible, x, z) in [
            (false, 250.0, 84.6),
            (true, 100.0, 84.6),
            (true, 250.0, 150.0),
        ] {
            home.furniture[1].visible = visible;
            home.furniture[1].position.x = x;
            home.furniture[1].elevation = z;
            assert!(says(
                &review(&home, &Profile::default()),
                Severity::Dica,
                "Coifa de 60 cm sobre cocção de 70 cm"
            ));
        }
    }

    #[test]
    fn a_hood_narrower_than_the_cooktop_is_reported_and_a_window_softens_the_gas_rule() {
        let mut home = Home::default();
        square(&mut home, "Cozinha", 400.0, 300.0);
        home.furniture.push(piece(
            50,
            "sink-counter",
            (100.0, 7.5 + 30.0),
            (120.0, 60.0, 90.0),
            0.0,
        ));
        let mut stove = piece(51, "stove", (250.0, 7.5 + 31.0), (75.0, 62.0, 90.0), 0.0);
        stove.name = "Fogão 5 bocas".to_owned();
        home.furniture.push(stove);
        let mut hood = piece(52, "hood", (250.0, 7.5 + 25.0), (60.0, 50.0, 60.0), 0.0);
        hood.elevation = 150.0;
        home.furniture.push(hood);
        let mut window = piece(53, "window", (200.0, 0.0), (120.0, 15.0, 120.0), 0.0);
        window.opening = Some(newera_core::Opening {
            kind: OpeningKind::Window,
            ..newera_core::Opening::default()
        });
        window.elevation = 100.0;
        home.furniture.push(window);
        let report = review(&home, &Profile::default());
        assert!(
            says(
                &report,
                Severity::Dica,
                "Coifa de 60 cm sobre cocção de 75 cm"
            ) && cites(&report, Severity::Dica, "lbnl-coifa"),
            "{report:#?}"
        );
        assert!(
            !says(&report, Severity::Alerta, "Cocção sem coifa"),
            "{report:#?}"
        );
        // With a window it still asks for a permanent opening, without
        // accusing: a window that closes is not ventilation.
        assert!(
            cites(&report, Severity::Alerta, "nbr13103")
                && !report
                    .findings
                    .iter()
                    .any(|f| f.severity == Severity::Erro && f.message.contains("gás")),
            "{report:#?}"
        );
        // Two permanent grilles to the outside close it.
        for (id, elev) in [(60, 20.0), (61, 200.0)] {
            let mut grille = piece(id, "vent-grille", (100.0, 7.5), (20.0, 4.0, 15.0), 0.0);
            grille.elevation = elev;
            home.furniture.push(grille);
        }
        let vented = review(&home, &Profile::default());
        assert!(
            !vented
                .findings
                .iter()
                .any(|f| f.reference == Some("nbr13103")),
            "{vented:#?}"
        );
    }

    #[test]
    fn the_front_is_where_the_doors_are_built_not_where_angle_points() {
        // A sink cabinet imported with angle 0 (front at +y) whose doors and
        // drawers are all built on its -y face, toward a 112 cm corridor. At
        // +y it touches the stone of the peninsula behind it.
        let mut home = Home::default();
        square(&mut home, "Cozinha", 300.0, 300.0);
        let mut sink = piece(20, "sink-counter", (150.0, 150.0), (120.0, 60.0, 90.0), 0.0);
        let panel = |id: u64, name: &str, elev: f64| {
            let mut part = piece(id, "panel", (150.0, 121.0), (118.0, 2.0, 30.0), 0.0);
            part.name = name.to_owned();
            part.elevation = elev;
            part
        };
        sink.children = vec![
            panel(21, "Porta esquerda", 0.0),
            panel(22, "Porta direita", 0.0),
            panel(23, "Gaveta", 30.0),
            panel(24, "Frente fixa", 60.0),
        ];
        assert_eq!(newera_core::facing(&sink), "-y");
        home.furniture.push(sink);
        home.furniture.push(piece(
            30,
            "base-cabinet",
            (150.0, 200.0),
            (120.0, 40.0, 90.0),
            180.0,
        ));
        let report = review(&home, &Profile::default());
        assert!(
            !report
                .findings
                .iter()
                .any(|f| f.place.contains("f20") && f.message.contains("livres à frente")),
            "the corridor is at the doors, not at the stone behind: {report:#?}"
        );

        // Squeeze the corridor at the doors and the same rule speaks, about
        // the side that is really used.
        home.furniture.push(piece(
            31,
            "base-cabinet",
            (150.0, 90.0),
            (120.0, 30.0, 90.0),
            0.0,
        ));
        let report = review(&home, &Profile::default());
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.place.contains("f20") && f.message.contains("15 cm livres à frente")),
            "{report:#?}"
        );
    }

    #[test]
    fn a_fix_does_not_pull_an_appliance_out_of_its_niche() {
        // A sink run on the top wall, 68 cm from a peninsula whose middle
        // module is a dishwasher in a 60 cm niche between two cabinets.
        let mut home = Home::default();
        square(&mut home, "Cozinha", 400.0, 400.0);
        let peninsula = |id, catalog, x| piece(id, catalog, (x, 165.5), (60.0, 60.0, 85.0), 180.0);
        home.furniture.push(peninsula(20, "dishwasher", 190.0));
        home.furniture.push(peninsula(21, "base-cabinet", 130.0));
        home.furniture.push(peninsula(22, "base-cabinet", 250.0));
        home.furniture.push(piece(
            23,
            "sink-counter",
            (190.0, 37.5),
            (180.0, 60.0, 90.0),
            0.0,
        ));
        let report = review(&home, &Profile::default());
        let narrow = report
            .findings
            .iter()
            .find(|f| f.place.contains("f23") && f.message.contains("68 cm livres à frente"))
            .unwrap_or_else(|| panic!("{report:#?}"));
        assert!(
            narrow.fix.is_none(),
            "17 cm into the corridor takes the dishwasher out of its niche: {narrow:#?}"
        );
        assert!(
            report
                .findings
                .iter()
                .filter_map(|f| f.fix.as_ref())
                .all(|fix| fix["ids"][0] != "f20"),
            "{report:#?}"
        );

        // Standing on its own, the same appliance may still be moved.
        home.furniture.retain(|f| f.id.0 != 21 && f.id.0 != 22);
        let report = review(&home, &Profile::default());
        let narrow = report
            .findings
            .iter()
            .find(|f| f.place.contains("f23") && f.message.contains("livres à frente"))
            .unwrap_or_else(|| panic!("{report:#?}"));
        assert_eq!(
            narrow.fix.as_ref().map(|f| f["ids"][0].clone()),
            Some(serde_json::json!("f20")),
            "{narrow:#?}"
        );
    }

    #[test]
    fn a_moulding_over_the_head_has_no_corridor_to_keep() {
        // A kitchen tower's crown moulding at 272 cm, 82 cm from a sofa.
        let mut home = Home::default();
        square(&mut home, "Cozinha", 400.0, 400.0);
        let mut moulding = piece(20, "base-cabinet", (200.0, 9.0), (80.0, 3.0, 8.0), 0.0);
        moulding.name = "moldura de roda-teto".into();
        moulding.elevation = 272.0;
        home.furniture.push(moulding);
        let sofa = piece(
            21,
            "sofa-3",
            (200.0, 10.5 + 82.0 + 45.0),
            (200.0, 90.0, 80.0),
            180.0,
        );
        home.furniture.push(sofa.clone());
        let report = review(&home, &Profile::default());
        assert!(
            !report.findings.iter().any(|f| f.place.contains("f20")),
            "{report:#?}"
        );

        // The same piece on the floor is a counter with a corridor to keep.
        home.furniture[0].elevation = 0.0;
        home.furniture[0].height = 90.0;
        let report = review(&home, &Profile::default());
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.place.contains("f20") && f.message.contains("livres à frente")),
            "{report:#?}"
        );
    }

    #[test]
    fn a_narrow_corner_says_how_much_of_the_side_it_takes() {
        // A 185 cm wardrobe whose doors face a free room, but for a 38 cm
        // nightstand 54 cm in front of one stretch of it.
        let mut home = Home::default();
        square(&mut home, "Quarto", 400.0, 400.0);
        home.furniture.push(piece(
            20,
            "wardrobe",
            (100.0, 37.5),
            (185.0, 60.0, 220.0),
            0.0,
        ));
        home.furniture.push(piece(
            21,
            "nightstand",
            (69.0, 67.5 + 54.0 + 20.0),
            (38.0, 40.0, 55.0),
            0.0,
        ));
        let report = review(&home, &Profile::default());
        let front = report
            .findings
            .iter()
            .find(|f| f.place.contains("f20") && f.message.contains("livres à frente"))
            .unwrap_or_else(|| panic!("{report:#?}"));
        assert!(
            front
                .message
                .starts_with("54 cm livres à frente em 38 dos 185 cm"),
            "{front:#?}"
        );

        let key = front.key.clone();

        // Across the whole front, the sentence stays as it was — and so does
        // the name it is accepted by.
        home.furniture[1].width = 185.0;
        home.furniture[1].position.x = 100.0;
        let report = review(&home, &Profile::default());
        let front = report
            .findings
            .iter()
            .find(|f| f.place.contains("f20") && f.message.contains("livres à frente"))
            .unwrap_or_else(|| panic!("{report:#?}"));
        assert!(
            front.message.starts_with("54 cm livres à frente ("),
            "{front:#?}"
        );
        assert_eq!(front.key, key);
    }

    #[test]
    fn a_hinge_flip_is_offered_only_when_it_clears_the_leaf_for_check_layout_too() {
        // A bathroom door: its leaf, from the left jamb, sweeps over a small
        // cabinet; from the right jamb it would sweep over a wall-hung basin
        // this review reads as built in, and check_layout does not. Both also
        // occupy the approach, which must remain clear regardless of hinge.
        let mut home = Home::default();
        square(&mut home, "Banheiro", 300.0, 240.0);
        let mut door = piece(20, "door", (150.0, 0.0), (70.0, 15.0, 210.0), 0.0);
        door.opening = Some(newera_core::Opening::default());
        home.furniture.push(door);
        home.furniture.push(piece(
            21,
            "base-cabinet",
            (122.0, 60.0),
            (15.0, 15.0, 80.0),
            0.0,
        ));
        let mut basin = piece(22, "sink", (178.0, 60.0), (15.0, 15.0, 18.0), 0.0);
        basin.name = "Lavatório social".into();
        basin.elevation = 70.0;
        home.furniture.push(basin);

        let door = home.furniture[0].clone();
        assert_eq!(
            newera_core::door_blocked_by(&home, &door),
            vec![FurnitureId(21), FurnitureId(22)]
        );
        let mut flipped = door.clone();
        flipped.opening.as_mut().unwrap().hinge_right = true;
        assert_eq!(
            newera_core::door_blocked_by(&home, &flipped),
            vec![FurnitureId(21), FurnitureId(22)]
        );

        let report = review(&home, &Profile::default());
        let hit = report
            .findings
            .iter()
            .find(|f| f.message.contains("A folha da porta bate"))
            .unwrap_or_else(|| panic!("{report:#?}"));
        assert!(
            hit.fix
                .as_ref()
                .is_none_or(|fix| fix["items"][0].get("hinge_right").is_none()),
            "a flip that check_layout would still call blocked is not offered: {hit:#?}"
        );
        assert!(!hit.message.contains("invertendo"), "{hit:#?}");
    }

    #[test]
    fn a_bedroom_reached_only_through_a_passage_is_said_to_have_no_door() {
        let mut home = Home::default();
        square(&mut home, "Dormitório", 300.0, 300.0);
        home.furniture.push(piece(
            20,
            "bed-double",
            (150.0, 111.5),
            (158.0, 208.0, 55.0),
            0.0,
        ));
        let mut passage = piece(21, "passage", (150.0, 300.0), (80.0, 15.0, 210.0), 0.0);
        passage.name = "Passagem".into();
        passage.opening = Some(newera_core::Opening {
            kind: OpeningKind::Passage,
            ..newera_core::Opening::default()
        });
        home.furniture.push(passage);
        let mut leaf = piece(22, "box", (230.0, 290.0), (78.0, 6.0, 208.0), 0.0);
        leaf.name = "Porta dormitório — aberta junto à parede".into();
        home.furniture.push(leaf);

        let report = review(&home, &Profile::default());
        let said = report
            .findings
            .iter()
            .find(|f| f.message.starts_with("Sem porta"))
            .unwrap_or_else(|| panic!("{report:#?}"));
        assert!(
            said.message.contains("f21") && said.message.contains("f22"),
            "{said:#?}"
        );

        // No way in at all — the door deleted — is an error, not silence.
        let mut sealed = home.clone();
        sealed.furniture.retain(|f| f.id.0 != 21);
        let mut elsewhere = piece(23, "door", (-300.0, 150.0), (80.0, 15.0, 210.0), 90.0);
        elsewhere.opening = Some(newera_core::Opening::default());
        sealed.furniture.push(elsewhere);
        sealed.rooms.push(newera_core::Room::new(
            newera_core::RoomId(11),
            "Sala",
            vec![
                Point2::new(-300.0, 0.0),
                Point2::new(0.0, 0.0),
                Point2::new(0.0, 300.0),
                Point2::new(-300.0, 300.0),
            ],
        ));
        let report = review(&sealed, &Profile::default());
        let said = report
            .findings
            .iter()
            .find(|f| f.message.starts_with("Sem acesso"))
            .unwrap_or_else(|| panic!("{report:#?}"));
        assert_eq!(said.severity, Severity::Erro);
        assert!(said.place.contains("Dormitório"), "{said:#?}");
        assert!(
            !report.findings.iter().any(|f| f.place.contains("Sala")
                && (f.message.starts_with("Sem acesso") || f.message.starts_with("Sem porta"))),
            "a living room is not held to it: {report:#?}"
        );

        // A real door there, and nothing to say.
        let door = home.furniture.iter_mut().find(|f| f.id.0 == 21).unwrap();
        door.opening = Some(newera_core::Opening::default());
        let report = review(&home, &Profile::default());
        assert!(
            !report
                .findings
                .iter()
                .any(|f| f.message.starts_with("Sem porta")),
            "{report:#?}"
        );
    }

    #[test]
    fn kitchens_bathrooms_and_doors_follow_the_references() {
        let mut home = Home::default();
        square(&mut home, "Cozinha", 300.0, 240.0);
        home.furniture.push(piece(
            20,
            "stove",
            (100.0, 7.5 + 31.0),
            (60.0, 62.0, 90.0),
            0.0,
        ));
        home.furniture.push(piece(
            21,
            "sink-counter",
            (200.0, 7.5 + 30.0),
            (120.0, 60.0, 90.0),
            0.0,
        ));
        // An island 60 cm in front of the stove.
        home.furniture.push(piece(
            22,
            "kitchen-island",
            (120.0, 69.5 + 60.0 + 45.0),
            (180.0, 90.0, 90.0),
            0.0,
        ));
        let mut door = piece(23, "door", (250.0, 240.0), (64.0, 15.0, 210.0), 0.0);
        door.opening = Some(newera_core::Opening::default());
        home.furniture.push(door);
        let report = review(&home, &Profile::default());
        assert!(
            says(&report, Severity::Alerta, "60 cm livres à frente"),
            "{report:#?}"
        );
        assert!(cites(&report, Severity::Alerta, "nbr15575g"), "{report:#?}");
        // The citation travels in the field, not inside the sentence.
        assert!(
            report.findings.iter().all(|f| !f.message.contains("NBR ")),
            "{report:#?}"
        );
        assert!(
            report.refs.iter().any(|r| r.code == "nbr15575g"),
            "the report resolves what it leaned on: {:?}",
            report.refs
        );
        assert!(
            says(&report, Severity::Dica, "Falta geladeira"),
            "{report:#?}"
        );
        assert!(says(&report, Severity::Dica, "Sem janela"), "{report:#?}");
        assert!(
            !says(&report, Severity::Alerta, "Vão livre"),
            "60 cm clear is enough without a wheelchair"
        );
        // A dresser in the door's swing: the review offers a checked way out.
        let mut blocked = home.clone();
        blocked.furniture.retain(|f| f.id != FurnitureId(22));
        // The door swings into the room.
        blocked
            .furniture
            .iter_mut()
            .find(|f| f.id == FurnitureId(23))
            .unwrap()
            .angle = 180.0;
        blocked.furniture.push(piece(
            24,
            "dresser",
            (228.0, 232.5 - 25.0 - 40.0),
            (60.0, 50.0, 85.0),
            180.0,
        ));
        let report = review(&blocked, &Profile::default());
        let door = report
            .findings
            .iter()
            .find(|f| f.message.contains("A folha da porta bate"))
            .unwrap_or_else(|| panic!("{report:#?}"));
        let fix = door.fix.clone().expect("a fix");
        if fix["tool"] == "update" {
            // The flip names the value to set, not only "the other side".
            let right = fix["items"][0]["hinge_right"].as_bool().unwrap();
            assert!(
                door.message
                    .contains(&format!("hinge_right hoje {}, use {right}", !right)),
                "{door:#?}"
            );
        }
        let mut fixed = blocked.clone();
        match fix["tool"].as_str() {
            Some("update") => {
                let d = fixed
                    .furniture
                    .iter_mut()
                    .find(|f| f.id == FurnitureId(23))
                    .unwrap();
                d.opening.as_mut().unwrap().hinge_right = fix["items"][0]["hinge_right"] == true;
            }
            Some("move") => {
                let id = fix["ids"][0].as_str().unwrap().to_owned();
                let f = fixed
                    .furniture
                    .iter_mut()
                    .find(|f| f.id.to_string() == id)
                    .unwrap();
                f.translate(fix["dx"].as_f64().unwrap(), fix["dy"].as_f64().unwrap());
            }
            _ => panic!("{fix}"),
        }
        assert!(
            !review(&fixed, &Profile::default())
                .findings
                .iter()
                .any(|f| f.message.starts_with("A folha da porta bate")),
            "{fix}"
        );
        // A wheelchair user needs 150 cm to turn and 80 cm doors.
        let wheel = review(
            &home,
            &Profile {
                wheelchair: true,
                ..Profile::default()
            },
        );
        assert!(
            says(&wheel, Severity::Alerta, "80 cm livres")
                && cites(&wheel, Severity::Alerta, "nbr9050"),
            "{wheel:#?}"
        );
        assert!(
            says(&wheel, Severity::Alerta, "giro de cadeira"),
            "{wheel:#?}"
        );
    }
}
