//! Connecting an AI to this window — and showing, live, that it worked.
//!
//! The MCP server runs inside the editor, but the connection is made in
//! another program's settings file. Someone who has just installed this has no
//! way of telling a working setup from a typo: nothing on screen changes. So
//! this module is two things — the address and the exact snippet each client
//! wants, and the proof: who said hello, when, and the tools they are calling
//! as they call them.

use eframe::egui::{self, RichText};
use egui_phosphor::regular as icon;

use crate::app::NewEraApp;

/// One AI client and what it needs pasted where.
pub(crate) struct Client {
    pub(crate) label: &'static str,
    /// Where the snippet goes, in the person's language.
    pub(crate) place: &'static str,
    /// What to paste, with this window's address already in it.
    pub(crate) code: String,
    pub(crate) note: &'static str,
    /// The same thing as a command this window can run itself, when the
    /// client has a command line: `(program, arguments)`.
    #[cfg_attr(
        target_arch = "wasm32",
        allow(dead_code, reason = "a browser tab runs no command lines")
    )]
    pub(crate) run: Option<(&'static str, Vec<String>)>,
}

/// The address to hand out. A window without a server still shows what the
/// snippet will look like, with the address it would have.
pub(crate) fn address(app: &NewEraApp) -> String {
    #[cfg(target_arch = "wasm32")]
    if let crate::ai_web::Link::On { url } = &app.ai_link.borrow().link {
        return url.clone();
    }
    app.mcp_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:7878/mcp".to_owned())
}

/// Whether an AI can reach this window at all right now — a desktop with its
/// server up, or a tab that has been switched on.
pub(crate) fn reachable(app: &NewEraApp) -> bool {
    #[cfg(target_arch = "wasm32")]
    {
        return matches!(app.ai_link.borrow().link, crate::ai_web::Link::On { .. });
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        app.mcp_url.is_some()
    }
}

/// The clients people actually arrive with, each with the one line it wants.
pub(crate) fn clients(url: &str) -> Vec<Client> {
    let terminal = crate::i18n::tr("No terminal:");
    vec![
        Client {
            label: "Claude Code",
            place: terminal,
            code: format!(
                "claude mcp remove newera 2>/dev/null; claude mcp add --transport http newera {url}"
            ),
            note: crate::i18n::tr(
                "Os instaladores já registram sozinhos se o Claude Code estiver instalado.",
            ),
            run: Some((
                "claude",
                vec![
                    "mcp".to_owned(),
                    "add".to_owned(),
                    "--transport".to_owned(),
                    "http".to_owned(),
                    "newera".to_owned(),
                    url.to_owned(),
                ],
            )),
        },
        Client {
            label: "Claude Desktop",
            place: crate::i18n::tr("Configurações → Desenvolvedor → Editar configuração"),
            code: format!(
                "{{\n  \"mcpServers\": {{\n    \"newera\": {{\n      \"command\": \"npx\",\n      \"args\": [\"-y\", \"mcp-remote\", \"{url}\"]\n    }}\n  }}\n}}"
            ),
            note: crate::i18n::tr(
                "Precisa do Node.js instalado para a ponte mcp-remote. Reinicie o Claude Desktop.",
            ),
            run: None,
        },
        Client {
            label: "Codex",
            place: terminal,
            code: format!("codex mcp remove newera 2>/dev/null; codex mcp add newera --url {url}"),
            note: crate::i18n::tr(
                "Os instaladores já registram sozinhos se o Codex estiver instalado.",
            ),
            run: Some((
                "codex",
                vec![
                    "mcp".to_owned(),
                    "add".to_owned(),
                    "newera".to_owned(),
                    "--url".to_owned(),
                    url.to_owned(),
                ],
            )),
        },
        Client {
            label: "Gemini CLI",
            place: terminal,
            code: format!(
                "gemini mcp remove newera 2>/dev/null; gemini mcp add --transport http newera {url}"
            ),
            note: crate::i18n::tr("Depois abra o Gemini CLI e peça o projeto."),
            run: Some((
                "gemini",
                vec![
                    "mcp".to_owned(),
                    "add".to_owned(),
                    "--transport".to_owned(),
                    "http".to_owned(),
                    "newera".to_owned(),
                    url.to_owned(),
                ],
            )),
        },
        Client {
            label: "VS Code",
            place: terminal,
            code: format!(
                "code --add-mcp '{{\"name\":\"newera\",\"type\":\"http\",\"url\":\"{url}\"}}'"
            ),
            note: crate::i18n::tr("Use no modo agente do Copilot."),
            run: Some((
                "code",
                vec![
                    "--add-mcp".to_owned(),
                    format!("{{\"name\":\"newera\",\"type\":\"http\",\"url\":\"{url}\"}}"),
                ],
            )),
        },
        Client {
            label: "Cursor",
            place: crate::i18n::tr("Arquivo ~/.cursor/mcp.json"),
            code: format!(
                "{{\n  \"mcpServers\": {{\n    \"newera\": {{ \"url\": \"{url}\" }}\n  }}\n}}"
            ),
            note: crate::i18n::tr("Reinicie o Cursor depois de salvar."),
            run: None,
        },
        Client {
            label: crate::i18n::tr("Outro app"),
            place: crate::i18n::tr(
                "Nas configurações de MCP do app (Cline, Roo Code, Cherry Studio, LM Studio…)",
            ),
            code: format!(
                "{{\n  \"mcp\": {{\n    \"newera\": {{ \"type\": \"remote\", \"url\": \"{url}\" }}\n  }}\n}}"
            ),
            note: crate::i18n::tr(
                "O que importa é o app falar MCP: o modelo pode ser qualquer um.",
            ),
            run: None,
        },
    ]
}

