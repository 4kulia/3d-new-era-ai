//! The standards the software leans on, as data instead of prose.
//!
//! A rule that cites its source in the middle of a sentence cannot be
//! clicked, cannot say which edition it followed, and cannot tell how much
//! it matters. So every reference lives here once — code, title, edition,
//! who published it, what it is, and link — and rules carry only the short
//! [`Standard::code`].
//!
//! The tiers are a ladder from what obliges to what merely describes, and
//! they are what turns a reference into a severity: a standard in force where
//! the project stands is not the same kind of claim as a market survey, and
//! treating them alike is how measurements without an owner end up in
//! software. Which rung a source stands on is computed against the place —
//! see [`Standard::force`].
//!
//! Numbers that could not be checked against the primary publication are
//! marked [`Confidence::ConfirmBeforeUse`] and never raise a finding to an
//! error. Paid standards whose figures we do not hold are represented by
//! presence checks — the requirement exists — never by an invented number.

use serde::Serialize;

/// How much a source obliges *at a place* — see [`Standard::force`]. The
/// letter is never stored on a source: the same publication is an A where its
/// authority carries and a B everywhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Tier {
    /// A law, standard or contract whose authority covers the place: breaking
    /// it is a legal or safety problem.
    A,
    /// The same, read outside its territory, or an association's guideline:
    /// good engineering, no legal force here.
    B,
    /// Architects and manufacturers: what makes a kitchen good, not what
    /// makes it legal.
    C,
    /// Laboratory research: empirical, with a published method.
    D,
    /// Market survey: what people do, never what they should do.
    E,
}

impl Tier {
    /// The letter used on the wire and in the interface.
    pub const fn letter(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::E => "E",
        }
    }

    /// What this tier does, in one word.
    pub const fn what(self) -> &'static str {
        match self {
            Self::A => "obriga",
            Self::B => "referencia",
            Self::C => "doutrina",
            Self::D => "mede",
            Self::E => "descreve",
        }
    }
}

/// How much a finding matters, in every discipline.
///
/// It sorts from the heaviest down, so the weaker of two severities is the
/// larger one: `claimed.max(Severity::ceiling(tier))` is how a finding is kept
/// from saying more than its source can.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Something can't be used as drawn.
    #[default]
    Erro,
    /// Below the reference: works badly.
    Alerta,
    /// Would be better.
    Dica,
}

impl Severity {
    /// The most a source of this tier may claim on its own: what obliges can
    /// accuse, what merely describes cannot.
    #[must_use]
    pub const fn ceiling(tier: Tier) -> Self {
        match tier {
            Tier::A => Self::Erro,
            Tier::B | Tier::D => Self::Alerta,
            Tier::C | Tier::E => Self::Dica,
        }
    }
}

/// What a finding of `severity` standing on `code` may claim at `at`, or
/// nothing at all — the one policy every discipline weighs its findings by.
///
/// How much a source obliges is a question about where the project is, not
/// about the source alone: an NBR that accuses in São Paulo warns in Texas. A
/// tier E survey says what people do, never what they should do, so it raises
/// nothing; the tier sets the ceiling; and a figure we could not confirm at
/// the source may warn, never accuse. A finding that cites nothing — common
/// practice, or geometry like two solids in one place — is capped by nothing.
#[must_use]
pub fn weigh(severity: Severity, code: Option<&str>, at: &Place) -> Option<Severity> {
    let Some(source) = code.and_then(standard) else {
        return Some(severity);
    };
    let tier = source.force(at);
    if tier == Tier::E {
        return None;
    }
    let mut severity = severity.max(Severity::ceiling(tier));
    if source.confidence == Confidence::ConfirmBeforeUse {
        severity = severity.max(Severity::Alerta);
    }
    Some(severity)
}

/// Who published a source, and over what territory it speaks.
///
/// Force is a relation between a source and a place, so the territory has to
/// be data: an ABNT standard obliges in Brazil and informs in Florida, and the
/// IRC does the reverse. Nothing here says how much it obliges — that is
/// [`Kind`] — only where its word carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Authority {
    /// Speaks everywhere it is read: ISO, a laboratory, a book.
    Global,
    /// A group of countries, by the codes of its members: `eu`.
    Bloc(&'static str),
    /// One country, normally ISO 3166-1 alpha-2: `br`, `us`.
    Country(&'static str),
    /// A state or province — in the United States it is the state that adopts
    /// a code, so this level is where most American force lives.
    Region {
        country: &'static str,
        region: &'static str,
    },
    /// One municipality.
    City {
        country: &'static str,
        region: &'static str,
        city: &'static str,
    },
}

/// The countries a bloc speaks for.
fn members(bloc: &str) -> &'static [&'static str] {
    match bloc {
        "eu" => &[
            "at", "be", "bg", "hr", "cy", "cz", "dk", "ee", "fi", "fr", "de", "gr", "hu", "ie",
            "it", "lv", "lt", "lu", "mt", "nl", "pl", "pt", "ro", "sk", "si", "es", "se",
        ],
        _ => &[],
    }
}

impl Authority {
    /// Whether this authority's word carries at `at`.
    ///
    /// The country has to match, because that is the question: a standard
    /// published elsewhere is a reference, not a law. Below the country,
    /// silence is not disagreement — a project that names Brazil and no state
    /// is still judged by a state's code, and between the two the stricter one
    /// wins. Naming a *different* city is disagreement, which is how São
    /// Paulo's decree stops following a project to Curitiba.
    #[must_use]
    pub fn covers(&self, at: &Place) -> bool {
        let here = at.country.as_deref();
        let same = |mine: &str, theirs: Option<&str>| theirs.is_none_or(|t| t == mine);
        match self {
            Self::Global => true,
            Self::Bloc(bloc) => here.is_some_and(|c| members(bloc).contains(&c)),
            Self::Country(country) => here == Some(*country),
            Self::Region { country, region } => {
                here == Some(*country) && same(region, at.region.as_deref())
            }
            Self::City {
                country,
                region,
                city,
            } => {
                here == Some(*country)
                    && same(region, at.region.as_deref())
                    && same(city, at.city.as_deref())
            }
        }
    }
}

/// What a source *is*, which is what decides how far it can go.
///
/// A publication's nature travels with it: a law is a law wherever it was
/// written, and a market survey is a survey even at home. Only the three that
/// can oblige care whether they cover the place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Kind {
    /// Statute, decree or regulation: a building code, a sanitary code.
    Law,
    /// A standards body's publication: ABNT, CEN, ISO.
    Standard,
    /// Binding by contract rather than by law: a bank's specification for the
    /// units it finances, a utility's spec for the connection it will accept.
    Contract,
    /// An association's guidance. Good engineering, no force anywhere.
    Guideline,
    /// Architects, designers and manufacturers on how to build well.
    Doctrine,
    /// A manufacturer's own data sheet, about its own product.
    Manufacturer,
    /// Research with a published method.
    Research,
    /// A market survey: what people do, never what they should do.
    Survey,
}

/// Whether a source is still the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Status {
    InForce,
    /// Cancelled in this year. It may still be the convention everybody
    /// draws by, so it keeps advising — it stops obliging.
    Withdrawn(&'static str),
}

/// Whether the figures behind a reference were checked at the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Confidence {
    /// Read in the primary publication, or in a free official version of it.
    Verified,
    /// Plausible and widely repeated, not confirmed at the source: it may
    /// advise, it may not accuse.
    ConfirmBeforeUse,
}

/// One source a rule can stand on.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Standard {
    /// Short key carried by findings, e.g. `nbr13103`.
    pub code: &'static str,
    /// Full name as it should be cited, with the year.
    pub title: &'static str,
    /// Edition or year, alone, for the interface.
    pub edition: &'static str,
    /// Who published it, and where its word carries.
    pub authority: Authority,
    /// What it is, which is how far it can go.
    pub kind: Kind,
    pub status: Status,
    /// What it governs, in one line.
    pub scope: &'static str,
    /// A legitimate public version: official PDF, sector guide or booklet.
    pub url: Option<&'static str>,
    pub confidence: Confidence,
    /// The letter this source is worth no matter where, because nothing in
    /// [`Authority`] or [`Kind`] can express why.
    ///
    /// Two reasons only, and each one is written in a comment at the entry:
    /// the source governs another kind of building than a home, or it is
    /// scoped out of what this software judges. Reaching for it to make a
    /// letter come out right is how the taxonomy rots.
    pub tier_override: Option<Tier>,
}

impl Standard {
    /// How much this source obliges at `at`.
    ///
    /// The same publication is not the same kind of claim everywhere: an ABNT
    /// standard obliges in Brazil and merely informs in Florida, and the IRC
    /// does the reverse. So force is a relation between a source and a place,
    /// never a property of the source alone.
    ///
    /// A source that no longer applies, or applies to another kind of
    /// building, is the one thing this cannot compute: `tier_override` says so
    /// out loud, with the reason in a comment, instead of the taxonomy being
    /// bent until the letter comes out right.
    #[must_use]
    pub fn force(&self, at: &Place) -> Tier {
        if let Some(tier) = self.tier_override {
            return tier;
        }
        // Cancelled: still perhaps the convention, no longer the rule.
        if matches!(self.status, Status::Withdrawn(_)) {
            return Tier::C;
        }
        match self.kind {
            // Only these three can oblige, and only where their word carries.
            Kind::Law | Kind::Standard | Kind::Contract => {
                if self.authority.covers(&at.or_home()) {
                    Tier::A
                } else {
                    Tier::B
                }
            }
            Kind::Guideline => Tier::B,
            Kind::Doctrine => Tier::C,
            Kind::Manufacturer | Kind::Research => Tier::D,
            Kind::Survey => Tier::E,
        }
    }
}

