//! Interface language. Texts are written in Portuguese in the code and
//! looked up here when another language is chosen; unknown texts stay as
//! written.

use std::sync::atomic::{AtomicU8, Ordering};

/// A language the window speaks.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Lang {
    /// Portuguese (Brazil): the language the texts are written in, so it
    /// needs no table.
    #[default]
    Pt,
    En,
    Es,
    Fr,
}

impl Lang {
    /// Every language, in the order the menu lists them.
    pub(crate) const ALL: [Self; 4] = [Self::Pt, Self::En, Self::Es, Self::Fr];

    /// How the language names itself — the one name whoever looks for it
    /// reads without knowing any of the others.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Pt => "Português (Brasil)",
            Self::En => "English",
            Self::Es => "Español",
            Self::Fr => "Français",
        }
    }

    /// The two letters a locale name starts with for this language.
    pub(crate) const fn tag(self) -> &'static str {
        match self {
            Self::Pt => "pt",
            Self::En => "en",
            Self::Es => "es",
            Self::Fr => "fr",
        }
    }

    /// Whether numbers are written with a comma for the decimals: everyone
    /// here but English.
    pub(crate) const fn decimal_comma(self) -> bool {
        !matches!(self, Self::En)
    }

    /// Which side of a table row the language is on; Portuguese is the key
    /// of the row, not a side of it.
    const fn column(self) -> Option<usize> {
        match self {
            Self::Pt => None,
            Self::En => Some(0),
            Self::Es => Some(1),
            Self::Fr => Some(2),
        }
    }

    const fn code(self) -> u8 {
        match self {
            Self::Pt => 0,
            Self::En => 1,
            Self::Es => 2,
            Self::Fr => 3,
        }
    }

    const fn from_code(code: u8) -> Self {
        match code {
            1 => Self::En,
            2 => Self::Es,
            3 => Self::Fr,
            _ => Self::Pt,
        }
    }

    /// The language a locale name asks for — `pt_BR.UTF-8`, `fr-CA`, `es` —
    /// or nothing when the window does not speak it.
    fn from_locale(locale: &str) -> Option<Self> {
        let tag = locale.get(..2)?.to_ascii_lowercase();
        Self::ALL.into_iter().find(|lang| lang.tag() == tag)
    }

    /// What the system asks for, when the window speaks it. `NEWERA_LANG`
    /// comes first, as it does in the installers.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn from_system() -> Option<Self> {
        ["NEWERA_LANG", "LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|name| std::env::var(name).ok())
            .find_map(|locale| Self::from_locale(&locale))
    }

    /// In the browser there is no environment to read: the language is the
    /// one the browser itself is set to.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn from_system() -> Option<Self> {
        Self::from_locale(&web_sys::window()?.navigator().language()?)
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

/// Switches the interface language.
pub(crate) fn set(lang: Lang) {
    LANG.store(lang.code(), Ordering::Relaxed);
}

pub(crate) fn lang() -> Lang {
    Lang::from_code(LANG.load(Ordering::Relaxed))
}

/// The language a finding is said in. Findings carry Portuguese and English;
/// where the window speaks neither, English is the one more readers share.
pub(crate) fn findings_language() -> newera_core::vocabulary::Language {
    match lang() {
        Lang::Pt => newera_core::vocabulary::Language::Portuguese,
        Lang::En | Lang::Es | Lang::Fr => newera_core::vocabulary::Language::English,
    }
}

/// Whether numbers on screen take a comma for the decimals.
pub(crate) fn decimal_comma() -> bool {
    lang().decimal_comma()
}

/// The text in the current language.
/// Translates a line that only exists at run time: what a background job says
/// it is doing comes from the work itself, not from a literal in this crate.
pub(crate) fn dynamic(pt: &str) -> String {
    let Some(column) = lang().column() else {
        return pt.to_owned();
    };
    translations(pt).map_or_else(|| pt.to_owned(), |row| row[column].to_owned())
}