/// What a registration attempt left behind: the program's name, or why not.
pub(crate) type RegisterSlot = std::sync::Arc<std::sync::Mutex<Option<Result<String, String>>>>;

/// Whether a command line is installed on this machine, by looking where the
/// shell would look. Windows keeps its command shims under other extensions.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn installed(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            let exe = dir.join(program);
            exe.is_file()
                || exe.with_extension("exe").is_file()
                || exe.with_extension("cmd").is_file()
                || exe.with_extension("bat").is_file()
        })
    })
}

/// Registers this window with a client, for the person, in the background —
/// the same command the panel shows, run instead of copied.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn register(app: &mut NewEraApp, program: &'static str, args: Vec<String>) {
    let slot: RegisterSlot = std::sync::Arc::default();
    app.ai_job = Some(slot.clone());
    std::thread::spawn(move || {
        let result = match std::process::Command::new(program).args(&args).output() {
            Ok(done) if done.status.success() => Ok(program.to_owned()),
            Ok(done) => {
                let said = String::from_utf8_lossy(&done.stderr);
                let said = said.trim();
                let said = if said.is_empty() {
                    String::from_utf8_lossy(&done.stdout).trim().to_owned()
                } else {
                    said.to_owned()
                };
                Err(said.lines().last().unwrap_or("").to_owned())
            }
            Err(err) => Err(err.to_string()),
        };
        if let Ok(mut guard) = slot.lock() {
            *guard = Some(result);
        }
    });
}

/// How long ago, in words, for a moment in Unix milliseconds.
pub(crate) fn ago(now_ms: u64, then_ms: u64) -> String {
    let seconds = now_ms.saturating_sub(then_ms) / 1000;
    match seconds {
        0..=4 => crate::i18n::tr("agora").to_owned(),
        5..=59 => crate::i18n::fill("há {} s", &[&seconds]),
        60..=3599 => crate::i18n::fill("há {} min", &[&(seconds / 60)]),
        _ => crate::i18n::fill("há {} h", &[&(seconds / 3600)]),
    }
}