use Confidence::{ConfirmBeforeUse, Verified};

// Eight of the ten fields of one table row, written out once per entry and
// nowhere else; the two that are almost always the same — status and the
// override — are set by `withdrawn` and `held_at` on the few entries that need
// them. A struct to carry the rest would be the same eight names with a type
// wrapped around them, and several hundred call sites longer.
#[allow(clippy::too_many_arguments)]
const fn s(
    code: &'static str,
    title: &'static str,
    edition: &'static str,
    authority: Authority,
    kind: Kind,
    scope: &'static str,
    url: Option<&'static str>,
    confidence: Confidence,
) -> Standard {
    Standard {
        code,
        title,
        edition,
        authority,
        kind,
        status: Status::InForce,
        scope,
        url,
        confidence,
        tier_override: None,
    }
}

impl Standard {
    /// The same entry, cancelled in `year`.
    #[must_use]
    const fn withdrawn(mut self, year: &'static str) -> Self {
        self.status = Status::Withdrawn(year);
        self
    }

    /// The same entry, held to `tier` everywhere. The reason belongs in a
    /// comment beside the call — see [`Standard::tier_override`].
    #[must_use]
    const fn held_at(mut self, tier: Tier) -> Self {
        self.tier_override = Some(tier);
        self
    }
}

/// Brazil, where this software was written and where most of its sources
/// oblige.
const BR: Authority = Authority::Country("br");