pub(crate) fn tr(pt: &'static str) -> &'static str {
    say(pt, lang())
}

/// The text in one named language, whatever the window is set to. The window
/// itself always wants [`tr`]; this is for the tests, which run beside each
/// other and must not shout across.
fn say(pt: &'static str, lang: Lang) -> &'static str {
    let Some(column) = lang.column() else {
        return pt;
    };
    translations(pt).map_or(pt, |row| row[column])
}

/// A sentence with things in it: `{}` in the text marks where each one goes,
/// in every language, so a translation is free to put them elsewhere.
pub(crate) fn fill(pt: &'static str, parts: &[&dyn std::fmt::Display]) -> String {
    fill_in(pt, lang(), parts)
}

/// [`fill`] in one named language. See [`say`].
fn fill_in(pt: &'static str, lang: Lang, parts: &[&dyn std::fmt::Display]) -> String {
    let mut text = String::new();
    let mut rest = say(pt, lang);
    for part in parts {
        let Some((before, after)) = rest.split_once("{}") else {
            break;
        };
        text.push_str(before);
        text.push_str(&part.to_string());
        rest = after;
    }
    text.push_str(rest);
    text
}

/// What the text says in English, Spanish and French, in this order.
#[allow(clippy::too_many_lines, clippy::match_same_arms)]
fn translations(pt: &str) -> Option<[&'static str; 3]> {
    Some(match pt {
        " · arco" => [" · arc", " · arco", " · arc"],
        "A tarefa termina sozinha em segundo plano; o resultado é descartado." => [
            "The task finishes on its own in the background; its result is thrown away.",
            "La tarea termina sola en segundo plano; su resultado se descarta.",
            "La tâche se termine seule en arrière-plan ; son résultat est ignoré.",
        ],
        "Abrindo imagens e modelos" => [
            "Opening images and models",
            "Abriendo imágenes y modelos",
            "Ouverture des images et des modèles",
        ],
        "Abrindo projeto" => [
            "Opening project",
            "Abriendo proyecto",
            "Ouverture du projet",
        ],
        "Desenhando a planta" => ["Drawing the plan", "Dibujando el plano", "Tracé du plan"],
        "Exportando" => ["Exporting", "Exportando", "Exportation"],
        "Extraindo imagens e modelos" => [
            "Extracting images and models",
            "Extrayendo imágenes y modelos",
            "Extraction des images et des modèles",
        ],
        "Forçar e encerrar a espera" => [
            "Force it and stop waiting",
            "Forzar y dejar de esperar",
            "Forcer et arrêter d'attendre",
        ],
        "Gravando o projeto" => [
            "Writing the project",
            "Grabando el proyecto",
            "Écriture du projet",
        ],
        "Guardando imagens e modelos" => [
            "Storing images and models",
            "Guardando imágenes y modelos",
            "Enregistrement des images et des modèles",
        ],
        "Importando modelo" => [
            "Importing model",
            "Importando modelo",
            "Importation du modèle",
        ],
        "Lendo o arquivo" => [
            "Reading the file",
            "Leyendo el archivo",
            "Lecture du fichier",
        ],
        "Lendo o projeto" => [
            "Reading the project",
            "Leyendo el proyecto",
            "Lecture du projet",
        ],
        "Montando o modelo 3D" => [
            "Building the 3D model",
            "Montando el modelo 3D",
            "Construction du modèle 3D",
        ],
        "Parando…" => ["Stopping…", "Deteniendo…", "Arrêt…"],
        "Salvando projeto" => [
            "Saving project",
            "Guardando proyecto",
            "Enregistrement du projet",
        ],
        "Trabalhando…" => ["Working…", "Trabajando…", "Travail en cours…"],
        "⚠ Espera encerrada — a tarefa termina em segundo plano." => [
            "⚠ Stopped waiting — the task finishes in the background.",
            "⚠ Espera terminada — la tarea sigue en segundo plano.",
            "⚠ Attente interrompue — la tâche se termine en arrière-plan.",
        ],
        "Ajustar ao telhado" => ["Fit to the roof", "Ajustar al tejado", "Ajuster au toit"],
        "Código de obras" => [
            "Building code",
            "Código de obras",
            "Code de la construction",
        ],
        "Embutir peça no móvel selecionado" => [
            "Embed the piece in the selected joinery",
            "Empotrar la pieza en el mueble seleccionado",
            "Encastrer la pièce dans le meuble sélectionné",
        ],
        "Embutido:" => ["Embedded:", "Empotrado:", "Encastré :"],
        "Armários na parede" => [
            "Cabinets on the wall",
            "Armarios en la pared",
            "Meubles sur le mur",
        ],
        "Armários na parede…" => [
            "Cabinets on the wall…",
            "Armarios en la pared…",
            "Meubles sur le mur…",
        ],
        "Balcões" => ["Base cabinets", "Muebles bajos", "Meubles bas"],
        "Aéreos" => ["Wall cabinets", "Muebles altos", "Meubles hauts"],
        "Entre a norma e a lei do município, prevalece o mais restritivo." => [
            "Between the standard and the city's law, the more restrictive one wins.",
            "Entre la norma y la ley del municipio, prevalece la más restrictiva.",
            "Entre la norme et la loi de la commune, la plus restrictive l'emporte.",
        ],
        "Fontes desta revisão:" => [
            "Sources of this review:",
            "Fuentes de esta revisión:",
            "Sources de cette révision :",
        ],
        "Não informado" => ["Not set", "Sin definir", "Non défini"],
        "Referências brasileiras onde existe norma; áreas e janelas variam com o código de obras do município." => {
            [
                "Brazilian standards where one exists; areas and windows vary with the city's building code.",
                "Referencias brasileñas donde existe norma; áreas y ventanas varían según el código de obras del municipio.",
                "Normes brésiliennes lorsqu'elles existent ; surfaces et fenêtres varient selon le code de la construction de la commune.",
            ]
        }
        "Torres / guarda-roupa" => [
            "Tall / wardrobe",
            "Columnas / armario ropero",
            "Colonnes / dressing",
        ],
        "Pia centrada em" => [
            "Sink centered at",
            "Fregadero centrado en",
            "Évier centré à",
        ],
        "Cooktop centrado em" => [
            "Cooktop centered at",
            "Placa de cocción centrada en",
            "Table de cuisson centrée à",
        ],
        "descreve" => ["describes", "describe", "décrit"],
        "do início da parede" => [
            "from the wall start",
            "desde el inicio de la pared",
            "depuis le début du mur",
        ],
        "Bancada de pedra por cima" => [
            "Stone countertop on top",
            "Encimera de piedra encima",
            "Plan de travail en pierre au-dessus",
        ],
        "Frentes" => ["Fronts", "Frentes", "Façades"],
        "Gaveteiros" => ["Drawer units", "Cajoneras", "Blocs-tiroirs"],
        "automático" => ["automatic", "automático", "automatique"],
        "Mede o trecho livre entre cantos, portas, janelas e eletrodomésticos e divide em módulos sem sobras; substitui os armários que já estão nessa parede." => {
            [
                "Measures the free stretch between corners, doors, windows and appliances and splits it into modules with no leftovers; replaces the cabinets already on that wall.",
                "Mide el tramo libre entre esquinas, puertas, ventanas y electrodomésticos y lo divide en módulos sin sobras; sustituye los armarios que ya están en esa pared.",
                "Mesure la portion libre entre angles, portes, fenêtres et appareils, puis la divise en modules sans chute ; remplace les meubles déjà posés sur ce mur.",
            ]
        }
        "Pré-visualizar" => ["Preview", "Vista previa", "Aperçu"],
        "Criar armários" => ["Build cabinets", "Crear armarios", "Créer les meubles"],
        "Criado (Ctrl+Z desfaz):" => [
            "Built (Ctrl+Z undoes it):",
            "Creado (Ctrl+Z deshace):",
            "Créé (Ctrl+Z annule) :",
        ],
        "Prévia:" => ["Preview:", "Vista previa:", "Aperçu :"],
        "doutrina" => ["doctrine", "doctrina", "doctrine"],
        "mede" => ["measures", "mide", "mesure"],
        "obriga" => ["obliges", "obliga", "impose"],
        "portas" => ["doors", "puertas", "portes"],
        "gaveteiro" => ["drawers", "cajonera", "tiroirs"],
        "porta-temperos" => ["pull-out", "especiero", "coulissant"],
        "canto cego" => ["blind corner", "esquina ciega", "angle borgne"],
        "cabideiro" => ["hanging", "barra de colgar", "penderie"],
        "referencia" => ["references", "referencia", "référence"],
        "sobre a geladeira" => [
            "over the fridge",
            "sobre el frigorífico",
            "au-dessus du réfrigérateur",
        ],
        "pia" => ["sink", "fregadero", "évier"],
        "tamponamento" => ["filler", "panel de relleno", "fileur"],
        "bancada" => ["countertop", "encimera", "plan de travail"],
        "Ergonomia" => ["Ergonomics", "Ergonomía", "Ergonomie"],
        "Ergonomia…" => ["Ergonomics…", "Ergonomía…", "Ergonomie…"],
        "Moradores" => ["Occupants", "Habitantes", "Occupants"],
        "Crianças" => ["Children", "Niños", "Enfants"],
        "Idosos" => ["Elderly", "Mayores", "Personnes âgées"],
        "Altura de quem cozinha" => [
            "Cook's height",
            "Altura de quien cocina",
            "Taille de la personne qui cuisine",
        ],
        "Alguém usa cadeira de rodas (NBR 9050)" => [
            "Someone uses a wheelchair (NBR 9050)",
            "Alguien usa silla de ruedas (NBR 9050)",
            "Une personne en fauteuil roulant (NBR 9050)",
        ],
        "Incluir elétrica na nota" => [
            "Include electrical in score",
            "Incluir electricidad en la nota",
            "Inclure l’électricité dans la note",
        ],
        "Incluir hidráulica na nota" => [
            "Include plumbing in score",
            "Incluir fontanería en la nota",
            "Inclure la plomberie dans la note",
        ],
        "Pendências fora do escopo continuam visíveis. A nota não certifica o projeto." => [
            "Findings outside the scope remain visible. The score does not certify the project.",
            "Los hallazgos fuera del alcance siguen visibles. La nota no certifica el proyecto.",
            "Les constats hors périmètre restent visibles. La note ne certifie pas le projet.",
        ],
        "Fora do escopo da nota" => [
            "Outside score scope",
            "Fuera del alcance de la nota",
            "Hors périmètre de la note",
        ],
        "nota de habitabilidade" => [
            "habitability score",
            "nota de habitabilidad",
            "note d'habitabilité",
        ],
        "lugares para dormir" => ["beds", "plazas para dormir", "couchages"],
        "banheiros" => ["bathrooms", "baños", "salles de bain"],
        "à mesa" => ["at the table", "en la mesa", "à table"],
        "na sala" => ["in the living room", "en el salón", "au salon"],
        "de guarda-roupa" => ["of wardrobe", "de armario ropero", "de dressing"],
        "Nada a apontar para esses moradores." => [
            "Nothing to point out for these occupants.",
            "Nada que señalar para estos habitantes.",
            "Rien à signaler pour ces occupants.",
        ],
        "Selecionar na planta" => [
            "Select on the plan",
            "Seleccionar en el plano",
            "Sélectionner sur le plan",
        ],
        "Aplicar correção" => ["Apply fix", "Aplicar corrección", "Appliquer la correction"],
        "Correção aplicada (Ctrl+Z desfaz)." => [
            "Fix applied (Ctrl+Z undoes it).",
            "Corrección aplicada (Ctrl+Z deshace).",
            "Correction appliquée (Ctrl+Z annule).",
        ],
        "Referências: NBR 9050, NBR 15575-1 e IBGE; áreas e janelas variam com o código de obras do município." => {
            [
                "References: NBR 9050, NBR 15575-1 and IBGE; areas and windows vary with the city's building code.",
                "Referencias: NBR 9050, NBR 15575-1 e IBGE; áreas y ventanas varían según el código de obras del municipio.",
                "Références : NBR 9050, NBR 15575-1 et IBGE ; surfaces et fenêtres varient selon le code de la construction de la commune.",
            ]
        }
        " · luz" => [" · light", " · luz", " · lumière"],
        "(cópia)" => ["(copy)", "(copia)", "(copie)"],
        "Abrir" => ["Open", "Abrir", "Ouvrir"],
        "Abrir (Ctrl+O)" => ["Open (Ctrl+O)", "Abrir (Ctrl+O)", "Ouvrir (Ctrl+O)"],
        "Abrir link" => ["Open link", "Abrir enlace", "Ouvrir le lien"],
        "Abrir…" => ["Open…", "Abrir…", "Ouvrir…"],
        "Adicionar texto" => ["Add text", "Añadir texto", "Ajouter du texte"],
        "Adicionar um andar acima do mais alto" => [
            "Add a storey above the highest one",
            "Añadir una planta encima de la más alta",
            "Ajouter un étage au-dessus du plus haut",
        ],
        "Afastamento" => ["Offset", "Separación", "Décalage"],
        "Afastar (Ctrl -)" => [
            "Zoom out (Ctrl -)",
            "Alejar (Ctrl -)",
            "Zoom arrière (Ctrl -)",
        ],
        "Ajuda" => ["Help", "Ayuda", "Aide"],
        "Quadro de cargas e NBR 5410" => [
            "Load schedule and NBR 5410",
            "Cuadro de cargas y NBR 5410",
            "Tableau des charges et NBR 5410",
        ],
        "Nenhum circuito: atribua os pontos a circuitos (MCP electrical assign)." => [
            "No circuits: assign the points to circuits (MCP electrical assign).",
            "Ningún circuito: asigne los puntos a circuitos (MCP electrical assign).",
            "Aucun circuit : affectez les points à des circuits (MCP electrical assign).",
        ],
        "Circuito" => ["Circuit", "Circuito", "Circuit"],
        "Pontos" => ["Points", "Puntos", "Points"],
        "VA" => ["VA", "VA", "VA"],
        "V" => ["V", "V", "V"],
        "A" => ["A", "A", "A"],
        "Fio mm²" => ["Wire mm²", "Hilo mm²", "Fil mm²"],
        "Disjuntor" => ["Breaker", "Magnetotérmico", "Disjoncteur"],
        "DR" => ["RCD", "Diferencial", "Différentiel"],
        "Total instalado:" => ["Installed total:", "Total instalado:", "Total installé :"],
        "Nada a apontar." => [
            "Nothing to point out.",
            "Nada que señalar.",
            "Rien à signaler.",
        ],
        "Camadas" => ["Layers", "Capas", "Calques"],
        "Iluminação" => ["Lighting", "Iluminación", "Éclairage"],
        "Eletrodomésticos" => ["Appliances", "Electrodomésticos", "Appareils"],
        "Marcenaria" => ["Joinery", "Carpintería", "Menuiserie"],
        "Mostrar ou esconder na planta e no 3D" => [
            "Show or hide on the plan and in 3D",
            "Mostrar u ocultar en el plano y en 3D",
            "Afficher ou masquer sur le plan et en 3D",
        ],
        "Mostrar tudo no 3D" => [
            "Show everything in 3D",
            "Mostrar todo en 3D",
            "Tout afficher en 3D",
        ],
        "Ligado, o 3D mostra a obra inteira; desligado, esconde o que a planta esconde" => [
            "On, the 3D shows the whole building; off, it hides what the plan hides",
            "Activado, el 3D muestra la obra entera; desactivado, oculta lo que el plano oculta",
            "Activé, la 3D montre tout le bâtiment ; désactivé, elle masque ce que le plan masque",
        ],
        "Enviar relatórios de erro" => [
            "Send error reports",
            "Enviar informes de error",
            "Envoyer les rapports d'erreur",
        ],
        "Falhas e notas de uso vão para os desenvolvedores, sem o seu projeto, sem IP e sem nome da máquina." => {
            [
                "Crashes and usage notes go to the developers, without your project, your IP or your machine's name.",
                "Los fallos y las notas de uso van a los desarrolladores, sin su proyecto, sin IP y sin el nombre de la máquina.",
                "Les plantages et les notes d'usage vont aux développeurs, sans votre projet, sans IP ni nom de machine.",
            ]
        }
        "Ajustes…" => ["Settings…", "Ajustes…", "Réglages…"],
        "Alinhamento" => ["Alignment", "Alineación", "Alignement"],
        "Altura" => ["Height", "Altura", "Hauteur"],
        "Andar" => ["Storey", "Planta", "Étage"],
        "Aproximar (Ctrl +)" => [
            "Zoom in (Ctrl +)",
            "Acercar (Ctrl +)",
            "Zoom avant (Ctrl +)",
        ],
        "Apoiar o projeto" => [
            "Support the project",
            "Apoyar el proyecto",
            "Soutenir le projet",
        ],
        "Arco" => ["Arc", "Arco", "Arc"],
        "Arquitetura" => ["Architecture", "Arquitectura", "Architecture"],
        "Arquivo" => ["File", "Archivo", "Fichier"],
        "Arraste para mover a vista" => [
            "Drag to move the view",
            "Arrastre para mover la vista",
            "Faites glisser pour déplacer la vue",
        ],
        "Arraste: girar · Shift/botão do meio: mover · Scroll: zoom · F: enquadrar" => [
            "Drag: orbit · Shift/middle button: pan · Scroll: zoom · F: frame",
            "Arrastrar: girar · Mayús/botón central: mover · Rueda: zoom · F: encuadrar",
            "Glisser : pivoter · Maj/bouton du milieu : déplacer · Molette : zoom · F : cadrer",
        ],
        "Atalhos e ferramentas" => [
            "Shortcuts and tools",
            "Atajos y herramientas",
            "Raccourcis et outils",
        ],
        "Automático (símbolos e vista de cima dos modelos)" => [
            "Automatic (symbols, top views for models)",
            "Automático (símbolos y vista superior de los modelos)",
            "Automatique (symboles, vues de dessus pour les modèles)",
        ],
        "Autor" => ["Author", "Autor", "Auteur"],
        "Boa" => ["Good", "Buena", "Bonne"],
        "Buscar: cama, janela, sofá…" => [
            "Search: bed, window, sofa…",
            "Buscar: cama, ventana, sofá…",
            "Rechercher : lit, fenêtre, canapé…",
        ],
        "Calibrar e posicionar" => [
            "Calibrate and place",
            "Calibrar y posicionar",
            "Calibrer et placer",
        ],
        "Calibrar escala da imagem" => [
            "Calibrate image scale",
            "Calibrar escala de la imagen",
            "Calibrer l'échelle de l'image",
        ],
        "Cancelar" => ["Cancel", "Cancelar", "Annuler"],
        "Casa e bússola" => ["Home and compass", "Casa y brújula", "Maison et boussole"],
        "Casa e bússola…" => [
            "Home and compass…",
            "Casa y brújula…",
            "Maison et boussole…",
        ],
        "Catálogo" => ["Catalog", "Catálogo", "Catalogue"],
        "Catálogo de origem" => [
            "Source catalog",
            "Catálogo de origen",
            "Catalogue d'origine",
        ],
        "Centro" => ["Center", "Centro", "Centre"],
        "Centro (x, y)" => ["Center (x, y)", "Centro (x, y)", "Centre (x, y)"],
        "Clique dois pontos de medida conhecida · arraste para posicionar a imagem" => [
            "Click two points of a known distance · drag to place the image",
            "Haga clic en dos puntos de distancia conocida · arrastre para posicionar la imagen",
            "Cliquez deux points d'une distance connue · faites glisser pour placer l'image",
        ],
        "Clique e depois clique na planta para posicionar" => [
            "Click, then click on the plan to place",
            "Haga clic y luego en el plano para posicionar",
            "Cliquez, puis cliquez sur le plan pour placer",
        ],
        "Clique encadeia paredes · digite o comprimento + Enter · Shift desliga o ímã · duplo clique encerra" => {
            [
                "Clicks chain walls · type a length + Enter · Shift disables magnetism · double-click ends",
                "El clic encadena paredes · escriba la longitud + Enter · Mayús desactiva el imán · doble clic termina",
                "Les clics enchaînent les murs · tapez une longueur + Entrée · Maj désactive l'aimant · double-clic termine",
            ]
        }
        "Clique início e fim, mova para afastar e clique · duplo clique numa parede cota a parede" => {
            [
                "Click start and end, move to offset and click · double-click a wall to dimension it",
                "Haga clic en inicio y fin, mueva para separar y haga clic · doble clic en una pared la acota",
                "Cliquez début et fin, déplacez pour décaler et cliquez · double-clic sur un mur pour le coter",
            ]
        }
        "Clique onde o texto deve ficar" => [
            "Click where the text goes",
            "Haga clic donde debe quedar el texto",
            "Cliquez à l'endroit où placer le texte",
        ],
        "Clique os cantos e duplo clique fecha · duplo clique dentro de paredes detecta o cômodo" => {
            [
                "Click the corners and double-click to close · double-click inside walls detects the room",
                "Haga clic en las esquinas y doble clic para cerrar · doble clic dentro de paredes detecta la habitación",
                "Cliquez les coins et double-cliquez pour fermer · double-clic à l'intérieur des murs détecte la pièce",
            ]
        }
        "Clique os pontos e duplo clique encerra · na Elétrica/Hidráulica a linha vira eletroduto/tubulação" => {
            [
                "Click the points and double-click to end · in Electrical/Plumbing lines become conduits/pipes",
                "Haga clic en los puntos y doble clic termina · en Eléctrica/Fontanería la línea se vuelve tubo/canalización",
                "Cliquez les points et double-cliquez pour terminer · en Électricité/Plomberie la ligne devient conduit/tuyau",
            ]
        }
        "Clique para posicionar · portas e janelas encaixam na parede mais próxima · Esc cancela" => {
            [
                "Click to place · doors and windows snap into the nearest wall · Esc cancels",
                "Haga clic para posicionar · puertas y ventanas encajan en la pared más cercana · Esc cancela",
                "Cliquez pour placer · portes et fenêtres s'encastrent dans le mur le plus proche · Échap annule",
            ]
        }
        "Clique seleciona · Ctrl+clique soma · arraste move · alças editam · duplo clique modifica" => {
            [
                "Click selects · Ctrl+click adds · drag moves · handles edit · double-click modifies",
                "El clic selecciona · Ctrl+clic suma · arrastrar mueve · los tiradores editan · doble clic modifica",
                "Le clic sélectionne · Ctrl+clic ajoute · glisser déplace · les poignées modifient · double-clic édite",
            ]
        }
        "Colar" => ["Paste", "Pegar", "Coller"],
        "Comece desenhando paredes (W), importe uma planta como imagem de fundo, ou peça para a IA via MCP." => {
            [
                "Start by drawing walls (W), import a plan as background image, or ask the AI through MCP.",
                "Empiece dibujando paredes (W), importe un plano como imagen de fondo o pídalo a la IA vía MCP.",
                "Commencez par tracer des murs (W), importez un plan en image de fond, ou demandez-le à l'IA via MCP.",
            ]
        }
        "Comparar" => ["Compare", "Comparar", "Comparer"],
        "Comparar as versões" => [
            "Compare versions",
            "Comparar las versiones",
            "Comparer les versions",
        ],
        "Comparar versões" => [
            "Compare versions",
            "Comparar versiones",
            "Comparer les versions",
        ],
        "Comprimento" => ["Length", "Longitud", "Longueur"],
        "Comprimento exato da parede sendo desenhada" => [
            "Exact length of the wall being drawn",
            "Longitud exacta de la pared que se está dibujando",
            "Longueur exacte du mur en cours de tracé",
        ],
        "Contorno" => ["Outline", "Contorno", "Contour"],
        "Contínuo" => ["Solid", "Continuo", "Continu"],
        "Copiar" => ["Copy", "Copiar", "Copier"],
        "Copiar · Recortar · Colar · Duplicar" => [
            "Copy · Cut · Paste · Duplicate",
            "Copiar · Cortar · Pegar · Duplicar",
            "Copier · Couper · Coller · Dupliquer",
        ],
        "Cor" => ["Color", "Color", "Couleur"],
        "Cotar paredes selecionadas" => [
            "Dimension selected walls",
            "Acotar paredes seleccionadas",
            "Coter les murs sélectionnés",
        ],
        "Cotas automáticas (engenharia)" => [
            "Automatic dimensions (engineering)",
            "Cotas automáticas (ingeniería)",
            "Cotes automatiques (ingénierie)",
        ],
        "Criar cotas" => ["Create dimensions", "Crear cotas", "Créer des cotes"],
        "Criar cômodos" => ["Create rooms", "Crear habitaciones", "Créer des pièces"],
        "Criar foto" => ["Create photo", "Crear foto", "Créer une photo"],
        "Criar foto…" => ["Create photo…", "Crear foto…", "Créer une photo…"],
        "Criar vídeo" => ["Create video", "Crear vídeo", "Créer une vidéo"],
        "Criar vídeo…" => ["Create video…", "Crear vídeo…", "Créer une vidéo…"],
        "Pontos do caminho" => ["Path points", "Puntos del recorrido", "Points du parcours"],
        "Adicionar ponto atual" => [
            "Add current point",
            "Añadir punto actual",
            "Ajouter le point actuel",
        ],
        "Ative o visitante na vista 3D" => [
            "Turn on the visitor in the 3D view",
            "Active el visitante en la vista 3D",
            "Activez le visiteur dans la vue 3D",
        ],
        "Órbita aérea" => ["Aerial orbit", "Órbita aérea", "Orbite aérienne"],
        "Limpar" => ["Clear", "Limpiar", "Effacer"],
        "Quadros por segundo" => [
            "Frames per second",
            "Cuadros por segundo",
            "Images par seconde",
        ],
        "Velocidade" => ["Speed", "Velocidad", "Vitesse"],
        "Gerar vídeo…" => ["Render video…", "Generar vídeo…", "Générer la vidéo…"],
        "Vídeo salvo em" => [
            "Video saved to",
            "Vídeo guardado en",
            "Vidéo enregistrée dans",
        ],
        "quadros" => ["frames", "cuadros", "images"],
        "Sol pela bússola e hora" => [
            "Sun from compass and time",
            "Sol según brújula y hora",
            "Soleil selon la boussole et l'heure",
        ],
        "Vista 3D" => ["3D view", "Vista 3D", "Vue 3D"],
        "Mostrar em 3D" => ["Show in 3D", "Mostrar en 3D", "Afficher en 3D"],
        "Inclinação" => ["Tilt", "Inclinación", "Inclinaison"],
        "Deitado" => ["Lying", "Tumbado", "Couché"],
        "Em pé" => ["Standing", "De pie", "Debout"],
        "Editor no navegador" => [
            "Browser editor",
            "Editor en el navegador",
            "Éditeur dans le navigateur",
        ],
        "Disponível no aplicativo para desktop." => [
            "Available in the desktop app.",
            "Disponible en la aplicación de escritorio.",
            "Disponible dans l'application de bureau.",
        ],
        "No navegador, exporte em .glb." => [
            "In the browser, export as .glb.",
            "En el navegador, exporte en .glb.",
            "Dans le navigateur, exportez en .glb.",
        ],
        "Nenhum plugin instalado" => [
            "No plugins installed",
            "Ningún plugin instalado",
            "Aucun plugin installé",
        ],
        "Pastas" => ["Folders", "Carpetas", "Dossiers"],
        "Rodando" => ["Running", "En ejecución", "En cours"],
        "concluído" => ["done", "terminado", "terminé"],
        "Acompanha as paredes" => ["Follows the walls", "Sigue las paredes", "Suit les murs"],
        "O contorno é detectado pelas paredes e divisores e se ajusta quando eles mudam" => [
            "The outline is detected from walls and dividers and adjusts when they change",
            "El contorno se detecta por las paredes y los divisores y se ajusta cuando cambian",
            "Le contour est détecté d'après les murs et les séparations et s'ajuste quand ils changent",
        ],
        "Ambientes" => ["Rooms", "Ambientes", "Espaces"],
        "Divisor de ambiente" => ["Room divider", "Divisor de ambiente", "Séparation d'espace"],
        "Separa ambientes sem parede (ex.: sala e jantar integrados)" => [
            "Separates rooms without a wall (e.g. open living and dining)",
            "Separa ambientes sin pared (p. ej.: salón y comedor integrados)",
            "Sépare des espaces sans mur (ex. : salon et salle à manger ouverts)",
        ],
        "Escala vertical" => ["Vertical scale", "Escala vertical", "Échelle verticale"],
        "Diferente" => ["Different", "Diferente", "Différent"],
        "Criar paredes" => ["Create walls", "Crear paredes", "Créer des murs"],
        "Curva suave" => ["Smooth curve", "Curva suave", "Courbe douce"],
        "Cômodos" => ["Rooms", "Habitaciones", "Pièces"],
        "Descartar" => ["Discard", "Descartar", "Abandonner"],
        "Descrição" => ["Description", "Descripción", "Description"],
        "Desenhar linhas" => ["Draw lines", "Dibujar líneas", "Tracer des lignes"],
        "Desfazer" => ["Undo", "Deshacer", "Annuler"],
        "Desfazer (Ctrl+Z)" => ["Undo (Ctrl+Z)", "Deshacer (Ctrl+Z)", "Annuler (Ctrl+Z)"],
        "Desfazer · Refazer (inclusive o que a IA fez)" => [
            "Undo · Redo (including what the AI did)",
            "Deshacer · Rehacer (incluso lo que hizo la IA)",
            "Annuler · Rétablir (y compris ce qu'a fait l'IA)",
        ],
        "Desliga o ímã (ângulos de 15°, pontos e grade)" => [
            "Disables magnetism (15° angles, points and grid)",
            "Desactiva el imán (ángulos de 15°, puntos y cuadrícula)",
            "Désactive l'aimant (angles de 15°, points et grille)",
        ],
        "Detalhes: marca, modelo e link" => [
            "Details: brand, model and link",
            "Detalles: marca, modelo y enlace",
            "Détails : marque, modèle et lien",
        ],
        "Digitar número + Enter" => [
            "Type number + Enter",
            "Escribir número + Enter",
            "Taper un nombre + Entrée",
        ],
        "Direita" => ["Right", "Derecha", "Droite"],
        "Disco" => ["Disc", "Disco", "Disque"],
        "Distância real" => ["Real distance", "Distancia real", "Distance réelle"],
        "Dividir parede ao meio" => [
            "Split wall in half",
            "Dividir la pared por la mitad",
            "Couper le mur en deux",
        ],
        "Diâmetro" => ["Diameter", "Diámetro", "Diamètre"],
        "Dobradiça à direita" => [
            "Hinge on the right",
            "Bisagra a la derecha",
            "Charnière à droite",
        ],
        "Duplicar" => ["Duplicate", "Duplicar", "Dupliquer"],
        "Duplicar versão atual" => [
            "Duplicate current version",
            "Duplicar versión actual",
            "Dupliquer la version actuelle",
        ],
        "Duplicar versão · Próxima versão (guias)" => [
            "Duplicate version · Next version (tabs)",
            "Duplicar versión · Siguiente versión (pestañas)",
            "Dupliquer la version · Version suivante (onglets)",
        ],
        "Duplo clique" => ["Double-click", "Doble clic", "Double-clic"],
        "Editar" => ["Edit", "Editar", "Modifier"],
        "Editar andar…" => ["Edit storey…", "Editar planta…", "Modifier l'étage…"],
        "Elevação" => ["Elevation", "Elevación", "Élévation"],
        "Elevação do piso" => ["Floor elevation", "Elevación del suelo", "Niveau du sol"],
        "Elétrica" => ["Electrical", "Eléctrica", "Électricité"],
        "Elétrica (tomadas, luz, cabos)" => [
            "Electrical (outlets, lights, cables)",
            "Eléctrica (enchufes, luz, cables)",
            "Électricité (prises, éclairage, câbles)",
        ],
        "Encerra paredes · fecha/detecta cômodo · cota parede · modifica" => [
            "Ends walls · closes/detects room · dimensions wall · modifies",
            "Termina paredes · cierra/detecta habitación · acota pared · modifica",
            "Termine les murs · ferme/détecte la pièce · cote le mur · modifie",
        ],
        "Enquadrar (Ctrl+0)" => ["Frame (Ctrl+0)", "Encuadrar (Ctrl+0)", "Cadrer (Ctrl+0)"],
        "Enquadrar 3D" => ["Frame 3D", "Encuadrar 3D", "Cadrer la 3D"],
        "Enquadrar planta" => ["Frame plan", "Encuadrar plano", "Cadrer le plan"],
        "Escala" => ["Scale", "Escala", "Échelle"],
        "Escala calibrada. Arraste para posicionar a imagem ou troque de ferramenta." => [
            "Scale calibrated. Drag to place the image or switch tools.",
            "Escala calibrada. Arrastre para posicionar la imagen o cambie de herramienta.",
            "Échelle calibrée. Faites glisser pour placer l'image ou changez d'outil.",
        ],
        "Espelhado" => ["Mirrored", "Reflejado", "Miroir"],
        "Espessura" => ["Thickness", "Espesor", "Épaisseur"],
        "Espessura da laje" => [
            "Slab thickness",
            "Espesor de la losa",
            "Épaisseur de la dalle",
        ],
        "Esquerda" => ["Left", "Izquierda", "Gauche"],
        "Esta versão e o histórico dela serão removidos do projeto." => [
            "This version and its history will be removed from the project.",
            "Esta versión y su historial se quitarán del proyecto.",
            "Cette version et son historique seront retirés du projet.",
        ],
        "Estilo" => ["Style", "Estilo", "Style"],
        "Excluir" => ["Delete", "Eliminar", "Supprimer"],
        "Excluir andar" => ["Delete storey", "Eliminar planta", "Supprimer l'étage"],
        "Exibir" => ["Show", "Ver", "Afficher"],
        "Exportar 3D" => ["Export 3D", "Exportar 3D", "Exporter la 3D"],
        "Exportar planta" => ["Export plan", "Exportar plano", "Exporter le plan"],
        "Fechada" => ["Closed", "Cerrada", "Fermée"],
        "Fechar" => ["Close", "Cerrar", "Fermer"],
        "Fechar versão" => ["Close version", "Cerrar versión", "Fermer la version"],
        "Fim" => ["End", "Fin", "Fin"],
        "Fim (x, y)" => ["End (x, y)", "Fin (x, y)", "Fin (x, y)"],
        "Grupo" => ["Group", "Grupo", "Groupe"],
        "Hidráulica" => ["Plumbing", "Fontanería", "Plomberie"],
        "Hora do dia" => ["Time of day", "Hora del día", "Heure de la journée"],
        "Imagem" => ["Image", "Imagen", "Image"],
        "Imagem de fundo" => ["Background image", "Imagen de fondo", "Image de fond"],
        "Imagem importada. Clique em dois pontos de medida conhecida para calibrar; arraste para posicionar." => {
            [
                "Image imported. Click two points of a known distance to calibrate; drag to place.",
                "Imagen importada. Haga clic en dos puntos de distancia conocida para calibrar; arrastre para posicionar.",
                "Image importée. Cliquez deux points d'une distance connue pour calibrer ; faites glisser pour placer.",
            ]
        }
        "Imagem…" => ["Image…", "Imagen…", "Image…"],
        "Imagens" => ["Images", "Imágenes", "Images"],
        "Importar imagem de fundo" => [
            "Import background image",
            "Importar imagen de fondo",
            "Importer une image de fond",
        ],
        "Importar modelo 3D…" => [
            "Import 3D model…",
            "Importar modelo 3D…",
            "Importer un modèle 3D…",
        ],
        "Importar…" => ["Import…", "Importar…", "Importer…"],
        "Informações" => ["Information", "Información", "Informations"],
        "Início" => ["Start", "Inicio", "Début"],
        "Início (x, y)" => ["Start (x, y)", "Inicio (x, y)", "Début (x, y)"],
        "Itálico" => ["Italic", "Cursiva", "Italique"],
        "Lado direito" => ["Right side", "Lado derecho", "Côté droit"],
        "Lado esquerdo" => ["Left side", "Lado izquierdo", "Côté gauche"],
        "Largura" => ["Width", "Anchura", "Largeur"],
        "Legenda de símbolos (elétrica e hidráulica)" => [
            "Symbol legend (electrical and plumbing)",
            "Leyenda de símbolos (eléctrica y fontanería)",
            "Légende des symboles (électricité et plomberie)",
        ],
        "Licença" => ["License", "Licencia", "Licence"],
        "Linhas (tubulação / eletroduto)" => [
            "Lines (pipes / conduits)",
            "Líneas (tubería / tubo eléctrico)",
            "Lignes (tuyaux / conduits)",
        ],
        "Link" => ["Link", "Enlace", "Lien"],
        "MCP desligado" => ["MCP off", "MCP apagado", "MCP désactivé"],
        "Manter proporções" => [
            "Keep proportions",
            "Mantener proporciones",
            "Conserver les proportions",
        ],
        "Marca" => ["Brand", "Marca", "Marque"],
        "Me paga um café" => ["Buy me a coffee", "Invítame un café", "Offre-moi un café"],
        "Medida" => ["Measure", "Medida", "Mesure"],
        "Modelo" => ["Model", "Modelo", "Modèle"],
        "Modelo importado. Ajuste medidas com Enter ou pelas alças." => [
            "Model imported. Adjust sizes with Enter or the handles.",
            "Modelo importado. Ajuste las medidas con Enter o con los tiradores.",
            "Modèle importé. Ajustez les dimensions avec Entrée ou les poignées.",
        ],
        "Modelos 3D" => ["3D models", "Modelos 3D", "Modèles 3D"],
        "Modificar andar" => ["Modify storey", "Modificar planta", "Modifier l'étage"],
        "Modificar cota" => ["Modify dimension", "Modificar cota", "Modifier la cote"],
        "Modificar cômodo" => ["Modify room", "Modificar habitación", "Modifier la pièce"],
        "Modificar linha" => ["Modify line", "Modificar línea", "Modifier la ligne"],
        "Modificar parede" => ["Modify wall", "Modificar pared", "Modifier le mur"],
        "Modificar texto" => ["Modify text", "Modificar texto", "Modifier le texte"],
        "Modificar · Excluir · Cancelar" => [
            "Modify · Delete · Cancel",
            "Modificar · Eliminar · Cancelar",
            "Modifier · Supprimer · Annuler",
        ],
        "Modificar…" => ["Modify…", "Modificar…", "Modifier…"],
        "Mostrar bússola" => ["Show compass", "Mostrar brújula", "Afficher la boussole"],
        "Move a seleção 1 cm (10 cm)" => [
            "Moves the selection 1 cm (10 cm)",
            "Mueve la selección 1 cm (10 cm)",
            "Déplace la sélection de 1 cm (10 cm)",
        ],
        "Mover vista" => ["Pan view", "Mover vista", "Déplacer la vue"],
        "Máxima" => ["Best", "Máxima", "Maximale"],
        "Móveis" => ["Furniture", "Muebles", "Mobilier"],
        "Móveis na planta" => [
            "Furniture on the plan",
            "Muebles en el plano",
            "Mobilier sur le plan",
        ],
        "Nada encontrado." => ["Nothing found.", "No se encontró nada.", "Aucun résultat."],
        "Negrito" => ["Bold", "Negrita", "Gras"],
        "Nenhum espaço fechado por paredes aqui" => [
            "No space enclosed by walls here",
            "Ningún espacio cerrado por paredes aquí",
            "Aucun espace fermé par des murs ici",
        ],
        "Nenhum ponto" => ["No points", "Ningún punto", "Aucun point"],
        "Nenhum ponto de vista salvo" => [
            "No saved points of view",
            "Ningún punto de vista guardado",
            "Aucun point de vue enregistré",
        ],
        "Nome" => ["Name", "Nombre", "Nom"],
        "Nome do projeto" => ["Project name", "Nombre del proyecto", "Nom du projet"],
        "Norte" => ["North", "Norte", "Nord"],
        "Nova versão da planta" => [
            "New plan version",
            "Nueva versión del plano",
            "Nouvelle version du plan",
        ],
        "Nova versão em branco" => [
            "New blank version",
            "Nueva versión en blanco",
            "Nouvelle version vierge",
        ],
        "Novo" => ["New", "Nuevo", "Nouveau"],
        "Novo (Ctrl+N)" => ["New (Ctrl+N)", "Nuevo (Ctrl+N)", "Nouveau (Ctrl+N)"],
        "Níveis na mesma elevação funcionam como layouts alternativos" => [
            "Levels at the same elevation work as alternative layouts",
            "Los niveles a la misma elevación funcionan como diseños alternativos",
            "Des niveaux à la même élévation servent de variantes d'aménagement",
        ],
        "O andar e tudo o que está nele serão removidos (dá para desfazer)." => [
            "The storey and everything on it will be removed (undoable).",
            "La planta y todo lo que hay en ella se quitarán (se puede deshacer).",
            "L'étage et tout ce qu'il contient seront supprimés (annulable).",
        ],
        "O projeto tem alterações que ainda não foram salvas." => [
            "The project has unsaved changes.",
            "El proyecto tiene cambios sin guardar.",
            "Le projet a des modifications non enregistrées.",
        ],
        "OBJ + MTL…" => ["OBJ + MTL…", "OBJ + MTL…", "OBJ + MTL…"],
        "OK" => ["OK", "OK", "OK"],
        "Opacidade" => ["Opacity", "Opacidad", "Opacité"],
        "Ordem (mesma elevação)" => [
            "Order (same elevation)",
            "Orden (misma elevación)",
            "Ordre (même élévation)",
        ],
        "PDF (A3, ajustado à folha)…" => [
            "PDF (A3, fit to sheet)…",
            "PDF (A3, ajustado a la hoja)…",
            "PDF (A3, ajusté à la feuille)…",
        ],
        "PDF 1:100…" => ["PDF 1:100…", "PDF 1:100…", "PDF 1:100…"],
        "PDF 1:50…" => ["PDF 1:50…", "PDF 1:50…", "PDF 1:50…"],
        "PNG…" => ["PNG…", "PNG…", "PNG…"],
        "Paredes" => ["Walls", "Paredes", "Murs"],
        "Inteiras" => ["Up", "Enteras", "Debout"],
        "Recortadas" => ["Cutaway", "Recortadas", "Coupés"],
        "Baixas" => ["Down", "Bajas", "Baissés"],
        "Todas as paredes em pé" => [
            "Every wall standing",
            "Todas las paredes en pie",
            "Tous les murs debout",
        ],
        "As paredes da frente descem para mostrar os cômodos" => [
            "The walls in front come down to show the rooms",
            "Las paredes de delante bajan para mostrar las habitaciones",
            "Les murs de devant s'abaissent pour montrer les pièces",
        ],
        "Todas as paredes descem" => [
            "Every wall comes down",
            "Todas las paredes bajan",
            "Tous les murs s'abaissent",
        ],
        "Personalizada" => ["Custom", "Personalizada", "Personnalisée"],
        "Peça" => ["Tile", "Azulejo", "Carrelage"],
        "Pintura" => ["Paint", "Pintura", "Peinture"],
        "Piso" => ["Floor", "Suelo", "Sol"],
        "Planta" => ["Plan", "Plano", "Plan"],
        "Pontilhado" => ["Dotted", "Punteado", "Pointillé"],
        "Posição (x, y)" => ["Position (x, y)", "Posición (x, y)", "Position (x, y)"],
        "Potência da luz" => [
            "Light power",
            "Potencia de la luz",
            "Puissance de la lumière",
        ],
        "Preço" => ["Price", "Precio", "Prix"],
        "Problemas" => ["Issues", "Problemas", "Problèmes"],
        "Profundidade" => ["Depth", "Profundidad", "Profondeur"],
        "Projeto em edição: novos símbolos e linhas vão para ele" => [
            "Project being edited: new symbols and lines go there",
            "Proyecto en edición: los nuevos símbolos y líneas van a él",
            "Projet en cours d'édition : les nouveaux symboles et lignes y vont",
        ],
        "Projetos (3D New Era AI, Sweet Home 3D)" => [
            "Projects (3D New Era AI, Sweet Home 3D)",
            "Proyectos (3D New Era AI, Sweet Home 3D)",
            "Projets (3D New Era AI, Sweet Home 3D)",
        ],
        "Pé-direito" => ["Ceiling height", "Altura libre", "Hauteur sous plafond"],
        "Qualidade" => ["Quality", "Calidad", "Qualité"],
        "Quantitativos" => ["Quantities", "Mediciones", "Quantitatifs"],
        "Rascunho" => ["Draft", "Borrador", "Brouillon"],
        "Recentes" => ["Recent", "Recientes", "Récents"],
        "Recortar" => ["Cut", "Cortar", "Couper"],
        "Refazer" => ["Redo", "Rehacer", "Rétablir"],
        "Refazer (Ctrl+Shift+Z)" => [
            "Redo (Ctrl+Shift+Z)",
            "Rehacer (Ctrl+Shift+Z)",
            "Rétablir (Ctrl+Maj+Z)",
        ],
        "Referências dos cômodos" => [
            "Room references",
            "Referencias de las habitaciones",
            "Références des pièces",
        ],
        "Remover imagem" => ["Remove image", "Quitar imagen", "Retirer l'image"],
        "Renderizar" => ["Render", "Renderizar", "Rendu"],
        "Renomear" => ["Rename", "Renombrar", "Renommer"],
        "Rotação" => ["Rotation", "Rotación", "Rotation"],
        "SVG em escala real…" => [
            "SVG at true scale…",
            "SVG a escala real…",
            "SVG à l'échelle réelle…",
        ],
        "Sair" => ["Quit", "Salir", "Quitter"],
        "Salvar" => ["Save", "Guardar", "Enregistrer"],
        "Salvar (Ctrl+S)" => ["Save (Ctrl+S)", "Guardar (Ctrl+S)", "Enregistrer (Ctrl+S)"],
        "Salvar PNG…" => ["Save PNG…", "Guardar PNG…", "Enregistrer en PNG…"],
        "Salvar alterações?" => [
            "Save changes?",
            "¿Guardar los cambios?",
            "Enregistrer les modifications ?",
        ],
        "Salvar como…" => ["Save as…", "Guardar como…", "Enregistrer sous…"],
        "Salvar ponto de vista" => [
            "Save point of view",
            "Guardar punto de vista",
            "Enregistrer le point de vue",
        ],
        "Scroll · botão do meio · F" => [
            "Scroll · middle button · F",
            "Rueda · botón central · F",
            "Molette · bouton du milieu · F",
        ],
        "Selecionar" => ["Select", "Seleccionar", "Sélectionner"],
        "Selecionar tudo" => ["Select all", "Seleccionar todo", "Tout sélectionner"],
        "Selecionar · Mover vista · Paredes · Cômodos · Cotas · Texto" => [
            "Select · Pan · Walls · Rooms · Dimensions · Text",
            "Seleccionar · Mover vista · Paredes · Habitaciones · Cotas · Texto",
            "Sélectionner · Déplacer · Murs · Pièces · Cotes · Texte",
        ],
        "Sem acabamento" => ["No finish", "Sin acabado", "Sans finition"],
        "Sem nome" => ["Unnamed", "Sin nombre", "Sans nom"],
        "Sem seta" => ["No arrow", "Sin flecha", "Sans flèche"],
        "Seta aberta" => ["Open arrow", "Flecha abierta", "Flèche ouverte"],
        "Seta cheia" => ["Filled arrow", "Flecha rellena", "Flèche pleine"],
        "Setas (+Shift)" => ["Arrows (+Shift)", "Flechas (+Mayús)", "Flèches (+Maj)"],
        "Shift (segurado)" => ["Shift (held)", "Mayús (mantenido)", "Maj (maintenue)"],
        "Sweet Home 3D" => ["Sweet Home 3D", "Sweet Home 3D", "Sweet Home 3D"],
        "Símbolos arquitetônicos" => [
            "Architectural symbols",
            "Símbolos arquitectónicos",
            "Symboles architecturaux",
        ],
        "Tamanho" => ["Size", "Tamaño", "Taille"],
        "Teto" => ["Ceiling", "Techo", "Plafond"],
        "Texto" => ["Text", "Texto", "Texte"],
        "Tipo" => ["Type", "Tipo", "Type"],
        "Tracejado" => ["Dashed", "Discontinuo", "Tirets"],
        "Traço" => ["Dash", "Raya", "Tiret"],
        "Traço e dois pontos" => ["Dash dot dot", "Raya y dos puntos", "Tiret point point"],
        "Traço e ponto" => ["Dash dot", "Raya y punto", "Tiret point"],
        "Térreo" => ["Ground floor", "Planta baja", "Rez-de-chaussée"],
        "Unidade" => ["Unit", "Unidad", "Unité"],
        "Unir paredes selecionadas" => [
            "Join selected walls",
            "Unir paredes seleccionadas",
            "Joindre les murs sélectionnés",
        ],
        "Usa o ponto de vista atual da vista 3D (aérea ou visitante)." => [
            "Uses the current 3D point of view (aerial or visitor).",
            "Usa el punto de vista actual de la vista 3D (aérea o visitante).",
            "Utilise le point de vue actuel de la vue 3D (aérienne ou visiteur).",
        ],
        "Ver" => ["View", "Ver", "Affichage"],
        "Versão" => ["Version", "Versión", "Version"],
        "Visitante" => ["Visitor", "Visitante", "Visiteur"],
        "Visitante — arraste: olhar · W/A/S/D ou setas: andar · Scroll: avançar · Esc: visão aérea" => {
            [
                "Visitor — drag: look · W/A/S/D or arrows: walk · Scroll: forward · Esc: aerial view",
                "Visitante — arrastrar: mirar · W/A/S/D o flechas: andar · Rueda: avanzar · Esc: vista aérea",
                "Visiteur — glisser : regarder · W/A/S/D ou flèches : marcher · Molette : avancer · Échap : vue aérienne",
            ]
        }
        "Vista de cima de todos" => [
            "Top views for all",
            "Vista superior de todos",
            "Vues de dessus pour tout",
        ],
        "Visualização 3D requer o backend wgpu." => [
            "The 3D view needs the wgpu backend.",
            "La vista 3D necesita el backend wgpu.",
            "La vue 3D nécessite le backend wgpu.",
        ],
        "Visão aérea" => ["Aerial view", "Vista aérea", "Vue aérienne"],
        "Frente" => ["Front", "Frente", "Face"],
        "Trás" => ["Back", "Detrás", "Arrière"],
        "Topo" => ["Top", "Superior", "Dessus"],
        "Visível" => ["Visible", "Visible", "Visible"],
        "Visível no 3D" => ["Visible in 3D", "Visible en 3D", "Visible en 3D"],
        "Zoom · mover vista · enquadrar" => [
            "Zoom · pan · frame",
            "Zoom · mover vista · encuadrar",
            "Zoom · déplacer · cadrer",
        ],
        "glTF binário (.glb)…" => [
            "Binary glTF (.glb)…",
            "glTF binario (.glb)…",
            "glTF binaire (.glb)…",
        ],
        "peças" => ["pieces", "piezas", "pièces"],
        "Área" => ["Area", "Área", "Surface"],
        "Área na planta" => [
            "Area on the plan",
            "Área en el plano",
            "Surface sur le plan",
        ],
        "Ângulo" => ["Angle", "Ángulo", "Angle"],
        "Cotas" => ["Dimensions", "Cotas", "Cotes"],
        "Textos" => ["Texts", "Textos", "Textes"],
        "Linhas" => ["Lines", "Líneas", "Lignes"],
        "paredes" => ["walls", "paredes", "murs"],
        "cômodos" => ["rooms", "habitaciones", "pièces"],
        "móveis" => ["pieces", "muebles", "meubles"],
        "grupo de" => ["group of", "grupo de", "groupe de"],
        "Sala de estar" => ["Living room", "Salón", "Salon"],
        "Sala de jantar" => ["Dining room", "Comedor", "Salle à manger"],
        "Cozinha" => ["Kitchen", "Cocina", "Cuisine"],
        "Quarto" => ["Bedroom", "Dormitorio", "Chambre"],
        "Banheiro" => ["Bathroom", "Baño", "Salle de bain"],
        "Lavanderia" => ["Laundry", "Lavadero", "Buanderie"],
        "Escritório" => ["Office", "Despacho", "Bureau"],
        "Portas e janelas" => [
            "Doors and windows",
            "Puertas y ventanas",
            "Portes et fenêtres",
        ],
        "Estrutura" => ["Structure", "Estructura", "Structure"],
        "Decoração" => ["Decor", "Decoración", "Décoration"],
        "Área externa" => ["Outdoor", "Zona exterior", "Extérieur"],
        "atual" => ["current", "actual", "actuel"],
        "Importado de {} — salve como projeto para manter tudo num arquivo" => [
            "Imported from {} — save it as a project to keep everything in one file",
            "Importado de {} — guárdelo como proyecto para tenerlo todo en un archivo",
            "Importé de {} — enregistrez-le en projet pour tout garder dans un seul fichier",
        ],
        "Aberto: {}" => ["Opened: {}", "Abierto: {}", "Ouvert : {}"],
        " · {} aviso(s)" => [
            " · {} warning(s)",
            " · {} aviso(s)",
            " · {} avertissement(s)",
        ],
        "⚠ Não foi possível abrir {}: {}" => [
            "⚠ Could not open {}: {}",
            "⚠ No se pudo abrir {}: {}",
            "⚠ Impossible d'ouvrir {} : {}",
        ],
        "Pontos de vista ({})" => [
            "Points of view ({})",
            "Puntos de vista ({})",
            "Points de vue ({})",
        ],
        "Ponto de vista {}" => ["Point of view {}", "Punto de vista {}", "Point de vue {}"],
        "Salvo em {}" => ["Saved to {}", "Guardado en {}", "Enregistré dans {}"],
        "⚠ Não foi possível salvar: {}" => [
            "⚠ Could not save: {}",
            "⚠ No se pudo guardar: {}",
            "⚠ Impossible d'enregistrer : {}",
        ],
        "Modelo 3D exportado para {}" => [
            "3D model exported to {}",
            "Modelo 3D exportado a {}",
            "Modèle 3D exporté vers {}",
        ],
        "⚠ Falha ao exportar: {}" => [
            "⚠ Export failed: {}",
            "⚠ Fallo al exportar: {}",
            "⚠ Échec de l'export : {}",
        ],
        "Planta exportada para {}" => [
            "Plan exported to {}",
            "Plano exportado a {}",
            "Plan exporté vers {}",
        ],
        "⚠ Imagem inválida: {}" => [
            "⚠ Invalid image: {}",
            "⚠ Imagen no válida: {}",
            "⚠ Image non valide : {}",
        ],
        "Modificar {} paredes" => [
            "Modify {} walls",
            "Modificar {} paredes",
            "Modifier {} murs",
        ],
        "Modificar {}" => ["Modify {}", "Modificar {}", "Modifier {}"],
        "{} peças" => ["{} pieces", "{} piezas", "{} pièces"],
        "Os pontos marcados estão a {} na escala atual." => [
            "The marked points are {} apart at the current scale.",
            "Los puntos marcados están a {} en la escala actual.",
            "Les points marqués sont distants de {} à l'échelle actuelle.",
        ],
        "Fechar “{}”?" => ["Close “{}”?", "¿Cerrar «{}»?", "Fermer « {} » ?"],
        "Excluir “{}”?" => ["Delete “{}”?", "¿Eliminar «{}»?", "Supprimer « {} » ?"],
        "Renderizando… {} s" => ["Rendering… {} s", "Renderizando… {} s", "Rendu… {} s"],
        "Pronta em {} s" => ["Ready in {} s", "Lista en {} s", "Prête en {} s"],
        "Foto salva em {}" => [
            "Photo saved to {}",
            "Foto guardada en {}",
            "Photo enregistrée dans {}",
        ],
        "Mostrar {}" => ["Show {}", "Mostrar {}", "Afficher {}"],
        "Tema" => ["Theme", "Tema", "Thème"],
        "Do sistema" => ["From the system", "Del sistema", "Du système"],
        "Noite" => ["Night", "Noche", "Nuit"],
        "Dia" => ["Day", "Día", "Jour"],
        "luz" => ["light", "luz", "lumière"],
        "Arraste ou dois dedos: girar · Shift: mover · Pinça ou scroll: zoom · F: enquadrar" => {
            [
                "Drag or two fingers: orbit · Shift: pan · Pinch or scroll: zoom · F: frame",
                "Arrastrar o dos dedos: girar · Mayús: mover · Pellizco o rueda: zoom · F: encuadrar",
                "Glisser ou deux doigts : pivoter · Maj : déplacer · Pincer ou molette : zoom · F : cadrer",
            ]
        }
        "Aparência" => ["Look", "Aspecto", "Apparence"],
        // Every piece of the catalog: what the window calls it, and what it
        // is named when someone drops it on the plan.
        "Sofá 3 lugares" => ["3-seat sofa", "Sofá de 3 plazas", "Canapé 3 places"],
        "Sofá 2 lugares" => ["2-seat sofa", "Sofá de 2 plazas", "Canapé 2 places"],
        "Sofá em L (chaise)" => [
            "L-shaped sofa (chaise)",
            "Sofá en L (chaise longue)",
            "Canapé d'angle (méridienne)",
        ],
        "Poltrona" => ["Armchair", "Sillón", "Fauteuil"],
        "Poltrona Win" => ["Win armchair", "Sillón Win", "Fauteuil Win"],
        "Cadeira Ares" => ["Ares chair", "Silla Ares", "Chaise Ares"],
        "Mesa de centro" => ["Coffee table", "Mesa de centro", "Table basse"],
        "Mesa lateral" => ["Side table", "Mesa auxiliar", "Table d'appoint"],
        "Rack de TV" => ["TV unit", "Mueble de TV", "Meuble TV"],
        "Televisão 55\"" => ["55\" television", "Televisor de 55\"", "Téléviseur 55\""],
        "Estante" => ["Shelving unit", "Estantería", "Étagère"],
        "Tapete" => ["Rug", "Alfombra", "Tapis"],
        "Radiador de painel 60×10×50" => [
            "Panel radiator 60×10×50",
            "Radiador de panel 60×10×50",
            "Radiateur à panneau 60×10×50",
        ],
        "Radiador de painel 100×10×50" => [
            "Panel radiator 100×10×50",
            "Radiador de panel 100×10×50",
            "Radiateur à panneau 100×10×50",
        ],
        "Radiador de painel 120×10×50" => [
            "Panel radiator 120×10×50",
            "Radiador de panel 120×10×50",
            "Radiateur à panneau 120×10×50",
        ],
        "Convector elétrico 100×10×45" => [
            "Electric convector 100×10×45",
            "Convector eléctrico 100×10×45",
            "Convecteur électrique 100×10×45",
        ],
        "Luminária de piso" => ["Floor lamp", "Lámpara de pie", "Lampadaire"],
        "Abajur de mesa" => ["Table lamp", "Lámpara de mesa", "Lampe de table"],
        "Spot embutido LED 7 W" => [
            "Recessed LED spot 7 W",
            "Foco LED empotrado 7 W",
            "Spot LED encastré 7 W",
        ],
        "Pendente" => ["Pendant light", "Lámpara colgante", "Suspension"],
        "Painel LED 60×60 36 W" => [
            "LED panel 60×60 36 W",
            "Panel LED 60×60 36 W",
            "Dalle LED 60×60 36 W",
        ],
        "Fita LED (perfil)" => [
            "LED strip (profile)",
            "Tira LED (perfil)",
            "Ruban LED (profilé)",
        ],
        "Mesa de jantar 4 lugares" => [
            "Dining table for 4",
            "Mesa de comedor de 4",
            "Table à manger 4 places",
        ],
        "Mesa de jantar 6 lugares" => [
            "Dining table for 6",
            "Mesa de comedor de 6",
            "Table à manger 6 places",
        ],
        "Mesa com 4 cadeiras" => [
            "Table with 4 chairs",
            "Mesa con 4 sillas",
            "Table avec 4 chaises",
        ],
        "Mesa com 6 cadeiras" => [
            "Table with 6 chairs",
            "Mesa con 6 sillas",
            "Table avec 6 chaises",
        ],
        "Mesa redonda" => ["Round table", "Mesa redonda", "Table ronde"],
        "Cadeira" => ["Chair", "Silla", "Chaise"],
        "Aparador" => ["Sideboard", "Aparador", "Buffet"],
        "Geladeira" => ["Fridge", "Frigorífico", "Réfrigérateur"],
        "Fogão 4 bocas" => ["4-burner range", "Cocina de 4 fuegos", "Cuisinière 4 feux"],
        "Bancada com pia" => [
            "Countertop with sink",
            "Encimera con fregadero",
            "Plan de travail avec évier",
        ],
        "Armário de cozinha (baixo)" => [
            "Base cabinet",
            "Mueble bajo de cocina",
            "Meuble bas de cuisine",
        ],
        "Armário aéreo" => ["Wall cabinet", "Mueble alto", "Meuble haut"],
        "Coifa" => ["Range hood", "Campana extractora", "Hotte"],
        "Lava-louças" => ["Dishwasher", "Lavavajillas", "Lave-vaisselle"],
        "Micro-ondas" => ["Microwave", "Microondas", "Micro-ondes"],
        "Cooktop 4 bocas (embutir)" => [
            "4-burner cooktop (built-in)",
            "Placa de 4 fuegos (encastrar)",
            "Table de cuisson 4 feux (encastrable)",
        ],
        "Cuba de inox (embutir)" => [
            "Stainless steel sink (built-in)",
            "Fregadero de acero inoxidable (encastrar)",
            "Évier inox (encastrable)",
        ],
        "Forno de embutir" => ["Built-in oven", "Horno empotrable", "Four encastrable"],
        "Ilha de cozinha" => ["Kitchen island", "Isla de cocina", "Îlot de cuisine"],
        "Banqueta alta" => ["Bar stool", "Taburete alto", "Tabouret de bar"],
        "Cama de casal" => ["Double bed", "Cama de matrimonio", "Lit double"],
        "Cama queen" => ["Queen bed", "Cama queen", "Lit queen"],
        "Cama king" => ["King bed", "Cama king", "Lit king"],
        "Cama de solteiro" => ["Single bed", "Cama individual", "Lit simple"],
        "Criado-mudo" => ["Bedside table", "Mesita de noche", "Table de chevet"],
        "Guarda-roupa" => ["Wardrobe", "Armario ropero", "Armoire"],
        "Cômoda" => ["Chest of drawers", "Cómoda", "Commode"],
        "Berço" => ["Cot", "Cuna", "Lit bébé"],
        "Vaso sanitário" => ["Toilet", "Inodoro", "WC"],
        "Gabinete com lavatório" => [
            "Vanity unit with basin",
            "Mueble con lavabo",
            "Meuble avec vasque",
        ],
        "Box de chuveiro" => ["Shower tray", "Plato de ducha", "Receveur de douche"],
        "Box de vidro" => [
            "Glass shower screen",
            "Mampara de vidrio",
            "Paroi de douche en verre",
        ],
        "Banheira" => ["Bathtub", "Bañera", "Baignoire"],
        "Máquina de lavar" => ["Washing machine", "Lavadora", "Lave-linge"],
        "Secadora" => ["Tumble dryer", "Secadora", "Sèche-linge"],
        "Tanque" => ["Laundry sink", "Lavadero", "Bac à laver"],
        "Escrivaninha" => ["Desk", "Escritorio", "Bureau"],
        "Cadeira de escritório" => ["Office chair", "Silla de oficina", "Chaise de bureau"],
        "Porta" => ["Door", "Puerta", "Porte"],
        "Porta dupla" => ["Double door", "Puerta doble", "Porte double"],
        "Porta de correr" => ["Sliding door", "Puerta corredera", "Porte coulissante"],
        "Portão de garagem" => ["Garage door", "Puerta de garaje", "Porte de garage"],
        "Vão livre" => ["Opening", "Vano libre", "Ouverture libre"],
        "Janela" => ["Window", "Ventana", "Fenêtre"],
        "Janela basculante" => ["Awning window", "Ventana abatible", "Fenêtre à soufflet"],
        "Porta-janela" => ["French door", "Puerta ventana", "Porte-fenêtre"],
        "Escada reta" => ["Straight stair", "Escalera recta", "Escalier droit"],
        "Pilar" => ["Column", "Pilar", "Poteau"],
        "Coluna redonda" => ["Round column", "Columna redonda", "Colonne ronde"],
        "Sapata" => ["Footing", "Zapata", "Semelle"],
        "Viga / caibro" => ["Beam / rafter", "Viga / cabio", "Poutre / chevron"],
        "Painel" => ["Panel", "Panel", "Panneau"],
        "Telha ondulada" => ["Corrugated sheet", "Chapa ondulada", "Tôle ondulée"],
        "Gradil de ferro (guarda-corpo de barras verticais)" => [
            "Iron railing (vertical bar guard)",
            "Barandilla de hierro (barrotes verticales)",
            "Garde-corps en fer (barreaux verticaux)",
        ],
        "Guarda-corpo de vidro laminado com corrimão metálico" => [
            "Laminated glass guard with a metal handrail",
            "Barandilla de vidrio laminado con pasamanos metálico",
            "Garde-corps en verre feuilleté avec main courante métallique",
        ],
        "Fechamento de vidro da sacada (envidraçamento retrátil)" => [
            "Balcony glazing (retractable)",
            "Cerramiento de vidrio del balcón (acristalamiento retráctil)",
            "Fermeture vitrée du balcon (vitrage rétractable)",
        ],
        "Caixa" => ["Box", "Caja", "Caisse"],
        "Planta em vaso" => ["Potted plant", "Planta en maceta", "Plante en pot"],
        "Floreira" => ["Planter", "Jardinera", "Jardinière"],
        "Muro / cerca" => ["Wall / fence", "Muro / valla", "Mur / clôture"],
        "Piscina" => ["Swimming pool", "Piscina", "Piscine"],
        "Piscina oval" => ["Oval pool", "Piscina ovalada", "Piscine ovale"],
        "Espreguiçadeira" => ["Sun lounger", "Tumbona", "Transat"],
        "Churrasqueira" => ["Barbecue", "Barbacoa", "Barbecue"],
        "Banco" => ["Bench", "Banco", "Banc"],
        "Árvore" => ["Tree", "Árbol", "Arbre"],
        "Carro" => ["Car", "Coche", "Voiture"],
        "Tomada baixa (30 cm)" => [
            "Low socket (30 cm)",
            "Enchufe bajo (30 cm)",
            "Prise basse (30 cm)",
        ],
        "Tomada média (1,10 m)" => [
            "Mid socket (1.10 m)",
            "Enchufe medio (1,10 m)",
            "Prise à mi-hauteur (1,10 m)",
        ],
        "Tomada alta (2,20 m)" => [
            "High socket (2.20 m)",
            "Enchufe alto (2,20 m)",
            "Prise haute (2,20 m)",
        ],
        "Interruptor simples" => ["Single switch", "Interruptor simple", "Interrupteur simple"],
        "Interruptor duplo" => ["Double switch", "Interruptor doble", "Interrupteur double"],
        "Interruptor paralelo (three-way)" => [
            "Two-way switch (three-way)",
            "Conmutador (three-way)",
            "Va-et-vient (three-way)",
        ],
        "Ponto de luz no teto" => [
            "Ceiling light point",
            "Punto de luz en el techo",
            "Point lumineux au plafond",
        ],
        "Arandela (ponto de luz na parede)" => [
            "Wall light (light point on the wall)",
            "Aplique (punto de luz en la pared)",
            "Applique (point lumineux mural)",
        ],
        "Quadro de distribuição" => ["Consumer unit", "Cuadro eléctrico", "Tableau électrique"],
        "Ponto de ar-condicionado" => [
            "Air conditioning point",
            "Punto de aire acondicionado",
            "Point de climatisation",
        ],
        "Ponto para chuveiro elétrico" => [
            "Electric shower point",
            "Punto para calentador de ducha",
            "Point pour chauffe-eau de douche",
        ],
        "Tomada de dados / TV" => [
            "Data / TV socket",
            "Toma de datos / TV",
            "Prise données / TV",
        ],
        "Ponto de rede RJ45 (Cat 6)" => [
            "RJ45 network point (Cat 6)",
            "Punto de red RJ45 (Cat 6)",
            "Prise réseau RJ45 (Cat 6)",
        ],
        "Ponto de TV (coaxial)" => [
            "TV point (coaxial)",
            "Punto de TV (coaxial)",
            "Prise TV (coaxiale)",
        ],
        "Ponto de Wi-Fi no teto (access point)" => [
            "Ceiling Wi-Fi point (access point)",
            "Punto de Wi-Fi en el techo (punto de acceso)",
            "Point Wi-Fi au plafond (point d'accès)",
        ],
        "Quadro de telecomunicações (rack / DG)" => [
            "Telecom panel (rack / MDF)",
            "Cuadro de telecomunicaciones (rack / RITI)",
            "Coffret de communication (baie / DTI)",
        ],
        "Campainha" => ["Doorbell", "Timbre", "Sonnette"],
        "Torre de tomadas retrátil de embutir (furo 60 mm, 3 tomadas, plugue)" => [
            "Retractable socket tower, built in (60 mm hole, 3 sockets, plug)",
            "Torre de enchufes retráctil empotrable (agujero 60 mm, 3 enchufes, clavija)",
            "Bloc de prises escamotable encastré (perçage 60 mm, 3 prises, fiche)",
        ],
        "Torre de tomadas automática com indução (furo 85 mm, ligada à instalação)" => [
            "Automatic socket tower with induction (85 mm hole, wired in)",
            "Torre de enchufes automática con inducción (agujero 85 mm, conectada a la instalación)",
            "Bloc de prises automatique avec induction (perçage 85 mm, raccordé)",
        ],
        "Torre de 4 tomadas + USB e indução (furo 100 mm)" => [
            "4-socket tower + USB and induction (100 mm hole)",
            "Torre de 4 enchufes + USB e inducción (agujero 100 mm)",
            "Bloc 4 prises + USB et induction (perçage 100 mm)",
        ],
        "Caixa de tomadas de mesa (4 tomadas, RJ45, USB, HDMI; recorte 120×335 mm)" => [
            "Desk socket box (4 sockets, RJ45, USB, HDMI; 120×335 mm cutout)",
            "Caja de enchufes de mesa (4 enchufes, RJ45, USB, HDMI; recorte 120×335 mm)",
            "Boîtier de prises de table (4 prises, RJ45, USB, HDMI ; découpe 120×335 mm)",
        ],
        "Tomada de embutir em móvel (furo 35 mm)" => [
            "Socket built into furniture (35 mm hole)",
            "Enchufe empotrado en el mueble (agujero 35 mm)",
            "Prise encastrée dans le meuble (perçage 35 mm)",
        ],
        "Relé de automação (atrás do interruptor ou da luminária)" => [
            "Automation relay (behind the switch or the fixture)",
            "Relé de automatización (detrás del interruptor o de la luminaria)",
            "Relais domotique (derrière l'interrupteur ou le luminaire)",
        ],
        "Interruptor inteligente" => [
            "Smart switch",
            "Interruptor inteligente",
            "Interrupteur connecté",
        ],
        "Dimmer" => ["Dimmer", "Regulador", "Variateur"],
        "Sensor de presença de teto" => [
            "Ceiling presence sensor",
            "Sensor de presencia de techo",
            "Détecteur de présence au plafond",
        ],
        "Fechadura eletrônica" => [
            "Electronic lock",
            "Cerradura electrónica",
            "Serrure électronique",
        ],
        "Ponto de água fria" => [
            "Cold water point",
            "Punto de agua fría",
            "Point d'eau froide",
        ],
        "Ponto de água quente" => [
            "Hot water point",
            "Punto de agua caliente",
            "Point d'eau chaude",
        ],
        "Ponto de esgoto" => ["Drain point", "Punto de desagüe", "Point d'évacuation"],
        "Caixa sifonada 150×150×50 (ralo)" => [
            "Trapped gully 150×150×50 (drain)",
            "Sumidero sifónico 150×150×50",
            "Siphon de sol 150×150×50",
        ],
        "Caixa sifonada 100×150×50 (ralo)" => [
            "Trapped gully 100×150×50 (drain)",
            "Sumidero sifónico 100×150×50",
            "Siphon de sol 100×150×50",
        ],
        "Caixa sifonada 150×185×75 (ralo)" => [
            "Trapped gully 150×185×75 (drain)",
            "Sumidero sifónico 150×185×75",
            "Siphon de sol 150×185×75",
        ],
        "Ralo sifonado 100 mm (fecho hídrico curto)" => [
            "Trapped floor drain 100 mm (shallow seal)",
            "Sumidero sifónico 100 mm (cierre hídrico corto)",
            "Bonde siphoïde 100 mm (garde d'eau courte)",
        ],
        "Ralo seco 100 mm" => [
            "Dry floor drain 100 mm",
            "Sumidero seco 100 mm",
            "Bonde sèche 100 mm",
        ],
        "Ralo linear 70 cm (sem sifão)" => [
            "Linear drain 70 cm (untrapped)",
            "Canaleta lineal 70 cm (sin sifón)",
            "Caniveau 70 cm (sans siphon)",
        ],
        "Ralo linear sifonado 70 cm" => [
            "Trapped linear drain 70 cm",
            "Canaleta lineal sifónica 70 cm",
            "Caniveau siphoïde 70 cm",
        ],
        "Ralo de águas pluviais (varanda descoberta, terraço)" => [
            "Rainwater drain (open balcony, terrace)",
            "Sumidero de pluviales (balcón descubierto, terraza)",
            "Bonde d'eaux pluviales (balcon découvert, terrasse)",
        ],
        "Registro" => ["Stopcock", "Llave de paso", "Robinet d'arrêt"],
        "Caixa de gordura" => ["Grease trap", "Trampa de grasas", "Bac à graisse"],
        "Grelha de ventilação permanente (gás)" => [
            "Permanent ventilation grille (gas)",
            "Rejilla de ventilación permanente (gas)",
            "Grille de ventilation permanente (gaz)",
        ],
        "Tubo de ventilação" => ["Vent pipe", "Tubo de ventilación", "Colonne de ventilation"],
        "Caixa de inspeção" => [
            "Inspection chamber",
            "Arqueta de inspección",
            "Regard de visite",
        ],
        "Hidrômetro" => ["Water meter", "Contador de agua", "Compteur d'eau"],
        "Ponto de gás" => ["Gas point", "Punto de gas", "Point de gaz"],
        // What walls are built of and what surfaces are finished with, as
        // the pickers list them.
        "Drywall 73 mm" => [
            "Drywall 73 mm",
            "Yeso laminado 73 mm",
            "Plaque de plâtre 73 mm",
        ],
        "Drywall 95 mm" => [
            "Drywall 95 mm",
            "Yeso laminado 95 mm",
            "Plaque de plâtre 95 mm",
        ],
        "Drywall 115 mm" => [
            "Drywall 115 mm",
            "Yeso laminado 115 mm",
            "Plaque de plâtre 115 mm",
        ],
        "Drywall 120 mm chapa dupla" => [
            "Drywall 120 mm, double board",
            "Yeso laminado 120 mm, placa doble",
            "Plaque de plâtre 120 mm, double parement",
        ],
        "Tijolo cerâmico 9 cm" => [
            "Clay brick 9 cm",
            "Ladrillo cerámico 9 cm",
            "Brique en terre cuite 9 cm",
        ],
        "Tijolo cerâmico 14 cm" => [
            "Clay brick 14 cm",
            "Ladrillo cerámico 14 cm",
            "Brique en terre cuite 14 cm",
        ],
        "Tijolo cerâmico 19 cm" => [
            "Clay brick 19 cm",
            "Ladrillo cerámico 19 cm",
            "Brique en terre cuite 19 cm",
        ],
        "Bloco de concreto 14 cm" => [
            "Concrete block 14 cm",
            "Bloque de hormigón 14 cm",
            "Bloc de béton 14 cm",
        ],
        "Bloco de concreto 19 cm" => [
            "Concrete block 19 cm",
            "Bloque de hormigón 19 cm",
            "Bloc de béton 19 cm",
        ],
        "Parede de concreto 10 cm" => [
            "Concrete wall 10 cm",
            "Muro de hormigón 10 cm",
            "Mur en béton 10 cm",
        ],
        "Concreto armado 15 cm" => [
            "Reinforced concrete 15 cm",
            "Hormigón armado 15 cm",
            "Béton armé 15 cm",
        ],
        "Steel frame 14 cm" => [
            "Steel frame 14 cm",
            "Steel frame 14 cm",
            "Ossature métallique 14 cm",
        ],
        "Wood frame 10 cm" => [
            "Wood frame 10 cm",
            "Entramado de madera 10 cm",
            "Ossature bois 10 cm",
        ],
        "Divisória de vidro" => ["Glass partition", "Mampara de vidrio", "Cloison vitrée"],
        "Montante 48 mm + 1 chapa de 12,5 mm em cada face" => [
            "48 mm stud + one 12.5 mm board each side",
            "Montante de 48 mm + 1 placa de 12,5 mm por cara",
            "Montant 48 mm + 1 plaque de 12,5 mm par face",
        ],
        "Montante 70 mm + 1 chapa de 12,5 mm em cada face" => [
            "70 mm stud + one 12.5 mm board each side",
            "Montante de 70 mm + 1 placa de 12,5 mm por cara",
            "Montant 70 mm + 1 plaque de 12,5 mm par face",
        ],
        "Montante 90 mm + 1 chapa de 12,5 mm em cada face" => [
            "90 mm stud + one 12.5 mm board each side",
            "Montante de 90 mm + 1 placa de 12,5 mm por cara",
            "Montant 90 mm + 1 plaque de 12,5 mm par face",
        ],
        "Montante 70 mm + 2 chapas de 12,5 mm em cada face (acústica)" => [
            "70 mm stud + two 12.5 mm boards each side (acoustic)",
            "Montante de 70 mm + 2 placas de 12,5 mm por cara (acústica)",
            "Montant 70 mm + 2 plaques de 12,5 mm par face (acoustique)",
        ],
        "Bloco de 9 cm + reboco de 2,5 cm em cada face" => [
            "9 cm block + 2.5 cm render each side",
            "Bloque de 9 cm + enfoscado de 2,5 cm por cara",
            "Bloc de 9 cm + enduit de 2,5 cm par face",
        ],
        "Bloco de 14 cm + reboco de 2,5 cm em cada face" => [
            "14 cm block + 2.5 cm render each side",
            "Bloque de 14 cm + enfoscado de 2,5 cm por cara",
            "Bloc de 14 cm + enduit de 2,5 cm par face",
        ],
        "Bloco de 19 cm + reboco de 2,5 cm em cada face" => [
            "19 cm block + 2.5 cm render each side",
            "Bloque de 19 cm + enfoscado de 2,5 cm por cara",
            "Bloc de 19 cm + enduit de 2,5 cm par face",
        ],
        "Bloco estrutural de 14 cm + revestimento de 1,5 cm em cada face" => [
            "14 cm structural block + 1.5 cm finish each side",
            "Bloque estructural de 14 cm + revestimiento de 1,5 cm por cara",
            "Bloc structurel de 14 cm + revêtement de 1,5 cm par face",
        ],
        "Bloco estrutural de 19 cm + revestimento de 1,5 cm em cada face" => [
            "19 cm structural block + 1.5 cm finish each side",
            "Bloque estructural de 19 cm + revestimiento de 1,5 cm por cara",
            "Bloc structurel de 19 cm + revêtement de 1,5 cm par face",
        ],
        "Concreto moldado in loco, sistema parede de concreto" => [
            "Cast-in-place concrete, concrete wall system",
            "Hormigón moldeado in situ, sistema de muro de hormigón",
            "Béton coulé en place, système de mur en béton",
        ],
        "Parede estrutural de concreto armado" => [
            "Structural reinforced concrete wall",
            "Muro estructural de hormigón armado",
            "Mur structurel en béton armé",
        ],
        "Perfil 90 mm + OSB e placa cimentícia/gesso" => [
            "90 mm profile + OSB and cement/plaster board",
            "Perfil de 90 mm + OSB y placa de cemento/yeso",
            "Profilé 90 mm + OSB et plaque ciment/plâtre",
        ],
        "Estrutura de madeira com fechamento em chapas" => [
            "Timber frame closed with boards",
            "Estructura de madera cerrada con placas",
            "Ossature bois fermée par des panneaux",
        ],
        "Vidro temperado de 10 mm" => [
            "10 mm toughened glass",
            "Vidrio templado de 10 mm",
            "Verre trempé de 10 mm",
        ],
        "Madeira (réguas)" => ["Wood (planks)", "Madera (tablas)", "Bois (lames)"],
        "Taco espinha de peixe" => [
            "Herringbone parquet",
            "Parqué en espiga",
            "Parquet à chevrons",
        ],
        "Porcelanato / cerâmica" => [
            "Porcelain / ceramic tile",
            "Porcelánico / cerámica",
            "Grès cérame / carrelage",
        ],
        "Azulejo metrô" => ["Subway tile", "Azulejo metro", "Carrelage métro"],
        "Tijolo aparente" => ["Exposed brick", "Ladrillo visto", "Brique apparente"],
        "Pedra" => ["Stone", "Piedra", "Pierre"],
        "Cimento queimado" => ["Polished cement", "Cemento pulido", "Béton ciré"],
        "Mármore" => ["Marble", "Mármol", "Marbre"],
        "Carpete" => ["Carpet", "Moqueta", "Moquette"],
        "Grama" => ["Grass", "Césped", "Gazon"],
        "Água (piscina)" => ["Water (pool)", "Agua (piscina)", "Eau (piscine)"],
        "Deck de madeira" => ["Wood deck", "Tarima de madera", "Terrasse en bois"],
        "O projeto mudou na nuvem: esta aba baixou uma cópia do seu e abriu a versão da nuvem." => {
            [
                "The project changed in the cloud: this tab downloaded a copy of yours and opened the cloud version.",
                "El proyecto cambió en la nube: esta pestaña descargó una copia del suyo y abrió la versión de la nube.",
                "Le projet a changé dans le cloud : cet onglet a téléchargé une copie du vôtre et ouvert la version du cloud.",
            ]
        }
        "⚠ O projeto mudou na nuvem e esta aba guardou uma cópia; a versão da nuvem não abriu: {}" => {
            [
                "⚠ The project changed in the cloud and this tab kept a copy; the cloud version did not open: {}",
                "⚠ El proyecto cambió en la nube y esta pestaña guardó una copia; la versión de la nube no se abrió: {}",
                "⚠ Le projet a changé dans le cloud et cet onglet a gardé une copie ; la version du cloud ne s'est pas ouverte : {}",
            ]
        }
        "Projeto recuperado desta sessão." => [
            "Project recovered from this session.",
            "Proyecto recuperado de esta sesión.",
            "Projet récupéré de cette session.",
        ],
        "No terminal:" => ["In a terminal:", "En el terminal:", "Dans un terminal :"],
        "Os instaladores já registram sozinhos se o Claude Code estiver instalado." => [
            "The installers register it for you when Claude Code is installed.",
            "Los instaladores lo registran solos si Claude Code está instalado.",
            "Les installateurs l'enregistrent tout seuls si Claude Code est installé.",
        ],
        "Configurações → Desenvolvedor → Editar configuração" => [
            "Settings → Developer → Edit Config",
            "Configuración → Desarrollador → Editar configuración",
            "Réglages → Développeur → Modifier la configuration",
        ],
        "Precisa do Node.js instalado para a ponte mcp-remote. Reinicie o Claude Desktop." => [
            "Needs Node.js for the mcp-remote bridge. Restart Claude Desktop.",
            "Necesita Node.js para el puente mcp-remote. Reinicie Claude Desktop.",
            "Nécessite Node.js pour le pont mcp-remote. Redémarrez Claude Desktop.",
        ],
        "Os instaladores já registram sozinhos se o Codex estiver instalado." => [
            "The installers register it for you when Codex is installed.",
            "Los instaladores lo registran solos si Codex está instalado.",
            "Les installateurs l'enregistrent tout seuls si Codex est installé.",
        ],
        "Depois abra o Gemini CLI e peça o projeto." => [
            "Then open Gemini CLI and ask for your project.",
            "Después abra Gemini CLI y pídale el proyecto.",
            "Ouvrez ensuite Gemini CLI et demandez-lui votre projet.",
        ],
        "Use no modo agente do Copilot." => [
            "Use it in Copilot's agent mode.",
            "Úselo en el modo agente de Copilot.",
            "À utiliser dans le mode agent de Copilot.",
        ],
        "Arquivo ~/.cursor/mcp.json" => [
            "File ~/.cursor/mcp.json",
            "Archivo ~/.cursor/mcp.json",
            "Fichier ~/.cursor/mcp.json",
        ],
        "Reinicie o Cursor depois de salvar." => [
            "Restart Cursor after saving.",
            "Reinicie Cursor después de guardar.",
            "Redémarrez Cursor après avoir enregistré.",
        ],
        "Outro app" => ["Another app", "Otra app", "Une autre app"],
        "Nas configurações de MCP do app (Cline, Roo Code, Cherry Studio, LM Studio…)" => [
            "In the app's MCP settings (Cline, Roo Code, Cherry Studio, LM Studio…)",
            "En los ajustes de MCP de la app (Cline, Roo Code, Cherry Studio, LM Studio…)",
            "Dans les réglages MCP de l'app (Cline, Roo Code, Cherry Studio, LM Studio…)",
        ],
        "O que importa é o app falar MCP: o modelo pode ser qualquer um." => [
            "What matters is that the app speaks MCP: the model can be any of them.",
            "Lo que importa es que la app hable MCP: el modelo puede ser cualquiera.",
            "Ce qui compte, c'est que l'app parle MCP : le modèle peut être n'importe lequel.",
        ],
        "agora" => ["just now", "ahora", "à l'instant"],
        "há {} s" => ["{} s ago", "hace {} s", "il y a {} s"],
        "há {} min" => ["{} min ago", "hace {} min", "il y a {} min"],
        "há {} h" => ["{} h ago", "hace {} h", "il y a {} h"],
        "{} · {} chamadas" => ["{} · {} calls", "{} · {} llamadas", "{} · {} appels"],
        "MCP · esperando sua IA" => [
            "MCP · waiting for your AI",
            "MCP · esperando su IA",
            "MCP · en attente de votre IA",
        ],
        "IA: só no aplicativo" => [
            "AI: in the app only",
            "IA: solo en la aplicación",
            "IA : seulement dans l'application",
        ],
        "Conectar sua IA — clique para ver como" => [
            "Connect your AI — click to see how",
            "Conecte su IA — haga clic para ver cómo",
            "Connectez votre IA — cliquez pour voir comment",
        ],
        "IA" => ["AI", "IA", "IA"],
        "{} conectado · {}" => ["{} connected · {}", "{} conectado · {}", "{} connecté · {}"],
        "Nenhuma IA conectada ainda" => [
            "No AI connected yet",
            "Ninguna IA conectada todavía",
            "Aucune IA connectée pour l'instant",
        ],
        "Conectar sua IA…" => ["Connect your AI…", "Conecte su IA…", "Connecter votre IA…"],
        "Copiar o endereço do MCP" => [
            "Copy the MCP address",
            "Copiar la dirección del MCP",
            "Copier l'adresse MCP",
        ],
        "Endereço copiado" => ["Address copied", "Dirección copiada", "Adresse copiée"],
        "Conectar sua IA" => ["Connect your AI", "Conecte su IA", "Connecter votre IA"],
        "O editor abre uma porta MCP: a sua IA desenha aqui dentro, em centímetros, e você vê acontecer." => {
            [
                "The editor opens an MCP port: your AI draws in here, in centimetres, and you watch it happen.",
                "El editor abre un puerto MCP: su IA dibuja aquí dentro, en centímetros, y usted lo ve ocurrir.",
                "L'éditeur ouvre un port MCP : votre IA dessine ici, en centimètres, et vous la regardez faire.",
            ]
        }
        "Agora" => ["Right now", "Ahora mismo", "En ce moment"],
        "Servidor MCP ligado" => [
            "MCP server on",
            "Servidor MCP encendido",
            "Serveur MCP allumé",
        ],
        "No navegador o editor roda sozinho: o MCP vive no aplicativo do computador." => [
            "In a browser the editor runs on its own: the MCP lives in the desktop app.",
            "En el navegador el editor funciona solo: el MCP vive en la aplicación de escritorio.",
            "Dans un navigateur, l'éditeur tourne seul : le MCP vit dans l'application de bureau.",
        ],
        "O servidor está desligado (--no-server): reabra o aplicativo sem essa opção." => [
            "The server is off (--no-server): reopen the app without that option.",
            "El servidor está apagado (--no-server): vuelva a abrir la aplicación sin esa opción.",
            "Le serveur est éteint (--no-server) : rouvrez l'application sans cette option.",
        ],
        "Nenhuma IA conectada ainda — siga os três passos abaixo." => [
            "No AI connected yet — the three steps below do it.",
            "Ninguna IA conectada todavía: los tres pasos de abajo lo resuelven.",
            "Aucune IA connectée — les trois étapes ci-dessous s'en chargent.",
        ],
        "1. Escolha o aplicativo de IA que você usa" => [
            "1. Pick the AI app you use",
            "1. Elija la app de IA que usa",
            "1. Choisissez l'app d'IA que vous utilisez",
        ],
        "2. Cole isto onde ele pede" => [
            "2. Paste this where it asks",
            "2. Pegue esto donde lo pide",
            "2. Collez ceci là où il le demande",
        ],
        "3. Peça alguma coisa" => [
            "3. Ask it for something",
            "3. Pídale algo",
            "3. Demandez-lui quelque chose",
        ],
        "Quantas paredes tem este projeto? Depois coloque uma janela de 120 cm na sala." => [
            "How many walls does this project have? Then put a 120 cm window in the living room.",
            "¿Cuántas paredes tiene este proyecto? Después ponga una ventana de 120 cm en el salón.",
            "Combien de murs a ce projet ? Ensuite, posez une fenêtre de 120 cm dans le salon.",
        ],
        "Copiado" => ["Copied", "Copiado", "Copié"],
        "Últimas chamadas" => ["Latest calls", "Últimas llamadas", "Derniers appels"],
        "Nada ainda. Assim que sua IA usar uma ferramenta, ela aparece aqui." => [
            "Nothing yet. The moment your AI uses a tool, it shows up here.",
            "Nada todavía. En cuanto su IA use una herramienta, aparece aquí.",
            "Rien encore. Dès que votre IA utilise un outil, cela apparaît ici.",
        ],
        "Baixar o aplicativo" => [
            "Download the app",
            "Descargar la aplicación",
            "Télécharger l'application",
        ],
        "{} conectado · {} chamadas · {}" => [
            "{} connected · {} calls · {}",
            "{} conectado · {} llamadas · {}",
            "{} connecté · {} appels · {}",
        ],
        "{} conectou — sua IA já pode desenhar aqui" => [
            "{} connected — your AI can draw in here now",
            "{} se conectó — su IA ya puede dibujar aquí",
            "{} s'est connecté — votre IA peut dessiner ici",
        ],
        "Registrar agora" => ["Register now", "Registrar ahora", "Enregistrer maintenant"],
        "Registrando…" => ["Registering…", "Registrando…", "Enregistrement…"],
        "Roda esse mesmo comando aqui, sem abrir o terminal." => [
            "Runs that same command from here, no terminal needed.",
            "Ejecuta ese mismo comando desde aquí, sin abrir el terminal.",
            "Exécute cette même commande d'ici, sans ouvrir de terminal.",
        ],
        "Registrado no {} — agora é só pedir" => [
            "Registered with {} — now just ask",
            "Registrado en {} — ahora solo pida",
            "Enregistré dans {} — il n'y a plus qu'à demander",
        ],
        "Não deu para registrar: {}" => [
            "Could not register: {}",
            "No se pudo registrar: {}",
            "Impossible d'enregistrer : {}",
        ],
        "Ligue e esta aba ganha um endereço que a sua IA alcança. O projeto não sai daqui: o servidor no meio só passa recados." => {
            [
                "Switch it on and this tab gets an address your AI can reach. The project stays here: the server in between only passes messages.",
                "Actívelo y esta pestaña obtiene una dirección a la que su IA llega. El proyecto no sale de aquí: el servidor del medio solo pasa mensajes.",
                "Activez-le et cet onglet reçoit une adresse que votre IA peut joindre. Le projet reste ici : le serveur au milieu ne fait que passer les messages.",
            ]
        }
        "Ligar o MCP nesta aba" => [
            "Switch the MCP on in this tab",
            "Activar el MCP en esta pestaña",
            "Activer le MCP dans cet onglet",
        ],
        "Ligando…" => ["Switching on…", "Activando…", "Activation…"],
        "Esta aba está no ar para a sua IA" => [
            "This tab is reachable by your AI",
            "Esta pestaña está al alcance de su IA",
            "Cet onglet est joignable par votre IA",
        ],
        "Quem tiver o endereço abaixo pode editar este projeto: trate como senha." => [
            "Anyone holding the address below can edit this project: treat it like a password.",
            "Quien tenga la dirección de abajo puede editar este proyecto: trátela como una contraseña.",
            "Quiconque détient l'adresse ci-dessous peut modifier ce projet : traitez-la comme un mot de passe.",
        ],
        "Não deu para ligar: {}" => [
            "Could not switch it on: {}",
            "No se pudo activar: {}",
            "Impossible de l'activer : {}",
        ],
        "Tentar de novo" => ["Try again", "Intentar de nuevo", "Réessayer"],
        "{} só no aplicativo do computador: aqui ele renderiza na CPU e travaria a aba" => [
            "{} only in the desktop app: here it renders on the CPU and would freeze the tab",
            "{} solo en la aplicación de escritorio: aquí renderiza en la CPU y congelaría la pestaña",
            "{} seulement dans l'application de bureau : ici il rend sur le CPU et figerait l'onglet",
        ],
        "MCP desligado · ligar" => [
            "MCP off · switch on",
            "MCP apagado · activar",
            "MCP éteint · activer",
        ],
        "Desligar" => ["Switch off", "Apagar", "Éteindre"],
        "o relay recusou esta aba — ligue de novo" => [
            "the relay refused this tab — switch it on again",
            "el relay rechazó esta pestaña — actívelo de nuevo",
            "le relais a refusé cet onglet — rallumez-le",
        ],
        "Conectado como {}" => [
            "Signed in as {}",
            "Conectado como {}",
            "Connecté en tant que {}",
        ],
        "Com este endereço, o Claude, o ChatGPT e outras IAs chegam nesta aba entrando com a sua conta." => {
            [
                "With this address, Claude, ChatGPT and other AIs reach this tab by signing in with your account.",
                "Con esta dirección, Claude, ChatGPT y otras IA llegan a esta pestaña iniciando sesión con su cuenta.",
                "Avec cette adresse, Claude, ChatGPT et d'autres IA atteignent cet onglet en se connectant avec votre compte.",
            ]
        }
        "Entrar para usar no Claude e no ChatGPT" => [
            "Sign in to use it in Claude and ChatGPT",
            "Iniciar sesión para usarlo en Claude y ChatGPT",
            "Se connecter pour l'utiliser dans Claude et ChatGPT",
        ],
        "Com uma conta, a sua IA chega nesta aba por um endereço fixo, sem colar nada." => [
            "With an account, your AI reaches this tab at one fixed address, with nothing to paste.",
            "Con una cuenta, su IA llega a esta pestaña por una dirección fija, sin pegar nada.",
            "Avec un compte, votre IA atteint cet onglet à une adresse fixe, sans rien coller.",
        ],
        "Fecha o endereço. A sua IA perde o acesso na hora; o projeto continua aqui." => [
            "Closes the address. Your AI loses access at once; the project stays here.",
            "Cierra la dirección. Su IA pierde el acceso al instante; el proyecto sigue aquí.",
            "Ferme l'adresse. Votre IA perd l'accès aussitôt ; le projet reste ici.",
        ],
        "Conectar sua IA (no navegador, liga e desliga)" => [
            "Connect your AI (in a browser, switches it on and off)",
            "Conecte su IA (en el navegador, activa y desactiva)",
            "Connecter votre IA (dans un navigateur, active et désactive)",
        ],
        "3D" => ["3D", "3D", "3D"],
        "Catálogo · Planta · 3D (telas estreitas)" => [
            "Catalogue · Plan · 3D (narrow screens)",
            "Catálogo · Plano · 3D (pantallas estrechas)",
            "Catalogue · Plan · 3D (écrans étroits)",
        ],
        "o servidor que leva a sua IA até esta aba não respondeu — no aplicativo do computador o MCP não precisa dele" => {
            [
                "the server that carries your AI to this tab did not answer — in the desktop app the MCP needs no server",
                "el servidor que lleva su IA hasta esta pestaña no respondió — en la aplicación de escritorio el MCP no lo necesita",
                "le serveur qui relie votre IA à cet onglet n'a pas répondu — dans l'application de bureau, le MCP n'en a pas besoin",
            ]
        }
        "Disciplina, camadas e andares" => [
            "Discipline, layers and storeys",
            "Disciplina, capas y plantas",
            "Discipline, calques et étages",
        ],
        "IA · esperando" => ["AI · waiting", "IA · esperando", "IA · en attente"],
        "Conectar IA" => ["Connect AI", "Conectar IA", "Connecter l'IA"],
        "{} conectado · {} chamadas" => [
            "{} connected · {} calls",
            "{} conectado · {} llamadas",
            "{} connecté · {} appels",
        ],
        "Esperando a sua IA chegar" => [
            "Waiting for your AI to arrive",
            "Esperando a que llegue su IA",
            "En attente de votre IA",
        ],
        "Ainda não dá para alcançar esta janela" => [
            "Nothing can reach this window yet",
            "Todavía nada puede alcanzar esta ventana",
            "Rien ne peut encore joindre cette fenêtre",
        ],
        "Escolha o aplicativo de IA que você usa" => [
            "Pick the AI app you use",
            "Elija la app de IA que usa",
            "Choisissez l'app d'IA que vous utilisez",
        ],
        "Cole isto onde ele pede" => [
            "Paste this where it asks",
            "Pegue esto donde lo pide",
            "Collez ceci là où il le demande",
        ],
        "Peça alguma coisa" => [
            "Ask it for something",
            "Pídale algo",
            "Demandez-lui quelque chose",
        ],
        "o endereço guardado não vale mais — ligue de novo" => [
            "the address kept here is no longer yours — switch it on again",
            "la dirección guardada ya no es suya — actívela de nuevo",
            "l'adresse gardée ici n'est plus la vôtre — activez-la à nouveau",
        ],
        "No ar, esperando a sua IA" => [
            "Up, waiting for your AI",
            "En el aire, esperando su IA",
            "En ligne, en attente de votre IA",
        ],
        "Quem tiver esse endereço edita este projeto: trate como senha." => [
            "Anyone with that address edits this project: treat it like a password.",
            "Quien tenga esa dirección edita este proyecto: trátela como una contraseña.",
            "Quiconque a cette adresse modifie ce projet : traitez-la comme un mot de passe.",
        ],
        "Abrir do servidor…" => [
            "Open from server…",
            "Abrir desde el servidor…",
            "Ouvrir depuis le serveur…",
        ],
        "Novo projeto no servidor…" => [
            "New project on server…",
            "Nuevo proyecto en el servidor…",
            "Nouveau projet sur le serveur…",
        ],
        "Novo projeto no servidor" => [
            "New project on the server",
            "Nuevo proyecto en el servidor",
            "Nouveau projet sur le serveur",
        ],
        "Projetos no servidor" => [
            "Projects on the server",
            "Proyectos en el servidor",
            "Projets sur le serveur",
        ],
        "Carregando os projetos…" => [
            "Loading the projects…",
            "Cargando los proyectos…",
            "Chargement des projets…",
        ],
        "⚠ Não foi possível listar os projetos: {}" => [
            "⚠ Could not list the projects: {}",
            "⚠ No se pudieron listar los proyectos: {}",
            "⚠ Impossible de lister les projets : {}",
        ],
        "Nenhum projeto no servidor ainda." => [
            "No projects on the server yet.",
            "Todavía no hay proyectos en el servidor.",
            "Aucun projet sur le serveur pour l'instant.",
        ],
        "Última alteração" => ["Last change", "Último cambio", "Dernière modification"],
        "Criando o projeto…" => [
            "Creating the project…",
            "Creando el proyecto…",
            "Création du projet…",
        ],
        "⚠ O servidor recusou: {}" => [
            "⚠ The server refused: {}",
            "⚠ El servidor lo rechazó: {}",
            "⚠ Le serveur a refusé : {}",
        ],
        "Entre na sua conta para ver os projetos do servidor." => [
            "Sign in to your account to see the projects on the server.",
            "Inicie sesión en su cuenta para ver los proyectos del servidor.",
            "Connectez-vous à votre compte pour voir les projets du serveur.",
        ],
        "O serviço respondeu algo inesperado: {}" => [
            "The service answered something unexpected: {}",
            "El servicio respondió algo inesperado: {}",
            "Le service a répondu quelque chose d'inattendu : {}",
        ],
        "O serviço respondeu algo inesperado" => [
            "The service answered something unexpected",
            "El servicio respondió algo inesperado",
            "Le service a répondu quelque chose d'inattendu",
        ],
        "O serviço respondeu {}" => [
            "The service answered {}",
            "El servicio respondió {}",
            "Le service a répondu {}",
        ],
        "Aberto do servidor: {}" => [
            "Opened from the server: {}",
            "Abierto desde el servidor: {}",
            "Ouvert depuis le serveur : {}",
        ],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every literal handed to `tr` has every other language, or the window
    /// silently falls back to Portuguese for whoever picked one of them —
    /// which is exactly how a new panel ships half translated.
    #[test]
    fn every_string_on_screen_is_translated() {
        fn sources(dir: &std::path::Path, into: &mut String) {
            for entry in std::fs::read_dir(dir).expect("src is readable") {
                let path = entry.expect("entry").path();
                if path.is_dir() {
                    sources(&path, into);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    into.push_str(&std::fs::read_to_string(&path).expect("source"));
                }
            }
        }
        let mut code = String::new();
        sources(std::path::Path::new("src"), &mut code);
        // A literal handed to tr, whatever the wrapping and the trailing comma.
        let mut missing: Vec<String> = Vec::new();
        for needle in ["tr(", "fill("] {
            missing.extend(untranslated(&code, needle));
        }
        assert!(missing.is_empty(), "not translated: {missing:#?}");
    }

    /// Every literal handed to `call` — `tr(`, `fill(` — that no language
    /// speaks, or that some language leaves blank or without the `{}` the
    /// sentence needs.
    fn untranslated(code: &str, call: &str) -> Vec<String> {
        let mut missing: Vec<String> = Vec::new();
        let mut rest = code;
        while let Some(at) = rest.find(call) {
            // Match a function name, not the suffix of from_str or refill.
            let identifier_prefix = rest[..at]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
            rest = &rest[at + call.len()..];
            if identifier_prefix {
                continue;
            }
            let Some(open) = rest.find('"') else { break };
            if rest[..open].contains(|c: char| !c.is_whitespace()) {
                continue;
            }
            let body = &rest[open + 1..];
            let Some(close) = body.find('"') else { break };
            let (text, after) = (&body[..close], body[close + 1..].trim_start());
            if after.starts_with(',') || after.starts_with(')') {
                // The test's own probe, and words spelled the same everywhere.
                if ["Algo novo", "Plugins", "cooktop"].contains(&text) {
                    continue;
                }
                // A blank side is worse than no row: the label disappears.
                // A side without the `{}` of the key drops what goes in it.
                let places = text.matches("{}").count();
                match translations(text) {
                    Some(row)
                        if row
                            .iter()
                            .all(|t| !t.trim().is_empty() && t.matches("{}").count() == places) => {
                    }
                    _ => missing.push(text.to_owned()),
                }
            }
        }
        missing
    }

    #[test]
    fn translation_scan_matches_function_names_not_identifier_suffixes() {
        let code = "JsValue::from_str(\"neweraOperation\"); refill(\"buffer\"); i18n::tr(\"Missing UI text\"); tr(\"Salvar\"); fill(\"Missing format {}\", &[]);";
        assert_eq!(untranslated(code, "tr("), vec!["Missing UI text"]);
        assert_eq!(untranslated(code, "fill("), vec!["Missing format {}"]);
    }

    /// These run beside the tests that drive whole windows, which read the
    /// labels in Portuguese — so none of them touch the language the window
    /// is set to, they ask for a language by name.
    #[test]
    fn translates_to_the_chosen_language() {
        for (lang, saved) in [
            (Lang::Pt, "Salvar"),
            (Lang::En, "Save"),
            (Lang::Es, "Guardar"),
            (Lang::Fr, "Enregistrer"),
        ] {
            assert_eq!(say("Salvar", lang), saved);
            // A text nobody translated stays as written, in any language.
            assert_eq!(say("Algo novo", lang), "Algo novo");
        }
    }

    #[test]
    fn the_window_keeps_the_language_it_is_set_to() {
        for lang in Lang::ALL {
            assert_eq!(Lang::from_code(lang.code()), lang);
        }
    }

    #[test]
    fn fills_the_sentence_in_the_chosen_language() {
        assert_eq!(
            fill_in("Aberto: {}", Lang::Fr, &[&"casa.n3d"]),
            "Ouvert : casa.n3d"
        );
        assert_eq!(
            fill_in(
                "⚠ Não foi possível abrir {}: {}",
                Lang::Fr,
                &[&"casa.n3d", &"nada aqui"]
            ),
            "⚠ Impossible d'ouvrir casa.n3d : nada aqui"
        );
        assert_eq!(
            fill_in("Aberto: {}", Lang::Pt, &[&"casa.n3d"]),
            "Aberto: casa.n3d"
        );
        // Nothing to put in it, and nothing lost either.
        assert_eq!(fill_in("Salvo em {}", Lang::Pt, &[]), "Salvo em {}");
    }

    /// Every piece in the catalog is named in all four languages. Change the
    /// language and the sofa has to stop being a sofá — the catalog is the
    /// first thing anyone reads in this window.
    #[test]
    fn every_piece_of_the_catalog_is_named_in_every_language() {
        let mut missing: Vec<&str> = Vec::new();
        for item in newera_catalog::CATALOG {
            match translations(item.name) {
                Some(row) if row.iter().all(|t| !t.trim().is_empty()) => {}
                _ => missing.push(item.name),
            }
        }
        assert!(missing.is_empty(), "not translated: {missing:#?}");
    }

    #[test]
    fn reads_the_language_the_system_asks_for() {
        assert_eq!(Lang::from_locale("pt_BR.UTF-8"), Some(Lang::Pt));
        assert_eq!(Lang::from_locale("en_US.UTF-8"), Some(Lang::En));
        assert_eq!(Lang::from_locale("es-419"), Some(Lang::Es));
        assert_eq!(Lang::from_locale("fr_CA"), Some(Lang::Fr));
        assert_eq!(Lang::from_locale("de_DE.UTF-8"), None);
        assert_eq!(Lang::from_locale("C"), None);
        assert_eq!(Lang::from_locale(""), None);
    }

    #[test]
    fn only_english_writes_decimals_with_a_dot() {
        for lang in Lang::ALL {
            assert_eq!(lang.decimal_comma(), lang != Lang::En);
        }
    }
}