/// What the window knows right now about the AI side, read once per frame.
pub(crate) struct Pulse {
    pub(crate) agents: Vec<newera_core::collab::Agent>,
    pub(crate) recent: Vec<newera_core::collab::Call>,
    pub(crate) calls: u64,
    pub(crate) now_ms: u64,
}

impl Pulse {
    pub(crate) fn read(app: &NewEraApp) -> Self {
        let doc = app.document.read();
        let agents = doc.agents();
        Self {
            agents: agents.list().to_vec(),
            recent: agents.recent().to_vec(),
            calls: agents.calls(),
            now_ms: newera_core::collab::now_ms(),
        }
    }

    /// The client heard from last, which on one desktop is the one.
    pub(crate) fn newest(&self) -> Option<&newera_core::collab::Agent> {
        self.agents.iter().max_by_key(|a| a.seen_ms)
    }

    /// Something happened in the last couple of seconds: worth a flash.
    pub(crate) fn busy(&self) -> bool {
        self.newest()
            .is_some_and(|a| self.now_ms.saturating_sub(a.seen_ms) < 2_000)
    }
}

/// Says it out loud, once, when an AI connects for the first time: the whole
/// point of this is that nobody has to go looking to find out whether it
/// worked.
pub(crate) fn announce(app: &mut NewEraApp) {
    // A registration started from the panel finishes in its own thread.
    let finished = app
        .ai_job
        .as_ref()
        .and_then(|slot| slot.lock().ok().and_then(|mut done| done.take()));
    if let Some(result) = finished {
        app.ai_job = None;
        match result {
            Ok(program) => app.set_status(crate::i18n::fill(
                "Registrado no {} — agora é só pedir",
                &[&program],
            )),
            Err(why) => app.set_status(format!(
                "⚠ {}",
                crate::i18n::fill("Não deu para registrar: {}", &[&why])
            )),
        }
    }
    let (agents, calls, last): (Vec<String>, u64, Option<(String, String)>) = {
        let doc = app.document.read();
        let agents = doc.agents();
        (
            agents.list().iter().map(|a| a.name.clone()).collect(),
            agents.calls(),
            agents
                .recent()
                .last()
                .map(|c| (c.tool.clone(), c.agent.clone())),
        )
    };
    // Every tool the agent uses says its name where messages go: from across
    // the room that is the difference between "it is working" and silence.
    if calls > app.announced_calls {
        app.announced_calls = calls;
        if let Some((tool, agent)) = last {
            // Two proper names and a dot: nothing here to translate.
            app.set_status(format!("{tool} · {agent}"));
        }
    }
    if agents.len() <= app.announced_agents {
        return;
    }
    let newcomer = agents[app.announced_agents..].join(", ");
    app.announced_agents = agents.len();
    app.set_status(crate::i18n::fill(
        "{} conectou — sua IA já pode desenhar aqui",
        &[&newcomer],
    ));
}

/// Icon-only AI control in its own toolbar group, immediately after Frame.
pub(crate) fn button(app: &mut NewEraApp, ui: &mut egui::Ui) {
    let t = crate::theme::of(ui.visuals());
    let pulse = Pulse::read(app);
    let live = reachable(app);
    let agent = pulse.newest().map(|a| a.name.clone());

    // Three states, three different things to say — and the one that needs a
    // person to act is the loud one.
    let (fill, ink, edge) = match (&agent, live) {
        (Some(_), true) => (
            t.accent_soft,
            if pulse.busy() { t.accent_strong } else { t.ok },
            if pulse.busy() { t.accent } else { t.ok },
        ),
        (None, true) => (t.accent_soft, t.accent, t.accent),
        (_, false) => (t.accent, t.deep, t.accent),
    };
    let hint = match (&agent, live) {
        (Some(name), true) => {
            crate::i18n::fill("{} conectado · {} chamadas", &[name, &pulse.calls])
        }
        (None, true) => crate::i18n::tr("IA · esperando").to_owned(),
        (_, false) => crate::i18n::tr("Conectar sua IA — clique para ver como").to_owned(),
    };

    let label = RichText::new(icon::ROBOT).size(17.0).strong().color(ink);
    // The edge is drawn over the button, not set as its stroke: egui sizes a
    // button for the theme's stroke, which is none at rest, so an extra one
    // made it shrink whenever the pointer came over it.
    let button = egui::Button::new(label)
        .fill(fill)
        .corner_radius(6)
        .min_size(egui::vec2(28.0, 26.0));
    let response = ui.add(button);
    ui.painter().rect_stroke(
        response.rect,
        6,
        egui::Stroke::new(1.0, edge),
        egui::StrokeKind::Inside,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &hint));
    if response.on_hover_text(hint).clicked() {
        app.dialog = Some(crate::dialogs::Dialog::ConnectAi { client: 0 });
    }
    // Refresh activity colors even when nobody is moving the pointer.
    if live {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(if pulse.busy() {
                150
            } else {
                800
            }));
    }
}