/// Every source, grouped by tier.
pub static STANDARDS: &[Standard] = &[
    // ---- A: obliges ----
    s(
        "nbr15575g",
        "ABNT NBR 15575-1:2021 — Desempenho, Anexo F (informativo)",
        "2013 + Emenda 1:2021",
        BR,
        Kind::Standard,
        "Mobiliário mínimo por cômodo e circulação: 0,85 m diante de pia, fogão e geladeira; 0,50 m entre móveis e paredes e diante de assentos; 0,40 m diante de vaso e lavatório; 0,75 m da mesa; guarda-roupa de 1,60 m no casal.",
        Some(
            "https://www.ufsb.edu.br/propa/images/dinfra/coman/Legisla%C3%A7%C3%B5es/NBR15575-1.pdf",
        ),
        Verified,
    ),
    s(
        "nbr15575",
        "ABNT NBR 15575-1:2021 — Norma de Desempenho",
        "2021",
        BR,
        Kind::Standard,
        "Pé-direito mínimo de 2,50 m, e 2,30 m em halls, corredores, banheiros e despensas (16.1.1).",
        Some("https://cbic.org.br/wp-content/uploads/2017/11/Guia_da_Norma_de_Desempenho_2013.pdf"),
        Verified,
    ),
    s(
        "nbr9050",
        "ABNT NBR 9050:2020 — Acessibilidade",
        "2020",
        BR,
        Kind::Standard,
        "Área de aproximação e uso, alcances manuais, alturas de comando e giro de cadeira de rodas.",
        Some("https://www.confea.org.br/midias/acessibilidade_abnt_2022.pdf"),
        Verified,
    ),
    s(
        "nbr13103",
        "ABNT NBR 13103:2024 — Instalação de aparelhos a gás — Requisitos",
        "2024",
        BR,
        Kind::Standard,
        "Volume, ventilação e exaustão por tipo de aparelho (A, B, C), até 75 kW por ambiente; banheiros e ambientes de permanência prolongada só admitem tipo C.",
        Some(
            "https://gasescombustiveis.com.br/seminario/169/palestras/ABNT%20NBR%2013103%20BELO%20HORIZONTE%202025.pdf",
        ),
        ConfirmBeforeUse,
    ),
    s(
        "nbr5410",
        "ABNT NBR 5410 — Instalações elétricas de baixa tensão",
        "2004 (versão corrigida 2008)",
        BR,
        Kind::Standard,
        "Cozinhas e copas: uma tomada a cada 3,5 m de perímetro ou fração, e duas sobre a bancada da pia, no mesmo ponto ou em pontos distintos (9.5.2.2.1 b).",
        Some("https://www.saladaeletrica.com.br/pontos-de-tomada-por-comodo/"),
        Verified,
    ),
    s(
        "nbr14718",
        "ABNT NBR 14718 — Esquadrias — Guarda-corpos para edificação",
        "2019",
        BR,
        Kind::Standard,
        "Altura mínima de 1,10 m do piso ao corrimão (0,90 m sobre mureta), vão entre barras de até 11 cm, nada escalável até 0,45 m, vidro laminado classe 1.",
        Some("https://pdfcoffee.com/gramnbr147182019guarda-corpopdf-3-pdf-free.html"),
        Verified,
    ),
    s(
        "nbr16259",
        "ABNT NBR 16259 — Sistemas de envidraçamentos de sacadas — Requisitos e métodos de ensaio",
        "2014",
        BR,
        Kind::Standard,
        "O envidraçamento não exerce a função de guarda-corpo (3.11); vidro temperado ou laminado de segurança; montado sobre o guarda-corpo, o conjunto atende também a NBR 14718.",
        Some("http://sistemaking.com.br/wp-content/uploads/2021/03/NBR-16259-1.pdf"),
        Verified,
    ),
    s(
        "nbr7199",
        "ABNT NBR 7199 — Vidros na construção civil — Projeto, execução e aplicações",
        "2016",
        BR,
        Kind::Standard,
        "Guarda-corpos de sacadas, escadas e desníveis só com vidro laminado, aramado ou insulado feito deles (tabela 8).",
        Some("https://files.comunidades.net/doutorvidros/NBR_7199_.pdf"),
        Verified,
    ),
    s(
        "nbr13753",
        "ABNT NBR 13753 — Revestimento de piso com placas cerâmicas",
        "1996",
        BR,
        Kind::Standard,
        "Caimento do piso para o ralo: 0,5 % em áreas molhadas, 1,5 % a 2,5 % dentro do box, 1 % a 1,5 % em áreas externas.",
        Some(
            "https://normadedesempenho.com.br/a-execucao-em-conformidade-dos-revestimentos-de-pisos-com-placas-ceramicas/",
        ),
        ConfirmBeforeUse,
    ),
    s(
        "nbr10844",
        "ABNT NBR 10844 — Instalações prediais de águas pluviais",
        "1989",
        BR,
        Kind::Standard,
        "Água de chuva de terraços e áreas descobertas vai ao sistema pluvial, sem ligação com o esgoto.",
        Some(
            "https://ecivilufes.files.wordpress.com/2013/06/nbr-10844-1989-instalac3a7c3b5es-prediais-de-c3a1guas-pluviais.pdf",
        ),
        Verified,
    ),
    s(
        "nbr5626",
        "ABNT NBR 5626:2020 — Sistemas prediais de água fria e água quente — Projeto, execução, operação e manutenção",
        "2020",
        BR,
        Kind::Standard,
        "Substitui a NBR 7198 (água quente): pressão estática até 400 kPa no ponto, dinâmica mínima de 10 kPa, registro antes dos sub-ramais de ao menos um ambiente sanitário.",
        None,
        Verified,
    ),
    s(
        "nbr8160",
        "ABNT NBR 8160:1999 — Sistemas prediais de esgoto sanitário — Projeto e execução",
        "1999",
        BR,
        Kind::Standard,
        "Ramais de descarga por aparelho (tabela 3), caimento recomendado de 2 % até 75 mm e 1 % a partir de 100, ventilação (tabela 1), caixas sifonadas, de gordura e de inspeção.",
        None,
        Verified,
    ),
    s(
        "nbr14565",
        "ABNT NBR 14565 — Cabeamento estruturado para edifícios comerciais",
        "2025",
        BR,
        Kind::Standard,
        "Cabeamento de telecomunicações de edifícios comerciais: enlace permanente até 90 m, canal até 100 m.",
        None,
        ConfirmBeforeUse,
    ),
    s(
        "nm60898",
        "ABNT NBR NM 60898 — Disjuntores para proteção de sobrecorrentes",
        "2004",
        BR,
        Kind::Standard,
        "Correntes nominais preferenciais, capacidade de interrupção (1,5 a 10 kA) e faixas de disparo instantâneo B (3–5 In), C (5–10 In), D (10–20 In).",
        None,
        Verified,
    ),
    s(
        "enel-sp",
        "Enel SP — ET GRI-EDBR-CNC-GRI-0017, conexão individual em baixa tensão",
        "v02, 06/03/2025",
        Authority::Region { country: "br", region: "sp" },
        Kind::Contract,
        "127/220 V: monofásico até 12 kW, bifásico até 20 kW, trifásico até 75 kW; disjuntores de entrada fixos (50, 63, 80, 100… A) e 10 kA até 63 A.",
        Some("https://www.enel.com.br/pt-saopaulo/normas-tecnicas.html"),
        Verified,
    ),
    s(
        "fabricantes",
        "Dados de fabricantes de automação (Shelly, Exatron, Qualitronix)",
        "fichas técnicas",
        Authority::Global,
        Kind::Manufacturer,
        "Dimmer LED 10 W a 1,1 A, consumo em espera até 1–1,2 W, sensor de teto a cerca de 2,4 m cobrindo Ø 7 m.",
        None,
        ConfirmBeforeUse,
    ),
    s(
        "nbr16264",
        "ABNT NBR 16264 — Cabeamento estruturado residencial",
        "2016 (confirmada em 2025)",
        BR,
        Kind::Standard,
        "Tomadas de telecomunicações por cômodo (tabela 1), cabo de 4 pares até o distribuidor de residência, canal até 100 m, tomada de energia junto a cada ponto e ao distribuidor.",
        None,
        ConfirmBeforeUse,
    ),
    s(
        "nbr16415",
        "ABNT NBR 16415 — Caminhos e espaços para cabeamento estruturado",
        "2021",
        BR,
        Kind::Standard,
        "Eletrodutos e caminhos dos cabos de telecomunicações, separados dos de energia.",
        None,
        ConfirmBeforeUse,
    ),
    s(
        "nbr5444",
        "ABNT NBR 5444 — Símbolos gráficos para instalações elétricas prediais",
        "1989 (cancelada em 2014, ainda a convenção de desenho)",
        BR,
        Kind::Standard,
        "Símbolos dos pontos de luz, tomadas, interruptores e quadros na planta.",
        None,
        ConfirmBeforeUse,
    )
    .withdrawn("2014"),
    s(
        "nbr5413",
        "ABNT NBR 5413:1992 — Iluminância de interiores (cancelada pela NBR ISO/CIE 8995-1)",
        "1992 (cancelada em 2013)",
        BR,
        Kind::Standard,
        "A única tabela residencial: sala e dormitório 150 lx (leitura 500), cozinha e banheiro 150 lx com 300 na bancada e no espelho, circulação e garagem 100 lx.",
        Some("http://ftp.demec.ufpr.br/disciplinas/TM802/NBR5413.pdf"),
        Verified,
    )
    .withdrawn("2013"),
    s(
        "nbr8995",
        "ABNT NBR ISO/CIE 8995-1:2013 — Iluminação de ambientes de trabalho (substituiu a NBR 5413)",
        "2013",
        BR,
        Kind::Standard,
        "Locais de trabalho, não residências: de 20 lx a 2.000 lx conforme a tarefa; circulação 100 lx, escrever e ler 500 lx. Os valores residenciais vêm da NBR 5413:1992 (cancelada): sala e dormitório 150, cozinha e banheiro 150 com 300 na bancada e no espelho.",
        None,
        Verified,
    )
    // In force, and Brazilian, so the computation would have it oblige. It
    // governs workplaces, and a home is not one: held at reference on
    // purpose. The letter is a statement about scope, not about authority.
    .held_at(Tier::B),
    s(
        "nbr16280",
        "ABNT NBR 16280 — Reforma em edificações",
        "2020",
        BR,
        Kind::Standard,
        "Reforma em condomínio exige plano com cronograma, segurança e responsável técnico.",
        None,
        Verified,
    ),
    s(
        "nbr14037",
        "ABNT NBR 14037 e NBR 5674 — manual do proprietário e manutenção",
        "2011 / 2012",
        BR,
        Kind::Standard,
        "O que a construtora entrega: manual de uso, sistemas instalados e programa de manutenção.",
        None,
        Verified,
    ),
    s(
        "nbr14810",
        "ABNT NBR 14810 — Painéis de partículas de média densidade (MDP)",
        "2018",
        BR,
        Kind::Standard,
        "Define o MDP: painel de partículas com densidade entre 551 e 750 kg/m³, bom em arranque de parafuso.",
        Some("https://iba.org/psq-paineis"),
        Verified,
    ),
    s(
        "nbr15316",
        "ABNT NBR 15316 — Painéis de fibras de média densidade (MDF)",
        "2021",
        BR,
        Kind::Standard,
        "Define o MDF: painel de fibras de processo seco, com usinabilidade de face e de topo.",
        None,
        Verified,
    ),
    s(
        "caixa-mcmv",
        "Ministério das Cidades — especificações da unidade MCMV/FAR (Portaria MCID 725/2023)",
        "2023",
        BR,
        Kind::Contract,
        "Cozinha de 1,80 m de largura mínima, com previsão de pia 120×50, fogão 55×60 e geladeira 70×70 cm.",
        Some("https://www.legisweb.com.br/legislacao/?id=446563"),
        ConfirmBeforeUse,
    ),
    s(
        "coe-municipal",
        "COE-SP (Lei 16.642/2017, Decreto 57.776/2017) e Código Sanitário estadual (Decreto 12.342/1978)",
        "varia por município",
        BR,
        Kind::Law,
        "São Paulo (Decreto 57.776, 5.A.6): permanência 5 m² e círculo de 2,00 m, cozinha 1,50 m e pé-direito 2,50 m, sanitário, lavanderia e circulação 0,90 m; o estadual, subsidiário: iluminação 1/8 do piso (1/5 trabalho, 1/10 demais), ralo no piso de áreas molhadas. Contra norma, prevalece o mais restritivo.",
        Some("https://www.saopaulo.sp.leg.br/iah/fulltext/decretos/D57776.pdf"),
        Verified,
    ),
    s(
        "rdc216",
        "ANVISA RDC nº 216/2004 — serviços de alimentação",
        "2004",
        BR,
        Kind::Law,
        "Cozinha profissional: estrutura física, revestimentos, higienização, resíduos e pragas.",
        None,
        Verified,
    ),
    // ---- D: measures ----
    s(
        "ibge-adensamento",
        "IBGE — critério de adensamento excessivo",
        "atual",
        BR,
        Kind::Research,
        "Acima de três moradores por dormitório o domicílio é considerado adensado.",
        None,
        Verified,
    ),
    // ---- B: references ----
    s(
        "en1116",
        "EN 1116:2018 — Coordinating sizes for kitchen furniture and appliances",
        "2018",
        Authority::Bloc("eu"),
        Kind::Standard,
        "Larguras nominais de 400, 500, 600 e 900 mm para módulos inferiores, e os nichos de embutir.",
        Some("https://standards.cencenelec.eu/dyn/www/f?p=CEN:110:0::::FSP_PROJECT:41638&cs=1"),
        ConfirmBeforeUse,
    ),
    s(
        "nkba",
        "NKBA Kitchen & Bath Planning Guidelines",
        "5ª edição",
        Authority::Country("us"),
        Kind::Guideline,
        "31 diretrizes de cozinha, cada uma com projeto, exigência de código e Access Standards.",
        Some("https://nkba.org/professional-resources/kitchen-bath-planning-guidelines/"),
        ConfirmBeforeUse,
    ),
    s(
        "irc2024",
        "IRC 2024 / NEC 2023 — elétrica de cozinha (EUA)",
        "2024 / 2023",
        Authority::Country("us"),
        Kind::Law,
        "Bancada a partir de 305 mm exige tomada, e ela não fica a mais de 20 in acima da bancada.",
        None,
        Verified,
    ),
    // ---- C: doctrine ----
    s(
        "alexander184",
        "Christopher Alexander — A Pattern Language, padrão 184 «Cooking Layout»",
        "1977",
        Authority::Global,
        Kind::Doctrine,
        "Bancada total ≥ 366 cm fora de pia, fogão e geladeira; nenhum trecho < 122 cm; nenhum par > 305 cm.",
        None,
        Verified,
    ),
    s(
        "blum-zonas",
        "Blum e Hettich — as cinco zonas de trabalho",
        "atual",
        Authority::Global,
        Kind::Doctrine,
        "Mantimentos, armazenagem, lavagem, preparo e cocção na ordem do fluxo; bancada 10–15 cm abaixo do cotovelo.",
        Some("https://www.blum.com/br/pt/company/dynamic-space/"),
        Verified,
    ),
    s(
        "gilbreth-triangulo",
        "Lillian Gilbreth e o Small Homes Council — o triângulo de trabalho",
        "1929 / anos 1940",
        Authority::Global,
        Kind::Doctrine,
        "Pia, fogão e geladeira próximos; calibrado para cozinha de uma pessoa, sem micro-ondas nem lava-louças.",
        None,
        Verified,
    ),
    s(
        "bulthaup-b1",
        "bulthaup b1 e b3 — a cozinha em três elementos",
        "atual",
        Authority::Global,
        Kind::Doctrine,
        "Ilha, linha de parede e bloco de torres: uma tipologia limpa de layout. Material de marca, sem método publicado.",
        None,
        ConfirmBeforeUse,
    ),
    s(
        "neufert",
        "Ernst Neufert — A Arte de Projetar em Arquitetura",
        "desde 1936",
        Authority::Global,
        Kind::Doctrine,
        "Dimensões antropométricas de referência para cozinha e mobiliário.",
        None,
        ConfirmBeforeUse,
    ),
    s(
        "panero-zelnik",
        "Panero & Zelnik — Dimensionamento Humano para Espaços Interiores",
        "1979",
        Authority::Global,
        Kind::Doctrine,
        "Alcances, folgas e zonas de trabalho derivados de percentis populacionais.",
        None,
        Verified,
    ),
    // ---- D: measures ----
    s(
        "lbnl-coifa",
        "Lawrence Berkeley National Laboratory — eficiência de captura de coifas",
        "2012",
        Authority::Global,
        Kind::Research,
        "Captura de menos de 15 % a mais de 98 % entre modelos de US$ 40 a 650; as que atendem a vazão capturam ≥80 % nas bocas traseiras e ≥50 % nas frontais.",
        Some(
            "https://newscenter.lbl.gov/2012/05/30/berkeley-lab-study-assesses-residential-cooking-exhaust-hoods-ability-to-vent-pollutants/",
        ),
        Verified,
    ),
    // ---- E: describes ----
    s(
        "houzz2026",
        "2026 U.S. Houzz Kitchen Trends Study",
        "2026",
        Authority::Country("us"),
        Kind::Survey,
        "1.780 respondentes americanos em reforma de cozinha, campo em julho de 2025. Não transferível ao Brasil.",
        Some(
            "https://www.houzz.com/magazine/2026-u-s-houzz-kitchen-trends-study-stsetivw-vs~184213864",
        ),
        Verified,
    ),
    s(
        "abimovel",
        "Abimóvel — Anuário Brasil Móveis",
        "2025",
        BR,
        Kind::Survey,
        "A indústria moveleira nacional: 22 mil empresas e receita acima de R$ 91,5 bilhões em 2024.",
        None,
        Verified,
    ),
];