/// A card: the shape everything in this panel is built from — one surface,
/// one hairline, room to breathe.
fn card<R>(ui: &mut egui::Ui, t: crate::theme::Tokens, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(t.raised)
        .stroke(egui::Stroke::new(1.0, t.rule))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(14, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            body(ui)
        })
        .inner
}

/// The panel: where this window stands, what to paste to reach it, and what
/// the AI has done since. No steps to follow — a tab, a line to copy, done.
pub(crate) fn panel(app: &mut NewEraApp, ctx: &egui::Context, chosen: &mut usize) -> bool {
    let mut close = false;
    let t = ctx.style_of(ctx.theme());
    let t = crate::theme::of(&t.visuals);
    let url = address(app);
    let serving = reachable(app);
    let pulse = Pulse::read(app);
    let clients = clients(&url);
    *chosen = (*chosen).min(clients.len() - 1);
    let narrow = ctx.content_rect().width() < crate::app::NARROW;

    crate::theme::modal(ctx, egui::Id::new("connect-ai")).show(ctx, |ui| {
        ui.set_min_width(if narrow { 300.0 } else { 620.0 });
        ui.spacing_mut().item_spacing.y = 10.0;
        crate::theme::title(
            ui,
            &format!("{} {}", icon::ROBOT, crate::i18n::tr("Conectar sua IA")),
        );

        // ---- Where this window stands, in one line ----
        card(ui, t, |ui| {
            ui.horizontal(|ui| {
                let on = pulse.newest().is_some();
                crate::app::lamp(
                    ui,
                    match (serving, on, pulse.busy()) {
                        (_, true, true) => t.accent,
                        (_, true, false) | (true, false, _) => t.ok,
                        (false, ..) => t.ink_faint,
                    },
                );
                let (line, colour) = match pulse.newest() {
                    Some(agent) => (
                        crate::i18n::fill(
                            "{} conectado · {} chamadas · {}",
                            &[&agent.name, &pulse.calls, &ago(pulse.now_ms, agent.seen_ms)],
                        ),
                        t.ok,
                    ),
                    None if serving => (
                        crate::i18n::tr("No ar, esperando a sua IA").to_owned(),
                        t.ink,
                    ),
                    None => (
                        crate::i18n::tr("Ainda não dá para alcançar esta janela").to_owned(),
                        t.ink_dim,
                    ),
                };
                ui.label(RichText::new(line).size(14.0).strong().color(colour));
                #[cfg(target_arch = "wasm32")]
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    web_switch(app, ui, serving);
                });
            });
            if serving {
                ui.add_space(6.0);
                well(
                    ui,
                    t,
                    RichText::new(&url).monospace().size(11.5).color(t.ink_dim),
                    |ui| {
                        if copy_button(ui, t).clicked() {
                            ui.ctx().copy_text(url.clone());
                            app.set_status(crate::i18n::tr("Endereço copiado"));
                        }
                    },
                );
                #[cfg(target_arch = "wasm32")]
                ui.label(
                    RichText::new(crate::i18n::tr(
                        "Quem tiver esse endereço edita este projeto: trate como senha.",
                    ))
                    .color(t.ink_faint)
                    .small(),
                );
                #[cfg(target_arch = "wasm32")]
                {
                    ui.add_space(6.0);
                    web_account(app, ui);
                }
            } else if !cfg!(target_arch = "wasm32") {
                ui.label(
                    RichText::new(crate::i18n::tr(
                        "O servidor está desligado (--no-server): reabra o aplicativo sem essa opção.",
                    ))
                    .color(t.ink_dim),
                );
            }
        });

        // ---- The tab, the line, and nothing else ----
        card(ui, t, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 6.0);
                for (i, client) in clients.iter().enumerate() {
                    let on = *chosen == i;
                    let tab = egui::Button::new(
                        RichText::new(client.label)
                            .size(13.5)
                            .color(if on { t.ink } else { t.ink_dim })
                            .strong(),
                    )
                    .fill(if on { t.inset } else { egui::Color32::TRANSPARENT })
                    .stroke(egui::Stroke::NONE)
                    .corner_radius(8)
                    .min_size(egui::vec2(0.0, 32.0));
                    if ui.add(tab).clicked() {
                        *chosen = i;
                    }
                }
            });
            let client = &clients[*chosen];
            ui.add_space(8.0);
            ui.label(RichText::new(client.place).color(t.ink_faint).small());
            ui.add_space(4.0);
            well(
                ui,
                t,
                RichText::new(&client.code).monospace().size(12.0),
                |ui| {
                    if copy_button(ui, t).clicked() {
                        ui.ctx().copy_text(client.code.clone());
                        app.set_status(crate::i18n::tr("Copiado"));
                    }
                    // Where the client has a command line and it is on this
                    // machine, nobody should have to open a terminal.
                    #[cfg(not(target_arch = "wasm32"))]
                    if let Some((program, args)) = &client.run
                        && installed(program)
                    {
                        let busy = app.ai_job.is_some();
                        let label = format!(
                            "{} {}",
                            icon::LIGHTNING,
                            if busy {
                                crate::i18n::tr("Registrando…")
                            } else {
                                crate::i18n::tr("Registrar agora")
                            }
                        );
                        let run = egui::Button::new(RichText::new(label).color(t.deep))
                            .fill(t.accent)
                            .corner_radius(8);
                        if ui
                            .add_enabled(!busy && serving, run)
                            .on_hover_text(crate::i18n::tr(
                                "Roda esse mesmo comando aqui, sem abrir o terminal.",
                            ))
                            .clicked()
                        {
                            register(app, program, args.clone());
                        }
                    }
                },
            );
            ui.label(RichText::new(client.note).color(t.ink_faint).small());
        });

        // ---- What it has done ----
        card(ui, t, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(crate::i18n::tr("Últimas chamadas"))
                        .size(13.5)
                        .strong()
                        .color(t.ink),
                );
                if pulse.calls > 0 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            crate::theme::fig(ui.visuals(), &pulse.calls.to_string())
                                .color(t.ink_faint),
                        );
                    });
                }
            });
            ui.add_space(2.0);
            if pulse.recent.is_empty() {
                ui.label(
                    RichText::new(crate::i18n::tr(
                        "Nada ainda. Assim que sua IA usar uma ferramenta, ela aparece aqui.",
                    ))
                    .color(t.ink_faint)
                    .small(),
                );
            } else {
                egui::ScrollArea::vertical()
                    .max_height(120.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for call in pulse.recent.iter().rev() {
                            let fresh = pulse.now_ms.saturating_sub(call.at_ms) < 2_000;
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(&call.tool)
                                        .monospace()
                                        .size(12.0)
                                        .color(if fresh { t.accent } else { t.ink }),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            RichText::new(ago(pulse.now_ms, call.at_ms))
                                                .color(t.ink_faint)
                                                .small(),
                                        );
                                        ui.label(
                                            RichText::new(&call.agent).color(t.ink_dim).small(),
                                        );
                                    },
                                );
                            });
                        }
                    });
            }
        });

        crate::theme::footer(ui, |ui| {
            if crate::theme::primary(ui, crate::i18n::tr("Fechar")).clicked() {
                close = true;
            }
            if !serving
                && cfg!(target_arch = "wasm32")
                && crate::theme::secondary(ui, crate::i18n::tr("Baixar o aplicativo")).clicked()
            {
                ui.ctx()
                    .open_url(egui::OpenUrl::new_tab("https://3dneweraai.com/#baixar"));
            }
        });
    });
    ctx.request_repaint_after(std::time::Duration::from_millis(250));
    close || ctx.input(|i| i.key_pressed(egui::Key::Escape))
}

/// A well: the sunken block a thing to be copied sits in, with its buttons on
/// their own line underneath — a long address must never run under them.
fn well(
    ui: &mut egui::Ui,
    t: crate::theme::Tokens,
    text: RichText,
    actions: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(t.inset)
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(text).wrap());
            ui.add_space(6.0);
            // A row of its own height: a right-to-left layout with nothing
            // bounding it takes every pixel the modal has, and the panel comes
            // out as one black field with a button in it.
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 30.0),
                egui::Layout::right_to_left(egui::Align::Center),
                actions,
            );
        });
}

/// The same copy button everywhere, so it is recognised rather than read.
fn copy_button(ui: &mut egui::Ui, t: crate::theme::Tokens) -> egui::Response {
    ui.add(
        egui::Button::new(
            RichText::new(format!("{} {}", icon::COPY, crate::i18n::tr("Copiar")))
                .size(12.5)
                .color(t.ink),
        )
        .fill(t.raised)
        .stroke(egui::Stroke::new(1.0, t.rule))
        .corner_radius(8),
    )
}

/// In a browser: the one button that decides whether this tab can be reached,
/// and what it says while it is deciding.
#[cfg(target_arch = "wasm32")]
fn web_switch(app: &mut NewEraApp, ui: &mut egui::Ui, serving: bool) {
    let t = crate::theme::of(ui.visuals());
    let state = app.ai_link.borrow().link.clone();
    match state {
        crate::ai_web::Link::Opening => {
            ui.label(RichText::new(crate::i18n::tr("Ligando…")).color(t.ink_dim));
        }
        crate::ai_web::Link::On { .. } => {
            if ui
                .button(crate::i18n::tr("Desligar"))
                .on_hover_text(crate::i18n::tr(
                    "Fecha o endereço. A sua IA perde o acesso na hora; o projeto continua aqui.",
                ))
                .clicked()
            {
                crate::ai_web::disconnect(&app.ai_link);
                app.set_status(crate::i18n::tr("MCP desligado"));
            }
        }
        crate::ai_web::Link::Off | crate::ai_web::Link::Failed { .. } => {
            let label = RichText::new(format!(
                "{}  {}",
                icon::LIGHTNING,
                crate::i18n::tr("Ligar o MCP nesta aba")
            ))
            .color(t.deep)
            .strong();
            if ui
                .add(
                    egui::Button::new(label)
                        .fill(t.accent)
                        .corner_radius(8)
                        .min_size(egui::vec2(0.0, 30.0)),
                )
                .clicked()
            {
                let (document, ctx, link) =
                    (app.document.clone(), ui.ctx().clone(), app.ai_link.clone());
                crate::ai_web::connect(document, ctx, link);
            }
        }
    }
    let _ = serving;
    if let crate::ai_web::Link::Failed { why } = app.ai_link.borrow().link.clone() {
        ui.label(
            RichText::new(crate::i18n::fill("Não deu para ligar: {}", &[&why]))
                .color(t.warn)
                .small(),
        );
    }
}