/// The source behind `code`, if it is one we hold.
pub fn standard(code: &str) -> Option<&'static Standard> {
    sources().find(|s| s.code == code)
}

/// Every source we hold: the home pack and the world's, then each foreign
/// country's own.
pub fn sources() -> impl Iterator<Item = &'static Standard> {
    STANDARDS.iter().chain(us::STANDARDS)
}

/// Every figure a rule can ask for, in every pack.
pub fn figures() -> impl Iterator<Item = &'static Figure> {
    FIGURES.iter().chain(us::FIGURES)
}

mod us;

/// Where a project stands, as far as a building code cares.
///
/// Hierarchical and partial, because authority is: a country alone already
/// settles what a national standard demands, a state decides which code was
/// adopted, and a municipality only narrows it further. A place that names
/// less is not a place without rules — it is a place where fewer authorities
/// have spoken, and what is left keeps only the force it can carry.
///
/// Every part is folded on the way in (see [`crate::fold`]), so `São Paulo`
/// and `sao paulo` are the same place and a slug survives being typed by
/// hand.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Place {
    /// Country, normally ISO 3166-1 alpha-2: `br`, `us`, `de`.
    pub country: Option<String>,
    /// State, province or Land: `sp`, `fl`, `by`.
    pub region: Option<String>,
    /// Municipality: `sao-paulo`.
    pub city: Option<String>,
}

impl Place {
    /// The country assumed when a project says nothing about where it is.
    ///
    /// A project with no address is overwhelmingly the common case — the
    /// compass starts with no city — and a review still has to weigh the
    /// standards that oblige *here*, or a gas appliance with no permanent
    /// opening would come out as a warning instead of an error. So silence
    /// means home, not nowhere. It stops being an assumption the moment the
    /// compass carries a country of its own.
    pub const HOME_COUNTRY: &'static str = "br";

    /// A place named from the outside in. Blank parts are simply not named.
    #[must_use]
    pub fn new(country: Option<&str>, region: Option<&str>, city: Option<&str>) -> Self {
        Self {
            country: part(country),
            region: part(region),
            city: part(city),
        }
    }

    /// The place behind the city a project declares, as it is declared today.
    ///
    /// A city we hold brings its state with it — and `estado-sp` is a state
    /// code, not a city, so it names a region and no municipality. A city we
    /// do not hold is still a city: it is kept, so a finding can say which
    /// one, and only the code that would judge it is missing.
    #[must_use]
    pub fn from_city(city: Option<&str>) -> Self {
        match city.map(str::trim).filter(|c| !c.is_empty()) {
            Some(slug) => municipal(slug).map_or_else(
                || Self::new(Some(Self::HOME_COUNTRY), None, Some(slug)),
                MunicipalCode::place,
            ),
            None => Self::new(Some(Self::HOME_COUNTRY), None, None),
        }
    }

    /// The same place with its country filled in when it was left unsaid.
    ///
    /// Judging needs a country, because force is measured against one. A
    /// project that never said where it is gets the home jurisdiction — see
    /// [`Self::HOME_COUNTRY`] — rather than falling off the ladder and having
    /// every standard demoted to a reference.
    #[must_use]
    pub fn or_home(&self) -> Self {
        Self {
            country: Some(
                self.country
                    .clone()
                    .unwrap_or_else(|| Self::HOME_COUNTRY.to_owned()),
            ),
            region: self.region.clone(),
            city: self.city.clone(),
        }
    }

    /// Nothing declared: every local rule may advise, none may accuse.
    #[must_use]
    pub fn is_unknown(&self) -> bool {
        self.country.is_none() && self.region.is_none() && self.city.is_none()
    }

    /// How precisely the place is named, from 0 (nowhere) to 3 (a city).
    ///
    /// A named city without its country is still only as specific as what it
    /// actually says: the count stops at the first part left blank, because
    /// an authority cannot be matched through a hole.
    #[must_use]
    pub fn depth(&self) -> u8 {
        match (&self.country, &self.region, &self.city) {
            (Some(_), Some(_), Some(_)) => 3,
            (Some(_), Some(_), None) => 2,
            (Some(_), None, _) => 1,
            (None, _, _) => 0,
        }
    }
}

/// One part of a place, folded, with the blank treated as unsaid.
pub(crate) fn part(raw: Option<&str>) -> Option<String> {
    let text = crate::fold(raw?.trim());
    (!text.is_empty()).then_some(text)
}

/// Which way a figure binds, so that "the stricter one wins" means something.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum Bound {
    /// At least this much: the larger value is the stricter one.
    AtLeast(f64),
    /// At most this much: the smaller value is the stricter one.
    AtMost(f64),
    /// A value to aim at, from which a rule measures a deviation: the share
    /// of the stature a worktop sits at, the height a transfer is made from.
    /// Neither way is stricter, so between two of them the first one stays.
    Target(f64),
}

impl Bound {
    /// The number, whichever way it binds.
    #[must_use]
    pub const fn value(self) -> f64 {
        match self {
            Self::AtLeast(v) | Self::AtMost(v) | Self::Target(v) => v,
        }
    }

    /// Whether `self` demands more than `other`. Two bounds that face
    /// different ways are not comparable, and `false` keeps the first.
    #[must_use]
    pub fn stricter_than(self, other: Self) -> bool {
        match (self, other) {
            (Self::AtLeast(a), Self::AtLeast(b)) => a > b,
            (Self::AtMost(a), Self::AtMost(b)) => a < b,
            _ => false,
        }
    }
}

/// One number a rule leans on, with who demands it and where.
///
/// The registry used to say only *who* a source is; the numbers lived as
/// literals in the rule bodies, which is why a decree written for one city
/// judged every project. A figure names its authority, so the same rule can
/// ask "what is the minimum here?" and get an answer that stops at the city
/// limits.
///
/// The authority is on the figure and not only on its `source`, because one
/// registry entry can stand for several authorities: `coe-municipal` covers
/// every municipal code, and each city's number is its own. Keeping the code
/// as the thing a finding cites is also what keeps acceptances written under
/// it working.
///
/// A rule keeps its arithmetic — a work triangle is a formula, not a number —
/// and what it compares against lives here. So do the parameters of the
/// formula when somebody published them. What stays in the rule is geometry
/// and classification: which band of a face is read, what counts as "above
/// the counter", that a double bed needs both sides — none of that changes
/// when the project crosses a border.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Figure {
    /// What it is, as a rule asks for it: `room.bedroom.min_area`.
    pub name: &'static str,
    /// Who demands it, and therefore where it applies.
    pub authority: Authority,
    /// The registry code a finding cites for it. `None` is common practice
    /// that no publication states: the finding cites nothing, and nothing
    /// caps it, because there is no tier to cap it with. Requiring a source
    /// here would have meant inventing one, or demoting the finding.
    pub source: Option<&'static str>,
    pub bound: Bound,
    /// How heavy a finding missing it makes, before the source's tier caps it
    /// at the place. It belongs to the figure and not to the tier: the tier is
    /// a ceiling, and most requirements sit well below it — a minimum area in
    /// a municipal code is a tip here, not an error.
    pub severity: Severity,
    /// The article or table it comes from.
    pub note: &'static str,
}

// Six fields, every row: the helpers below only spare the struct noise.
const fn fig(
    name: &'static str,
    authority: Authority,
    source: Option<&'static str>,
    bound: Bound,
    severity: Severity,
    note: &'static str,
) -> Figure {
    Figure {
        name,
        authority,
        source,
        bound,
        severity,
        note,
    }
}

/// A number a Brazilian publication demands.
const fn br(
    name: &'static str,
    source: &'static str,
    bound: Bound,
    severity: Severity,
    note: &'static str,
) -> Figure {
    fig(name, BR, Some(source), bound, severity, note)
}

/// Common practice: no publication states it, so it cites nothing and is
/// capped by nothing, wherever the project is.
const fn practice(
    name: &'static str,
    bound: Bound,
    severity: Severity,
    note: &'static str,
) -> Figure {
    fig(name, Authority::Global, None, bound, severity, note)
}

/// São Paulo, the city, whose decree 57.776 we hold.
const SAO_PAULO: Authority = Authority::City {
    country: "br",
    region: "sp",
    city: "sao-paulo",
};

/// São Paulo, the state, whose sanitary code (decree 12.342/1978) we hold.
const SP_STATE: Authority = Authority::Region {
    country: "br",
    region: "sp",
};

use Bound::{AtLeast, AtMost, Target};
use Severity::{Alerta, Dica, Erro};