/// In a browser: the account this tab belongs to, or the way to sign in, so
/// Claude, `ChatGPT` and the rest reach it at one fixed address.
#[cfg(target_arch = "wasm32")]
fn web_account(app: &mut NewEraApp, ui: &mut egui::Ui) {
    let t = crate::theme::of(ui.visuals());
    let account = app.ai_link.borrow().account.clone();
    if matches!(app.ai_link.borrow().link, crate::ai_web::Link::On { .. }) {
        match account {
            crate::ai_web::Account::In { email, mcp_url } => {
                ui.label(
                    RichText::new(crate::i18n::fill("Conectado como {}", &[&email]))
                        .color(t.ink_dim)
                        .small(),
                );
                well(
                    ui,
                    t,
                    RichText::new(&mcp_url)
                        .monospace()
                        .size(11.5)
                        .color(t.ink_dim),
                    |ui| {
                        if copy_button(ui, t).clicked() {
                            ui.ctx().copy_text(mcp_url.clone());
                            app.set_status(crate::i18n::tr("Endereço copiado"));
                        }
                    },
                );
                ui.label(
                    RichText::new(crate::i18n::tr(
                        "Com este endereço, o Claude, o ChatGPT e outras IAs chegam nesta aba entrando com a sua conta.",
                    ))
                    .color(t.ink_dim)
                    .small(),
                );
            }
            crate::ai_web::Account::Out => {
                crate::ai_web::recheck_account(&app.ai_link, ui.ctx());
                if ui
                    .button(crate::i18n::tr("Entrar para usar no Claude e no ChatGPT"))
                    .on_hover_text(crate::i18n::tr(
                        "Com uma conta, a sua IA chega nesta aba por um endereço fixo, sem colar nada.",
                    ))
                    .clicked()
                {
                    crate::ai_web::sign_in(&app.ai_link);
                }
            }
            crate::ai_web::Account::Unknown => {}
        }
    }
}