/// Every figure a rule asks for.
pub static FIGURES: &[Figure] = &[
    // ---- Who lives here ----
    fig(
        "occupancy.people_per_bedroom",
        BR,
        Some("ibge-adensamento"),
        AtMost(3.0),
        Alerta,
        "IBGE: mais de 3 moradores por dormitório é adensamento excessivo",
    ),
    practice(
        "occupancy.people_per_bedroom.sharing",
        AtMost(2.0),
        Dica,
        "ninguém divide o dormitório com mais de uma pessoa",
    ),
    practice(
        "occupancy.people_per_bathroom",
        AtMost(5.0),
        Alerta,
        "acima de 5 por banheiro as filas de manhã são certas",
    ),
    // Annex F asks 1,60 m in a couple's bedroom and 1,20 m in a single one;
    // per person is our average of the two, so it is not cited as the annex.
    practice(
        "occupancy.wardrobe_per_adult",
        AtLeast(80.0),
        Dica,
        "média do anexo F da NBR 15575-1: 1,60 m no casal, 1,20 m no solteiro",
    ),
    // ---- Doors ----
    practice(
        "door.clear_width",
        AtLeast(60.0),
        Alerta,
        "abaixo de 60 cm não passa um móvel nem uma pessoa com volumes",
    ),
    br(
        "door.clear_width.wheelchair",
        "nbr9050",
        AtLeast(80.0),
        Alerta,
        "NBR 9050 6.11.2.4: vão livre de 0,80 m",
    ),
    // ---- Clearances in front of and beside pieces, cm ----
    br(
        "clearance.bed.side",
        "nbr15575g",
        AtLeast(50.0),
        Alerta,
        "NBR 15575-1 anexo F: 0,50 m entre móveis e paredes",
    ),
    br(
        "clearance.bed.side.wheelchair",
        "nbr9050",
        AtLeast(90.0),
        Alerta,
        "NBR 9050: faixa de transferência lateral",
    ),
    br(
        "clearance.bed.foot",
        "nbr15575g",
        AtLeast(50.0),
        Dica,
        "NBR 15575-1 anexo F: 0,50 m de circulação",
    ),
    practice(
        "clearance.crib.front",
        AtLeast(50.0),
        Dica,
        "acesso ao berço",
    ),
    practice(
        "clearance.wardrobe.front",
        AtLeast(60.0),
        Alerta,
        "abrir as portas e ficar diante delas",
    ),
    // Hinged doors swing their own width, and a person stands past the leaf.
    practice(
        "clearance.wardrobe.past_leaf",
        AtLeast(10.0),
        Alerta,
        "folga além da folha aberta",
    ),
    practice(
        "clearance.dresser.front",
        AtLeast(70.0),
        Dica,
        "abrir gavetas e ficar diante delas",
    ),
    br(
        "clearance.kitchen.front",
        "nbr15575g",
        AtLeast(85.0),
        Alerta,
        "NBR 15575-1 anexo F: 0,85 m diante de pia, fogão e geladeira",
    ),
    br(
        "clearance.kitchen.front.wheelchair",
        "nbr9050",
        AtLeast(150.0),
        Alerta,
        "NBR 9050: giro de 1,50 m",
    ),
    practice(
        "clearance.island.around",
        AtLeast(90.0),
        Alerta,
        "circulação em volta da ilha",
    ),
    br(
        "clearance.toilet.front",
        "nbr15575g",
        AtLeast(40.0),
        Alerta,
        "NBR 15575-1 anexo F: 0,40 m diante do vaso",
    ),
    br(
        "clearance.toilet.front.wheelchair",
        "nbr9050",
        AtLeast(120.0),
        Alerta,
        "NBR 9050: módulo de transferência de 0,80 × 1,20 m",
    ),
    br(
        "clearance.basin.front",
        "nbr15575g",
        AtLeast(40.0),
        Alerta,
        "NBR 15575-1 anexo F: 0,40 m diante do lavatório",
    ),
    br(
        "clearance.basin.front.wheelchair",
        "nbr9050",
        AtLeast(120.0),
        Alerta,
        "NBR 9050: aproximação frontal de 0,80 × 1,20 m",
    ),
    practice(
        "clearance.shower.front",
        AtLeast(60.0),
        Dica,
        "entrar e sair do box",
    ),
    br(
        "clearance.laundry.front",
        "nbr15575g",
        AtLeast(50.0),
        Alerta,
        "NBR 15575-1 anexo F: 0,50 m diante de tanque e máquina",
    ),
    practice(
        "clearance.desk.front",
        AtLeast(75.0),
        Dica,
        "cadeira e levantar-se da mesa",
    ),
    br(
        "clearance.table.seat",
        "nbr15575g",
        AtLeast(75.0),
        Alerta,
        "NBR 15575-1 anexo F: 0,75 m da borda da mesa",
    ),
    // Annex F's 75 cm from the table edge, less the ~35 cm of chair a set
    // already includes.
    br(
        "clearance.dining_set.behind",
        "nbr15575g",
        AtLeast(40.0),
        Alerta,
        "NBR 15575-1 anexo F: 0,75 m da mesa, menos a cadeira",
    ),
    br(
        "clearance.seat.front",
        "nbr15575g",
        AtLeast(50.0),
        Dica,
        "NBR 15575-1 anexo F: 0,50 m diante de assentos",
    ),
    br(
        "clearance.between_beds",
        "nbr15575g",
        AtLeast(60.0),
        Alerta,
        "NBR 15575-1 anexo F: 0,60 m entre camas de solteiro",
    ),
    // ---- The room itself: minimum floor area, cm² ----
    fig(
        "room.bedroom.min_area",
        SAO_PAULO,
        Some("coe-municipal"),
        AtLeast(50_000.0),
        Dica,
        "Decreto 57.776/2017, tabela 5.A.6",
    ),
    fig(
        "room.living.min_area",
        SAO_PAULO,
        Some("coe-municipal"),
        AtLeast(50_000.0),
        Dica,
        "Decreto 57.776/2017, tabela 5.A.6",
    ),
    fig(
        "room.kitchen.min_area",
        SP_STATE,
        Some("coe-municipal"),
        AtLeast(40_000.0),
        Dica,
        "Código Sanitário estadual, decreto 12.342/1978",
    ),
    // ---- Narrowest side, cm ----
    fig(
        "room.bedroom.min_side",
        SAO_PAULO,
        Some("coe-municipal"),
        AtLeast(200.0),
        Dica,
        "Decreto 57.776/2017: círculo de 2 m inscrito",
    ),
    br(
        "room.living.min_side",
        "nbr15575g",
        AtLeast(240.0),
        Dica,
        "NBR 15575-1 anexo F",
    ),
    br(
        "room.kitchen.min_side",
        "nbr15575g",
        AtLeast(150.0),
        Dica,
        "NBR 15575-1 anexo F",
    ),
    br(
        "room.bathroom.min_side",
        "nbr15575g",
        AtLeast(110.0),
        Dica,
        "NBR 15575-1 anexo F",
    ),
    fig(
        "room.laundry.min_side",
        SAO_PAULO,
        Some("coe-municipal"),
        AtLeast(90.0),
        Dica,
        "Decreto 57.776/2017",
    ),
    fig(
        "room.corridor.min_side",
        SAO_PAULO,
        Some("coe-municipal"),
        AtLeast(90.0),
        Alerta,
        "Decreto 57.776/2017",
    ),
    // A wheelchair corridor is the accessibility standard's, not the city's,
    // and it is the longer run that needs the extra width.
    br(
        "room.corridor.min_side.wheelchair",
        "nbr9050",
        AtLeast(90.0),
        Alerta,
        "NBR 9050 6.11.1, até 4 m de extensão",
    ),
    br(
        "room.corridor.min_side.wheelchair.long",
        "nbr9050",
        AtLeast(120.0),
        Alerta,
        "NBR 9050 6.11.1, até 10 m de extensão",
    ),
    // ---- Ceiling height, cm ----
    br(
        "room.min_ceiling",
        "nbr15575",
        AtLeast(250.0),
        Alerta,
        "NBR 15575-1 16.1.1: 2,50 m",
    ),
    br(
        "room.bathroom.min_ceiling",
        "nbr15575",
        AtLeast(230.0),
        Alerta,
        "NBR 15575-1 16.1.1: 2,30 m em halls, corredores, banheiros e despensas",
    ),
    br(
        "room.corridor.min_ceiling",
        "nbr15575",
        AtLeast(230.0),
        Alerta,
        "NBR 15575-1 16.1.1: 2,30 m em halls, corredores, banheiros e despensas",
    ),
    // ---- Daylight: floor area per area of glass ----
    // The state sanitary code's ratios, cited as they have always been. They
    // stand on the whole country for now because nothing else says what a
    // room outside São Paulo needs; see docs/JURISDICTIONS.md.
    br(
        "room.floor_per_glass.work",
        "coe-municipal",
        AtMost(5.0),
        Dica,
        "Código Sanitário de SP, decreto 12.342/1978, art. 44: 1/5 para trabalhar ou estudar",
    ),
    br(
        "room.floor_per_glass",
        "coe-municipal",
        AtMost(8.0),
        Dica,
        "Código Sanitário de SP, decreto 12.342/1978, art. 44: 1/8 para dormir, estar, cozinhar e lavar",
    ),
    br(
        "room.floor_per_glass.service",
        "coe-municipal",
        AtMost(10.0),
        Dica,
        "Código Sanitário de SP, decreto 12.342/1978, art. 44: 1/10 nos demais",
    ),
    br(
        "room.wheelchair_turn",
        "nbr9050",
        AtLeast(150.0),
        Alerta,
        "NBR 9050 4.3.4: giro de 360° em 1,50 m",
    ),
    // ---- The kitchen at work ----
    fig(
        "kitchen.triangle.total",
        Authority::Global,
        Some("nkba"),
        AtMost(792.0),
        Dica,
        "NKBA, diretriz 3: 26 ft somados",
    ),
    fig(
        "kitchen.triangle.leg",
        Authority::Global,
        Some("nkba"),
        AtMost(274.0),
        Dica,
        "NKBA, diretriz 3: até 9 ft por lado",
    ),
    fig(
        "kitchen.triangle.leg.min",
        Authority::Global,
        Some("nkba"),
        AtLeast(122.0),
        Dica,
        "NKBA, diretriz 3: ao menos 4 ft por lado",
    ),
    practice(
        "kitchen.sink_to_stove",
        AtLeast(60.0),
        Alerta,
        "bancada entre pia e fogão; a NKBA soma 90 cm de apoio entre os dois",
    ),
    // Worktop height: about 10 to 15 cm below the elbow, which is at 63 % of
    // the stature.
    fig(
        "kitchen.counter.elbow_share",
        Authority::Global,
        Some("blum-zonas"),
        Target(0.63),
        Dica,
        "cotovelo a 63 % da estatura",
    ),
    fig(
        "kitchen.counter.below_elbow",
        Authority::Global,
        Some("blum-zonas"),
        Target(12.0),
        Dica,
        "bancada de 10 a 15 cm abaixo do cotovelo",
    ),
    fig(
        "kitchen.counter.tolerance",
        Authority::Global,
        Some("blum-zonas"),
        AtMost(6.0),
        Dica,
        "desvio tolerado da altura ideal",
    ),
    practice(
        "kitchen.wall_cabinet.min_elevation",
        AtLeast(135.0),
        Alerta,
        "45 cm sobre a bancada, prática de marcenaria",
    ),
    // The top shelf of a wall cabinet within reach: 1,2 times the stature,
    // or 1,20 m from a wheelchair, and a little over it is still usable.
    fig(
        "kitchen.wall_cabinet.reach_share",
        Authority::Global,
        Some("panero-zelnik"),
        Target(1.2),
        Dica,
        "alcance vertical de 1,2 vez a estatura",
    ),
    br(
        "kitchen.wall_cabinet.reach.wheelchair",
        "nbr9050",
        AtMost(120.0),
        Dica,
        "NBR 9050 4.6: alcance manual de quem usa cadeira de rodas",
    ),
    practice(
        "kitchen.wall_cabinet.over_reach",
        AtMost(30.0),
        Dica,
        "o que ainda se alcança acima do alcance confortável",
    ),
    fig(
        "kitchen.counter.total",
        Authority::Global,
        Some("alexander184"),
        AtLeast(366.0),
        Dica,
        "Alexander, padrão 184: 12 ft de bancada",
    ),
    fig(
        "kitchen.counter.run",
        Authority::Global,
        Some("alexander184"),
        AtLeast(122.0),
        Dica,
        "Alexander, padrão 184: nenhum trecho abaixo de 4 ft",
    ),
    fig(
        "kitchen.spread",
        Authority::Global,
        Some("alexander184"),
        AtMost(305.0),
        Dica,
        "Alexander, padrão 184: nenhum par a mais de 10 ft",
    ),
    br(
        "kitchen.sockets.perimeter_each",
        "nbr5410",
        AtMost(350.0),
        Erro,
        "NBR 5410 9.5.2.2.1: um ponto a cada 3,5 m de perímetro ou fração",
    ),
    br(
        "kitchen.sockets.over_counter",
        "nbr5410",
        AtLeast(2.0),
        Erro,
        "NBR 5410 9.5.2.2.1 b): duas tomadas sobre a bancada da pia",
    ),
    br(
        "kitchen.width",
        "caixa-mcmv",
        AtLeast(180.0),
        Dica,
        "Caixa, especificação do Minha Casa Minha Vida",
    ),
    fig(
        "kitchen.free_circle",
        SAO_PAULO,
        Some("coe-municipal"),
        AtLeast(150.0),
        Erro,
        "Decreto 57.776/2017, tabela 5.A.6",
    ),
    // ---- Reach, sight and height, for whoever lives here ----
    practice(
        "screen.distance.near",
        AtLeast(1.2),
        Dica,
        "1,2 vez a diagonal: perto disso a imagem não cabe no olhar",
    ),
    practice(
        "screen.distance.far",
        AtMost(2.5),
        Dica,
        "2,5 vezes a diagonal: longe disso o texto não se lê",
    ),
    br(
        "reach.top.wheelchair",
        "nbr9050",
        AtMost(85.0),
        Alerta,
        "NBR 9050: tampo a no máximo 0,85 m, com vão livre embaixo",
    ),
    br(
        "reach.bed.wheelchair",
        "nbr9050",
        Target(46.0),
        Dica,
        "NBR 9050: cama a uns 0,46 m para a transferência",
    ),
    br(
        "reach.bed.wheelchair.tolerance",
        "nbr9050",
        AtMost(4.0),
        Dica,
        "desvio tolerado da altura de transferência",
    ),
    br(
        "reach.switch.min",
        "nbr9050",
        AtLeast(60.0),
        Alerta,
        "NBR 9050 4.6.9, figura 26: interruptores de 0,60 a 1,00 m",
    ),
    br(
        "reach.switch.max",
        "nbr9050",
        AtMost(100.0),
        Alerta,
        "NBR 9050 4.6.9, figura 26: interruptores de 0,60 a 1,00 m",
    ),
    br(
        "reach.outlet.min",
        "nbr9050",
        AtLeast(40.0),
        Alerta,
        "NBR 9050 4.6.9, figura 26: tomadas de 0,40 a 1,00 m",
    ),
    br(
        "reach.outlet.max",
        "nbr9050",
        AtMost(100.0),
        Alerta,
        "NBR 9050 4.6.9, figura 26: tomadas de 0,40 a 1,00 m",
    ),
];

/// A figure as it stands at one place.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Resolved {
    pub figure: &'static Figure,
    /// The place names everything the authority names: the city of a city's
    /// decree, the state of a state's code. A figure heard because the place
    /// left a level unsaid is still heard — silence is not disagreement — but
    /// a rule that must not accuse on a guess asks this first.
    pub named: bool,
}

impl std::ops::Deref for Resolved {
    type Target = Figure;

    fn deref(&self) -> &Figure {
        self.figure
    }
}

/// Whether `at` names every level `authority` names.
fn names(authority: &Authority, at: &Place) -> bool {
    match authority {
        Authority::Global | Authority::Bloc(_) | Authority::Country(_) => true,
        Authority::Region { .. } => at.region.is_some(),
        Authority::City { .. } => at.city.is_some(),
    }
}

/// The strictest figure called `name` whose authority carries at `at`.
///
/// Silence below the country is not disagreement, so a project that has not
/// said which city it is in still hears what a city demands, at the figure's
/// own severity; [`Resolved::named`] says it was not named. Naming *another*
/// city is disagreement, and that is where a local figure goes quiet.
///
/// Where no figure carries at all, what the home country's own standards say
/// still informs, the way [`Standard::force`] keeps a foreign standard as a
/// reference rather than silencing it: a bedroom in Miami is still measured by
/// annex F until a code of its own is here, and the finding cites annex F at
/// the letter it has there. Only national figures travel. A city's decree or
/// a state's code never speaks outside its territory, and a figure a foreign
/// country publishes never speaks at home: adding one must not move a finding
/// in Brazil.
#[must_use]
pub fn figure(name: &str, at: &Place) -> Option<Resolved> {
    let here = at.or_home();
    let strictest = |rows: &mut dyn Iterator<Item = &'static Figure>| {
        rows.reduce(|best, f| {
            if f.bound.stricter_than(best.bound) {
                f
            } else {
                best
            }
        })
    };
    let named = figures().filter(|f| f.name == name && f.authority.covers(&here));
    if let Some(figure) = strictest(&mut named.into_iter()) {
        return Some(Resolved {
            figure,
            named: names(&figure.authority, &here),
        });
    }
    let home = figures()
        .filter(|f| f.name == name && f.authority == Authority::Country(Place::HOME_COUNTRY));
    strictest(&mut home.into_iter()).map(|figure| Resolved {
        figure,
        named: false,
    })
}

/// Every figure, resolved once at `at`, by name.
///
/// A review is re-run for every fix it weighs and for every storey an
/// acceptance is checked on, so the table is read once per review and the
/// rules ask this instead.
#[must_use]
pub fn figures_at(at: &Place) -> std::collections::BTreeMap<&'static str, Resolved> {
    figures()
        .filter_map(|f| figure(f.name, at).map(|r| (f.name, r)))
        .collect()
}

/// A municipal or state building code we hold, for a picker and for the
/// place it names.
///
/// Its numbers change from city to city and, against a standard, the more
/// restrictive one wins — so they are never constants inside a rule, and not
/// fields here either: they are rows of [`FIGURES`] under the authority of the
/// place this code speaks for.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct MunicipalCode {
    /// Slug used in tool arguments, e.g. `sao-paulo`.
    pub city: &'static str,
    /// Name to show, e.g. `São Paulo — SP`.
    pub label: &'static str,
    /// The code itself, as it is cited. Its numbers are in [`FIGURES`],
    /// under this place's authority.
    pub source: &'static str,
    /// Country, as a [`Place`] names it.
    pub country: &'static str,
    /// State or province, as a [`Place`] names it.
    pub region: &'static str,
    /// The municipality, when this entry is a city's code. A state code
    /// leaves it unsaid: it speaks for every city of the state that has
    /// nothing stricter of its own.
    pub municipality: Option<&'static str>,
}

impl MunicipalCode {
    /// The place this code speaks for.
    #[must_use]
    pub fn place(&self) -> Place {
        Place::new(Some(self.country), Some(self.region), self.municipality)
    }
}