#[cfg(test)]
mod tests {
    /// The snippet each client gets carries the address of this window, so
    /// pasting it connects to this window and not to a guess.
    #[test]
    fn every_client_gets_this_window_address() {
        let clients = super::clients("http://127.0.0.1:7999/mcp");
        assert!(clients.len() >= 6);
        for client in clients {
            assert!(
                client.code.contains("http://127.0.0.1:7999/mcp"),
                "{} lost the address: {}",
                client.label,
                client.code
            );
            assert_ne!(client.place, "");
            assert_ne!(client.note, "");
        }
    }

    /// The clients with a command line carry it ready to run, so the window
    /// can do the registering instead of asking for a terminal.
    #[test]
    fn the_command_line_clients_can_be_registered_from_here() {
        let clients = super::clients("http://127.0.0.1:7999/mcp");
        let runnable: Vec<&str> = clients
            .iter()
            .filter(|c| c.run.is_some())
            .map(|c| c.label)
            .collect();
        assert!(
            runnable.contains(&"Claude Code") && runnable.contains(&"Codex"),
            "expected the CLIs among {runnable:?}"
        );
        for client in clients.iter().filter_map(|c| c.run.as_ref()) {
            assert!(
                client.1.iter().any(|a| a.contains("127.0.0.1:7999")),
                "the command must carry this window's address"
            );
        }
    }

    /// Whatever else it finds, it finds the shell's own tools — and does not
    /// invent a program nobody has.
    #[cfg(unix)]
    #[test]
    fn it_can_tell_what_is_installed() {
        assert!(super::installed("sh"));
        assert!(!super::installed("newera-not-a-real-program"));
    }

    /// The moment reads as a person would say it.
    #[test]
    fn how_long_ago_in_words() {
        let now = 10_000_000;
        assert_eq!(super::ago(now, now), "agora");
        assert_eq!(super::ago(now, now - 30_000), "há 30 s");
        assert_eq!(super::ago(now, now - 300_000), "há 5 min");
        assert_eq!(super::ago(now, now - 7_200_000), "há 2 h");
    }
}