/// Cities whose code we hold.
pub static MUNICIPAL_CODES: &[MunicipalCode] = &[
    MunicipalCode {
        city: "sao-paulo",
        label: "São Paulo — SP",
        source: "Decreto 57.776/2017, tabela 5.A.6",
        country: "br",
        region: "sp",
        municipality: Some("sao-paulo"),
    },
    MunicipalCode {
        city: "estado-sp",
        label: "Estado de São Paulo (Código Sanitário)",
        source: "Decreto estadual 12.342/1978 (cozinha de 4 m², sem círculo)",
        country: "br",
        region: "sp",
        municipality: None,
    },
];

/// The code of `city`, if we hold it. Accepts the slug or the label.
pub fn municipal(city: &str) -> Option<&'static MunicipalCode> {
    let want = city.trim();
    MUNICIPAL_CODES
        .iter()
        .find(|m| m.city.eq_ignore_ascii_case(want) || m.label.eq_ignore_ascii_case(want))
}

/// Every city we hold, for a picker.
pub const fn cities() -> &'static [MunicipalCode] {
    MUNICIPAL_CODES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_resolvable() {
        for s in sources() {
            assert_eq!(standard(s.code), Some(s), "{} resolves to itself", s.code);
            assert_eq!(
                sources().filter(|o| o.code == s.code).count(),
                1,
                "{} appears once",
                s.code
            );
            assert!(!s.title.is_empty() && !s.edition.is_empty(), "{}", s.code);
        }
        assert!(standard("inventada").is_none());
    }

    #[test]
    fn paid_standards_we_did_not_read_ask_to_be_confirmed() {
        // The dossier's own rule: a figure we could not check at the source
        // may advise, never accuse.
        for code in ["caixa-mcmv", "nkba", "bulthaup-b1", "nbr13103", "en1116"] {
            assert_eq!(
                standard(code).unwrap().confidence,
                Confidence::ConfirmBeforeUse,
                "{code}"
            );
        }
    }

    #[test]
    fn a_place_is_folded_and_a_blank_part_is_left_unsaid() {
        let sp = Place::new(Some("BR"), Some("SP"), Some("São Paulo"));
        assert_eq!(sp.country.as_deref(), Some("br"));
        assert_eq!(sp.region.as_deref(), Some("sp"));
        assert_eq!(
            sp.city.as_deref(),
            Some("sao paulo"),
            "an accent never makes it a different place"
        );
        assert_eq!(sp, Place::new(Some("br"), Some(" sp "), Some("sao paulo")));

        let blank = Place::new(Some("BR"), Some("   "), None);
        assert_eq!(blank.region, None, "whitespace is not a region");
        assert!(!blank.is_unknown());
        assert!(Place::default().is_unknown());
    }

    #[test]
    fn a_place_is_only_as_specific_as_what_it_names() {
        assert_eq!(Place::default().depth(), 0);
        assert_eq!(Place::new(Some("us"), None, None).depth(), 1);
        assert_eq!(Place::new(Some("us"), Some("fl"), None).depth(), 2);
        assert_eq!(
            Place::new(Some("br"), Some("sp"), Some("sao-paulo")).depth(),
            3
        );
        // A city with no state behind it cannot be matched through the hole:
        // the state is what adopts a code in the United States.
        assert_eq!(Place::new(Some("us"), None, Some("miami")).depth(), 1);
        assert_eq!(Place::new(None, None, Some("sao-paulo")).depth(), 0);
    }

    #[test]
    fn the_city_a_project_declares_becomes_the_place_it_is_judged_at() {
        // A city we hold brings its state with it.
        let sp = Place::from_city(Some("sao-paulo"));
        assert_eq!(sp, Place::new(Some("br"), Some("sp"), Some("sao-paulo")));

        // `estado-sp` is a state code sitting in the city table: it names a
        // region and no municipality, or it would judge as if it were a city.
        let state = Place::from_city(Some("estado-sp"));
        assert_eq!(state, Place::new(Some("br"), Some("sp"), None));
        assert_eq!(state.depth(), 2);

        // A city we do not hold is still a city; only its code is missing.
        let unknown = Place::from_city(Some("Curitiba"));
        assert_eq!(unknown.city.as_deref(), Some("curitiba"));
        assert_eq!(unknown.region, None);

        // Silence means home, not nowhere: what obliges here still obliges.
        for quiet in [None, Some(""), Some("   ")] {
            let place = Place::from_city(quiet);
            assert_eq!(place.country.as_deref(), Some(Place::HOME_COUNTRY));
            assert!(!place.is_unknown(), "{quiet:?}");
            assert_eq!(place.city, None, "{quiet:?}");
        }
    }

    /// What every source is worth in Brazil, written by hand.
    ///
    /// This is the letter the registry carried on each entry before force was
    /// computed, kept here as the thing the computation answers to. It lives
    /// in the test and not on the entry so that a source has one authority,
    /// one nature and one status — and no second opinion to drift from.
    const IN_BRAZIL: &[(&str, Tier)] = &[
        ("nbr15575g", Tier::A),
        ("nbr15575", Tier::A),
        ("nbr9050", Tier::A),
        ("nbr13103", Tier::A),
        ("nbr5410", Tier::A),
        ("nbr14718", Tier::A),
        ("nbr16259", Tier::A),
        ("nbr7199", Tier::A),
        ("nbr13753", Tier::A),
        ("nbr10844", Tier::A),
        ("nbr5626", Tier::A),
        ("nbr8160", Tier::A),
        ("nbr14565", Tier::A),
        ("nm60898", Tier::A),
        ("enel-sp", Tier::A),
        ("fabricantes", Tier::D),
        ("nbr16264", Tier::A),
        ("nbr16415", Tier::A),
        ("nbr5444", Tier::C),
        ("nbr5413", Tier::C),
        ("nbr8995", Tier::B),
        ("nbr16280", Tier::A),
        ("nbr14037", Tier::A),
        ("nbr14810", Tier::A),
        ("nbr15316", Tier::A),
        ("caixa-mcmv", Tier::A),
        ("coe-municipal", Tier::A),
        ("rdc216", Tier::A),
        ("ibge-adensamento", Tier::D),
        ("en1116", Tier::B),
        ("nkba", Tier::B),
        ("irc2024", Tier::B),
        ("irc2024-planning", Tier::B),
        ("icc-a117", Tier::B),
        ("alexander184", Tier::C),
        ("blum-zonas", Tier::C),
        ("gilbreth-triangulo", Tier::C),
        ("bulthaup-b1", Tier::C),
        ("neufert", Tier::C),
        ("panero-zelnik", Tier::C),
        ("lbnl-coifa", Tier::D),
        ("houzz2026", Tier::E),
        ("abimovel", Tier::E),
    ];

    #[test]
    fn every_source_is_worth_in_brazil_what_it_was_worth_before_force_was_computed() {
        assert_eq!(
            IN_BRAZIL.len(),
            sources().count(),
            "a source was added or removed without saying what it is worth here"
        );
        // Every Brazilian way of saying where the project is, including saying
        // nothing — which is the common case, and the one where a silent
        // demotion would turn an error into a warning.
        for at in [
            Place::from_city(Some("sao-paulo")),
            Place::from_city(Some("estado-sp")),
            Place::from_city(Some("curitiba")),
            Place::from_city(None),
            Place::default(),
        ] {
            for &(code, expected) in IN_BRAZIL {
                let source = standard(code).unwrap_or_else(|| panic!("{code} is in the registry"));
                assert_eq!(source.force(&at), expected, "{code} at {at:?}");
            }
        }
    }

    #[test]
    fn the_same_source_changes_force_when_the_place_changes() {
        let br = Place::from_city(Some("sao-paulo"));
        let fl = Place::new(Some("us"), Some("fl"), Some("miami"));
        let de = Place::new(Some("de"), None, None);
        let at = |code: &str| standard(code).unwrap();

        // An ABNT standard obliges here and informs there.
        assert_eq!(at("nbr9050").force(&br), Tier::A);
        assert_eq!(at("nbr9050").force(&fl), Tier::B);

        // And the IRC does the reverse — which is the whole point.
        assert_eq!(at("irc2024").force(&br), Tier::B);
        assert_eq!(at("irc2024").force(&fl), Tier::A);

        // A European standard obliges in a member state, not in Brazil.
        assert_eq!(at("en1116").force(&br), Tier::B);
        assert_eq!(at("en1116").force(&de), Tier::A);

        // What a source *is* travels with it: an association's guidance never
        // obliges, doctrine stays doctrine, a survey stays silent.
        for place in [&br, &fl, &de] {
            assert_eq!(at("nkba").force(place), Tier::B);
            assert_eq!(at("alexander184").force(place), Tier::C);
            assert_eq!(at("lbnl-coifa").force(place), Tier::D);
            assert_eq!(at("houzz2026").force(place), Tier::E);
            // Cancelled in 2013: still the only residential lux table, so it
            // keeps advising everywhere and obliges nowhere.
            assert_eq!(at("nbr5413").force(place), Tier::C);
        }

        // A state's authority is not contradicted by a project that names only
        // the country, and is contradicted by another state.
        let enel = at("enel-sp");
        assert_eq!(enel.force(&Place::new(Some("br"), None, None)), Tier::A);
        assert_eq!(
            enel.force(&Place::new(Some("br"), Some("rj"), None)),
            Tier::B
        );
    }

    #[test]
    fn a_city_code_stops_following_a_project_to_another_city() {
        // The reason the model exists: São Paulo's decree must not judge a
        // project in Curitiba. The registry entry `coe-municipal` still
        // stands for every municipal code, so the city is on the figures, and
        // the rule is proven on the authority itself.
        let sp = Authority::City {
            country: "br",
            region: "sp",
            city: "sao-paulo",
        };
        assert!(sp.covers(&Place::from_city(Some("sao-paulo"))));
        assert!(
            sp.covers(&Place::new(Some("br"), None, None)),
            "silence below the country is not disagreement: the stricter one wins"
        );
        assert!(!sp.covers(&Place::from_city(Some("curitiba"))));
        assert!(!sp.covers(&Place::new(Some("us"), Some("fl"), Some("miami"))));
    }

    /// The string literals of an expression, from `from` up to the comma
    /// or closing bracket that ends it.
    fn literals_of(text: &str) -> Vec<&str> {
        let (mut depth, mut out, mut chars) = (0i32, Vec::new(), text.char_indices());
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => {
                    let rest = &text[i + 1..];
                    let end = rest.find('"').unwrap_or(rest.len());
                    out.push(&rest[..end]);
                    for _ in 0..=end {
                        chars.next();
                    }
                }
                '(' | '{' | '[' => depth += 1,
                ')' | '}' | ']' | ',' | ';' if depth == 0 => break,
                ')' | '}' | ']' => depth -= 1,
                _ => {}
            }
        }
        out
    }

    /// The part of a source file above its tests.
    fn body(file: &str) -> &str {
        file.split("\nmod tests {").next().unwrap_or(file)
    }

    #[test]
    fn every_source_a_discipline_cites_is_in_the_registry() {
        // An unknown code resolves to nothing: the finding shows no edition
        // and no link, and — once force is computed from the entry — no
        // letter either. The disciplines cite free strings, so the check has
        // to read what they wrote.
        let files = [
            ("electrical.rs", include_str!("electrical.rs")),
            ("plumbing.rs", include_str!("plumbing.rs")),
            ("guard.rs", include_str!("guard.rs")),
            ("lighting.rs", include_str!("lighting.rs")),
        ];
        let mut cited = 0;
        for (name, file) in files {
            let text = body(file);
            for (at, _) in text.match_indices("source:") {
                // A field, not the middle of a key like `plumb:source:water`.
                if !text[..at].ends_with(char::is_whitespace) {
                    continue;
                }
                let after = &text[at + "source:".len()..];
                // The field's declaration, not a value.
                if after.trim_start().starts_with('&') {
                    continue;
                }
                for code in literals_of(after) {
                    cited += 1;
                    assert!(
                        standard(code).is_some(),
                        "{name} cites `{code}`, which is not in the registry"
                    );
                }
            }
        }
        assert!(
            cited > 40,
            "the scan found only {cited} citations: is it reading?"
        );
        for f in figures() {
            if let Some(code) = f.source {
                assert!(
                    standard(code).is_some(),
                    "figure {} cites `{code}`, which is not in the registry",
                    f.name
                );
            }
        }
    }

    #[test]
    fn a_figure_says_whether_the_place_named_its_authority() {
        let circle = |at: &Place| figure("kitchen.free_circle", at);
        // Named: São Paulo's decree judges.
        let sp = circle(&Place::from_city(Some("sao-paulo"))).unwrap();
        assert!(sp.named);
        assert_eq!(sp.bound, Bound::AtLeast(150.0));
        // Unsaid: still heard, because silence is not disagreement — but a
        // rule that must not accuse on a guess can tell.
        for quiet in [
            Place::from_city(None),
            Place::from_city(Some("estado-sp")),
            Place::default(),
        ] {
            let heard = circle(&quiet).unwrap_or_else(|| panic!("{quiet:?}"));
            assert!(!heard.named, "{quiet:?}");
        }
        // Another city disagrees: nothing.
        assert!(circle(&Place::from_city(Some("curitiba"))).is_none());
    }

    #[test]
    fn a_national_figure_informs_abroad_and_a_local_one_never_travels() {
        let miami = Place::new(Some("us"), Some("fl"), Some("miami"));
        // Annex F still measures a kitchen in Miami, as a reference: nothing
        // there says otherwise yet, and the finding cites annex F at the letter
        // it has there.
        let side = figure("room.kitchen.min_side", &miami).unwrap();
        assert!(!side.named);
        assert_eq!(side.source, Some("nbr15575g"));
        assert_eq!(standard("nbr15575g").unwrap().force(&miami), Tier::B);
        // A city's decree and a state's code stay home: Miami hears the IRC
        // about a bedroom's area, never São Paulo's decree, and nothing about
        // a kitchen's, which the IRC exempts.
        assert_eq!(
            figure("room.bedroom.min_area", &miami).map(|f| f.source),
            Some(Some("irc2024-planning"))
        );
        assert!(figure("room.kitchen.min_area", &miami).is_none());
        assert!(figure("kitchen.free_circle", &miami).is_none());
        // What no one publishes travels everywhere, as what it is.
        let crib = figure("clearance.crib.front", &miami).unwrap();
        assert_eq!(crib.source, None);
    }

    /// Where each figure resolves, rule by rule and place by place.
    ///
    /// "No figure here, so the rule does not apply" is what feeds
    /// `orphaned()`, and `accept(prune=true)` deletes what that lists. So a
    /// figure missing at a place is a decision, and this is where it is
    /// written down: a name may go quiet only where the home pack's own rows
    /// for it are a city's or a state's, and only outside their territory with
    /// no row of the place's own country to answer instead.
    #[test]
    fn every_figure_resolves_everywhere_unless_it_is_local() {
        let places = [
            Place::default(),
            Place::from_city(None),
            Place::from_city(Some("sao-paulo")),
            Place::from_city(Some("estado-sp")),
            Place::from_city(Some("curitiba")),
            Place::new(Some("br"), Some("rj"), None),
            Place::new(Some("us"), Some("fl"), Some("miami")),
            Place::new(Some("de"), None, None),
            Place::new(Some("pt"), None, None),
        ];
        let mut names: Vec<&str> = figures().map(|f| f.name).collect();
        names.sort_unstable();
        names.dedup();
        for name in names {
            // The home pack: everything but another country's own rows.
            let foreign = |f: &&Figure| matches!(f.authority, Authority::Country(c) if c != Place::HOME_COUNTRY);
            let local = figures()
                .filter(|f| f.name == name && !foreign(f))
                .all(|f| {
                    matches!(
                        f.authority,
                        Authority::City { .. } | Authority::Region { .. }
                    )
                });
            for at in &places {
                let found = figure(name, at);
                if local {
                    // Quiet only where nothing covers: another city or state
                    // named, and no row of the country's own.
                    let covered = figures()
                        .filter(|f| f.name == name)
                        .any(|f| f.authority.covers(&at.or_home()));
                    assert_eq!(found.is_some(), covered, "{name} at {at:?}");
                } else {
                    assert!(found.is_some(), "{name} goes quiet at {at:?}");
                }
            }
        }
    }

    #[test]
    fn in_miami_the_irc_and_a117_judge_and_at_home_they_say_nothing() {
        let miami = Place::new(Some("us"), Some("fl"), Some("miami"));
        let at = |name: &str, place: &Place| {
            let found = figure(name, place).unwrap_or_else(|| panic!("{name} at {place:?}"));
            (found.bound.value(), found.source, found.named)
        };
        // IRC R304.1: 70 sq ft, and it obliges there.
        let (area, source, named) = at("room.bedroom.min_area", &miami);
        assert!((area - 65_032.1).abs() < 1.0, "{area}");
        assert_eq!(source, Some("irc2024-planning"));
        assert!(named);
        assert_eq!(standard("irc2024-planning").unwrap().force(&miami), Tier::A);
        // R305.1: 7 ft, 6 ft 8 in where water is.
        assert!((at("room.min_ceiling", &miami).0 - 213.36).abs() < 0.01);
        assert!((at("room.bathroom.min_ceiling", &miami).0 - 203.2).abs() < 0.01);
        assert!((at("room.corridor.min_ceiling", &miami).0 - 213.36).abs() < 0.01);
        // R307.1 is stricter than annex F in front of a toilet: 21 in.
        assert!((at("clearance.toilet.front", &miami).0 - 53.34).abs() < 0.01);
        // What the pack has no row for still hears the home standard, as a
        // reference.
        let (side, source, named) = at("clearance.bed.side", &miami);
        assert_eq!((side, source, named), (50.0, Some("nbr15575g"), false));

        // And none of it reaches home, named or not.
        for home in [
            Place::default(),
            Place::from_city(Some("sao-paulo")),
            Place::from_city(Some("curitiba")),
        ] {
            for f in figures() {
                if let Some(found) = figure(f.name, &home) {
                    assert_ne!(
                        found.authority,
                        Authority::Country("us"),
                        "{} at {home:?}",
                        f.name
                    );
                }
            }
            assert!((at("clearance.toilet.front", &home).0 - 40.0).abs() < f64::EPSILON);
            assert!((at("room.min_ceiling", &home).0 - 250.0).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn a_figure_row_is_whole() {
        for f in figures() {
            assert!(!f.note.is_empty(), "{} says where it comes from", f.name);
            assert!(
                f.bound.value().is_finite() && f.bound.value() > 0.0,
                "{}",
                f.name
            );
            // Common practice is everybody's, or it is somebody's publication.
            if f.source.is_none() {
                assert_eq!(f.authority, Authority::Global, "{}", f.name);
            }
        }
    }

    #[test]
    fn cities_are_found_by_slug_or_label() {
        let sp = municipal("sao-paulo").unwrap();
        assert_eq!(
            sp.place(),
            Place::new(Some("br"), Some("sp"), Some("sao-paulo"))
        );
        assert_eq!(municipal("São Paulo — SP"), Some(sp));
        assert!(municipal("atlantis").is_none());
        assert_ne!(cities(), []);
    }
}
