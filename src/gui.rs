use crate::{
    client::Client,
    engine::{Action, Job, Status},
    peers::{Connection, Host},
};
use adw::{gio, gtk, prelude::*};
use std::{
    cell::Cell,
    rc::Rc,
    sync::mpsc::{self, Sender},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Language(&'static str);

const LANGUAGE_OPTIONS: [(&str, &str); 13] = [
    ("de", "Deutsch"),
    ("en", "English"),
    ("es", "Español"),
    ("fr", "Français"),
    ("it", "Italiano"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("pl", "Polski"),
    ("pt", "Português"),
    ("ru", "Русский"),
    ("tr", "Türkçe"),
    ("uk", "Українська"),
    ("zh", "简体中文"),
];

impl Language {
    fn system() -> Self {
        language_from_names(adw::glib::language_names().iter().map(|s| s.as_str()))
    }

    fn text(self, key: &'static str) -> &'static str {
        crate::i18n::text(self.0, key)
    }
}

fn language_from_names<'a>(names: impl IntoIterator<Item = &'a str>) -> Language {
    for name in names {
        let code = name.split(['_', '-', '.', '@']).next().unwrap_or_default();
        if let Some((supported, _)) = crate::i18n::LOCALES
            .iter()
            .find(|(supported, _)| supported.eq_ignore_ascii_case(code))
        {
            return Language(supported);
        }
    }
    Language("en")
}

enum Request {
    Refresh(Option<String>),
    SelectHost(String),
    Start(Action),
    CheckConnection(String),
    RemoveConnection(String),
    MintPairing,
    AcceptPairing {
        link: String,
        origin: Option<String>,
        allow_public: bool,
    },
    SetPairing(bool),
}
struct Snapshot {
    hosts: Vec<Host>,
    connections: Vec<Connection>,
    selected_host: String,
    status: Status,
    jobs: Vec<Job>,
    selected: Option<String>,
    log: Option<String>,
    connection_health: std::collections::HashMap<String, bool>,
    pairing_enabled: bool,
}

enum Update {
    Snapshot(Box<Snapshot>),
    Error(String),
    Notice(Notice),
}

enum Notice {
    ConnectionRemoved,
    PairingCode { code: String, link: String },
    PairingPaired,
    PairingFailed(String),
    Error(String),
}

fn worker(client: Client, requests: mpsc::Receiver<Request>, updates: mpsc::Sender<Update>) {
    let mut selected = None;
    let mut selected_host = "local".to_owned();
    let mut connection_health = std::collections::HashMap::new();
    loop {
        let mut notice = None;
        let request = requests.recv_timeout(Duration::from_millis(1500));
        match request {
            Ok(Request::Start(action)) => match client.start_for(&selected_host, action, true) {
                Ok(job) => selected = Some(job.id),
                Err(error) => {
                    let _ = updates.send(Update::Error(error.to_string()));
                    continue;
                }
            },
            Ok(Request::Refresh(id)) => {
                if id.is_some() {
                    selected = id;
                }
            }
            Ok(Request::SelectHost(host)) => {
                if host != selected_host {
                    selected_host = host;
                    selected = None;
                }
            }
            Ok(Request::CheckConnection(id)) => {
                let result = client.post_api(&format!("connections/{id}/check"), &serde_json::json!({}));
                connection_health.insert(id, result.is_ok());
                if let Err(error) = result {
                    notice = Some(Notice::Error(error.to_string()));
                }
            }
            Ok(Request::RemoveConnection(id)) => {
                match client.post_api(&format!("connections/{id}/remove"), &serde_json::json!({})) {
                    Ok(_) => notice = Some(Notice::ConnectionRemoved),
                    Err(error) => notice = Some(Notice::Error(error.to_string())),
                }
            }
            Ok(Request::MintPairing) => {
                match client.post_api("pairings", &serde_json::json!({})) {
                    Ok(invitation) => {
                        let code = invitation.get("code").and_then(serde_json::Value::as_str).unwrap_or_default();
                        let link = invitation.get("link").and_then(serde_json::Value::as_str).unwrap_or_default();
                        notice = Some(Notice::PairingCode {
                            code: code.to_owned(),
                            link: link.to_owned(),
                        });
                    }
                    Err(error) => notice = Some(Notice::Error(error.to_string())),
                }
            }
            Ok(Request::AcceptPairing {
                link,
                origin,
                allow_public,
            }) => {
                match client.post_api(
                    "pairings/accept",
                    &serde_json::json!({"link": link, "origin": origin, "label": null, "allow_public": allow_public}),
                ) {
                    Ok(_) => notice = Some(Notice::PairingPaired),
                    Err(error) => notice = Some(Notice::PairingFailed(error.to_string())),
                }
            }
            Ok(Request::SetPairing(enabled)) => {
                match client.post_api(
                    "pairings/settings",
                    &serde_json::json!({"enabled": enabled}),
                ) {
                    Ok(_) => {}
                    Err(error) => notice = Some(Notice::Error(error.to_string())),
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(notice) = notice {
            if updates.send(Update::Notice(notice)).is_err() {
                return;
            }
        }
        let result = (|| -> anyhow::Result<Snapshot> {
            let hosts = client.hosts()?;
            let connections = client.connections()?;
            let pairing_enabled = client
                .get_api("pairings")?
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            if !hosts.iter().any(|host| host.id == selected_host) {
                selected_host = "local".to_owned();
                selected = None;
            }
            let status = client.status_for(&selected_host)?;
            let jobs = client.jobs_for(&selected_host)?;
            if !jobs.iter().any(|job| Some(&job.id) == selected.as_ref()) {
                selected = jobs.first().map(|job| job.id.clone());
            }
            let log = if let Some(id) = &selected {
                let mut log = client.log_for(&selected_host, id)?;
                if let Some(error) = jobs
                    .iter()
                    .find(|j| &j.id == id)
                    .and_then(|j| j.error.as_ref())
                {
                    log.push_str(&format!("\n{error}"));
                }
                Some(log)
            } else {
                None
            };
            Ok(Snapshot {
                hosts,
                connections,
                selected_host: selected_host.clone(),
                status,
                jobs,
                selected: selected.clone(),
                log,
                connection_health: connection_health.clone(),
                pairing_enabled,
            })
        })();
        let update = match result {
            Ok(snapshot) => Update::Snapshot(Box::new(snapshot)),
            Err(error) => Update::Error(error.to_string()),
        };
        if updates.send(update).is_err() {
            return;
        }
    }
}

fn label(language: Language, action: Action) -> &'static str {
    match action {
        Action::Check => language.text("check"),
        Action::Build => language.text("build"),
        Action::Diff => language.text("diff"),
        Action::Switch => language.text("switch"),
        Action::UpdateInputs => language.text("updateInputs"),
        Action::Rollback => language.text("rollback"),
        Action::Clean => language.text("clean"),
    }
}
fn status_label(language: Language, status: &str) -> &str {
    match status {
        "running" => language.text("running"),
        "succeeded" => language.text("succeeded"),
        "failed" => language.text("failed"),
        "interrupted" => language.text("interrupted"),
        other => other,
    }
}

fn error_message(error: &str) -> String {
    #[derive(serde::Deserialize)]
    struct ApiError {
        error: String,
    }

    let trimmed = error.trim();
    serde_json::from_str::<ApiError>(trimmed)
        .map(|body| body.error)
        .unwrap_or_else(|_| trimmed.to_owned())
}

fn format_timestamp(language: Language, timestamp: Option<u64>) -> adw::glib::GString {
    timestamp
        .and_then(|seconds| i64::try_from(seconds).ok())
        .and_then(|seconds| adw::glib::DateTime::from_unix_local(seconds).ok())
        .and_then(|date| date.format("%Y-%m-%d %H:%M:%S").ok())
        .unwrap_or_else(|| language.text("unavailable").into())
}

fn description(language: Language, action: Action) -> &'static str {
    match action {
        Action::Check => language.text("checkDescription"),
        Action::Build => language.text("buildDescription"),
        Action::Diff => language.text("diffDescription"),
        Action::Switch => language.text("switchDescription"),
        _ => label(language, action),
    }
}
#[cfg(test)]
fn launch(
    window: &adw::ApplicationWindow,
    sender: &Sender<Request>,
    action: Action,
    language: Language,
) {
    launch_for_host(window, sender, action, language, None);
}

fn launch_for_host(
    window: &adw::ApplicationWindow,
    sender: &Sender<Request>,
    action: Action,
    language: Language,
    host_label: Option<String>,
) {
    if !action.privileged() && action != Action::UpdateInputs {
        let _ = sender.send(Request::Start(action));
        return;
    }
    let explanation = match action {
        Action::Switch => language.text("confirmSwitch"),
        Action::Rollback => language.text("confirmRollback"),
        Action::Clean => language.text("confirmClean"),
        _ => language.text("confirmUpdate"),
    };
    let body = host_label
        .map(|host| format!("{}: {host}\n\n{explanation}", language.text("targetHost")))
        .unwrap_or_else(|| explanation.to_owned());
    let dialog = adw::AlertDialog::builder()
        .heading(label(language, action))
        .body(&body)
        .default_response("cancel")
        .close_response("cancel")
        .build();
    dialog.add_responses(&[
        ("cancel", language.text("cancel")),
        ("run", language.text("continue")),
    ]);
    dialog.set_response_appearance(
        "run",
        if action == Action::Clean {
            adw::ResponseAppearance::Destructive
        } else {
            adw::ResponseAppearance::Suggested
        },
    );
    let sender = sender.clone();
    dialog.connect_response(None, move |_, response| {
        if response == "run" {
            let _ = sender.send(Request::Start(action));
        }
    });
    dialog.present(Some(window));
}

const TABS: [(&str, &str, &str); 3] = [
    ("system", "computer-symbolic", "<Primary>1"),
    ("history", "document-open-recent-symbolic", "<Primary>2"),
    ("connections", "network-workgroup-symbolic", "<Primary>3"),
];
const SETTINGS_ACCELERATOR: &str = "<Primary>comma";

fn tab_content() -> gtk::Box {
    gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build()
}

fn icon_button(icon_name: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon_name)
        .tooltip_text(tooltip)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build()
}

fn build_window(
    app: &adw::Application,
    client: Client,
    initial_language: Language,
) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Yonix")
        .default_width(980)
        .default_height(840)
        .build();
    let toolbar = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    let stack = adw::ViewStack::new();
    let view_switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();
    header.set_title_widget(Some(&view_switcher));
    let narrow_switcher = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 700px").expect("valid width breakpoint"),
    );
    narrow_switcher.add_setter(
        &view_switcher,
        "policy",
        Some(&adw::ViewSwitcherPolicy::Narrow.to_value()),
    );
    window.add_breakpoint(narrow_switcher);
    let bottom_switcher = adw::ViewSwitcherBar::builder().stack(&stack).build();
    toolbar.add_bottom_bar(&bottom_switcher);
    let phone_title = adw::WindowTitle::new("Yonix", "");
    let phone_breakpoint = adw::Breakpoint::new(
        adw::BreakpointCondition::parse("max-width: 500px").expect("valid width breakpoint"),
    );
    phone_breakpoint.add_setter(&header, "title-widget", Some(&phone_title.to_value()));
    phone_breakpoint.add_setter(&bottom_switcher, "reveal", Some(&true.to_value()));
    window.add_breakpoint(phone_breakpoint);
    let (send, receive) = mpsc::channel();
    let (update_send, updates) = mpsc::channel();
    let host_selector = gtk::ComboBoxText::new();
    host_selector.set_tooltip_text(Some(initial_language.text("managedHost")));
    host_selector.set_valign(gtk::Align::Center);
    for cell in host_selector.cells() {
        if let Ok(text) = cell.downcast::<gtk::CellRendererText>() {
            text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.set_max_width_chars(16);
        }
    }
    let updating_hosts = Rc::new(Cell::new(false));
    let updating_hosts_for_callback = updating_hosts.clone();
    let selector_sender = send.clone();
    host_selector.connect_changed(move |selector| {
        if updating_hosts_for_callback.get() {
            return;
        }
        if let Some(host) = selector.active_id() {
            let _ = selector_sender.send(Request::SelectHost(host.to_string()));
        }
    });
    header.pack_start(&host_selector);
    let menu = gio::Menu::new();
    menu.append(
        Some(initial_language.text("settings")),
        Some("win.settings"),
    );
    menu.append(
        Some(initial_language.text("shortcuts")),
        Some("win.shortcuts"),
    );
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .tooltip_text(initial_language.text("menu"))
        .menu_model(&menu)
        .build();
    header.pack_end(&menu_button);
    for (name, _, accelerator) in TABS {
        let action = gio::SimpleAction::new(name, None);
        let stack_for_action = stack.downgrade();
        action.connect_activate(move |_, _| {
            if let Some(stack) = stack_for_action.upgrade() {
                stack.set_visible_child_name(name);
            }
        });
        window.add_action(&action);
        app.set_accels_for_action(&format!("win.{name}"), &[accelerator]);
    }
    let shortcuts_action = gio::SimpleAction::new("shortcuts", None);
    let weak_window_for_shortcuts = window.downgrade();
    shortcuts_action.connect_activate(move |_, _| {
        if let Some(window) = weak_window_for_shortcuts.upgrade() {
            let group = adw::PreferencesGroup::new();
            for (name, accelerator) in TABS
                .into_iter()
                .map(|(name, _, accelerator)| (name, accelerator))
                .chain([("settings", SETTINGS_ACCELERATOR)])
            {
                let row = adw::ActionRow::builder()
                    .title(initial_language.text(name))
                    .build();
                row.add_suffix(
                    &gtk::ShortcutLabel::builder()
                        .accelerator(accelerator)
                        .valign(gtk::Align::Center)
                        .build(),
                );
                group.add(&row);
            }
            let page = adw::PreferencesPage::new();
            page.add(&group);
            let dialog = adw::PreferencesDialog::builder()
                .title(initial_language.text("shortcuts"))
                .search_enabled(false)
                .build();
            dialog.add(&page);
            dialog.present(Some(&window));
        }
    });
    window.add_action(&shortcuts_action);
    toolbar.add_top_bar(&header);
    let toast = adw::ToastOverlay::new();
    let content = tab_content();
    let subtitle = gtk::Label::builder()
        .label(initial_language.text("connecting"))
        .xalign(0.0)
        .wrap(true)
        .build();
    subtitle.add_css_class("dim-label");
    content.append(&subtitle);
    let system = adw::PreferencesGroup::new();
    let host = adw::ActionRow::builder()
        .title(initial_language.text("hostLabel"))
        .subtitle(initial_language.text("connectingShort"))
        .build();
    host.add_prefix(&gtk::Image::from_icon_name("computer-symbolic"));
    system.add(&host);
    let source = adw::ActionRow::builder()
        .title(initial_language.text("configuration"))
        .subtitle_selectable(true)
        .build();
    source.add_prefix(&gtk::Image::from_icon_name("folder-symbolic"));
    system.add(&source);
    let current = adw::ActionRow::builder()
        .title(initial_language.text("currentGeneration"))
        .subtitle_lines(2)
        .subtitle_selectable(true)
        .build();
    system.add(&current);
    let version = adw::ActionRow::builder()
        .title(initial_language.text("version"))
        .subtitle_selectable(true)
        .build();
    system.add(&version);
    content.append(&system);
    let actions_group = adw::PreferencesGroup::builder()
        .title(initial_language.text("whatToDo"))
        .description(initial_language.text("checkingSafe"))
        .build();
    let flow = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(2)
        .max_children_per_line(4)
        .column_spacing(8)
        .row_spacing(8)
        .homogeneous(true)
        .build();
    let language_client = client.clone();
    std::thread::spawn(move || worker(client, receive, update_send));
    let mut buttons = vec![];
    let host_selector_for_actions = host_selector.clone();
    for (action, icon) in [
        (Action::Check, "emblem-ok-symbolic"),
        (Action::Build, "system-run-symbolic"),
        (Action::Diff, "view-dual-symbolic"),
        (Action::Switch, "software-update-available-symbolic"),
    ] {
        let button = gtk::Button::builder()
            .tooltip_text(description(initial_language, action))
            .height_request(82)
            .hexpand(true)
            .build();
        let button_content = adw::ButtonContent::builder()
            .icon_name(icon)
            .label(label(initial_language, action))
            .build();
        button.set_child(Some(&button_content));
        if action == Action::Switch {
            button.add_css_class("suggested-action");
        }
        let weak_window = window.downgrade();
        let sender = send.clone();
        let host_selector = host_selector_for_actions.clone();
        button.connect_clicked(move |_| {
            if let Some(window) = weak_window.upgrade() {
                let host_id = host_selector
                    .active_id()
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "local".to_owned());
                let host_label = host_selector.active_text().map(|text| text.to_string());
                let _ = sender.send(Request::SelectHost(host_id));
                launch_for_host(&window, &sender, action, initial_language, host_label);
            }
        });
        flow.insert(&button, -1);
        button.set_sensitive(false);
        buttons.push((action, button));
    }
    actions_group.add(&flow);
    content.append(&actions_group);
    let maintenance = adw::PreferencesGroup::builder()
        .title(initial_language.text("maintenance"))
        .build();
    for action in [Action::UpdateInputs, Action::Rollback, Action::Clean] {
        let row = adw::ActionRow::builder()
            .title(label(initial_language, action))
            .build();
        let button = gtk::Button::builder()
            .icon_name("go-next-symbolic")
            .valign(gtk::Align::Center)
            .tooltip_text(label(initial_language, action))
            .build();
        button.add_css_class("flat");
        row.add_suffix(&button);
        row.set_activatable_widget(Some(&button));
        let weak_window = window.downgrade();
        let sender = send.clone();
        let host_selector = host_selector_for_actions.clone();
        button.connect_clicked(move |_| {
            if let Some(window) = weak_window.upgrade() {
                let host_id = host_selector
                    .active_id()
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "local".to_owned());
                let host_label = host_selector.active_text().map(|text| text.to_string());
                let _ = sender.send(Request::SelectHost(host_id));
                launch_for_host(&window, &sender, action, initial_language, host_label);
            }
        });
        maintenance.add(&row);
        button.set_sensitive(false);
        buttons.push((action, button));
    }
    content.append(&maintenance);
    let history = adw::PreferencesGroup::builder()
        .title(initial_language.text("operations"))
        .build();
    let history_box = gtk::ListBox::new();
    history_box.set_selection_mode(gtk::SelectionMode::Single);
    history_box.set_activate_on_single_click(true);
    history_box.add_css_class("boxed-list");
    let operations_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .child(&history_box)
        .build();
    history.add(&operations_scroll);
    let history_content = tab_content();
    history_content.append(&history);
    let operation_details = adw::PreferencesGroup::new();
    let operation_status = adw::ActionRow::builder()
        .title(initial_language.text("operationStatus"))
        .subtitle(initial_language.text("logHint"))
        .subtitle_selectable(true)
        .build();
    let operation_started = adw::ActionRow::builder()
        .title(initial_language.text("operationStarted"))
        .subtitle(initial_language.text("unavailable"))
        .subtitle_selectable(true)
        .build();
    let operation_finished = adw::ActionRow::builder()
        .title(initial_language.text("operationFinished"))
        .subtitle(initial_language.text("unavailable"))
        .subtitle_selectable(true)
        .build();
    let operation_id = adw::ActionRow::builder()
        .title(initial_language.text("operationId"))
        .subtitle(initial_language.text("unavailable"))
        .subtitle_selectable(true)
        .build();
    for row in [
        &operation_status,
        &operation_started,
        &operation_finished,
        &operation_id,
    ] {
        operation_details.add(row);
    }
    let log_group = adw::PreferencesGroup::builder()
        .title(initial_language.text("operationLog"))
        .description(initial_language.text("fullLogs"))
        .build();
    let log = gtk::TextView::builder()
        .editable(false)
        .cursor_visible(false)
        .monospace(true)
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(16)
        .bottom_margin(16)
        .left_margin(16)
        .right_margin(16)
        .build();
    let log_buffer = log.buffer();
    log_buffer.set_text(initial_language.text("noOperations"));
    let log_end = log_buffer.create_mark(None, &log_buffer.end_iter(), false);
    let log_end_on_map = log_end.clone();
    log.connect_map(move |log| log.scroll_to_mark(&log_end_on_map, 0.0, true, 0.0, 1.0));
    let log_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(220)
        .max_content_height(380)
        .vexpand(true)
        .propagate_natural_height(true)
        .child(&log)
        .build();
    log_scroll.add_css_class("card");
    log_group.add(&log_scroll);
    history_content.append(&log_group);
    history_content.append(&operation_details);

    let connected_group = adw::PreferencesGroup::builder()
        .title(initial_language.text("connectedHosts"))
        .build();
    let local_connection = adw::ActionRow::builder()
        .title(initial_language.text("localHost"))
        .subtitle(initial_language.text("connectingShort"))
        .use_markup(false)
        .build();
    local_connection.add_prefix(&gtk::Image::from_icon_name("computer-symbolic"));
    connected_group.add(&local_connection);
    let no_connections = adw::ActionRow::builder()
        .title(initial_language.text("noConnections"))
        .visible(false)
        .build();
    no_connections.add_css_class("dim-label");
    connected_group.add(&no_connections);

    let pairing_group = adw::PreferencesGroup::new();
    let pairing_switch = adw::SwitchRow::builder()
        .title(initial_language.text("pairingAllow"))
        .sensitive(false)
        .build();
    pairing_group.add(&pairing_switch);
    let share_pairing = icon_button("list-add-symbolic", initial_language.text("addConnection"));
    share_pairing.set_sensitive(false);
    let share_row = adw::ActionRow::builder()
        .title(initial_language.text("addConnection"))
        .subtitle(initial_language.text("addConnectionHelp"))
        .activatable_widget(&share_pairing)
        .build();
    share_row.add_suffix(&share_pairing);
    pairing_group.add(&share_row);

    let pairing_code = adw::ActionRow::builder()
        .title(initial_language.text("pairingCode"))
        .subtitle_selectable(true)
        .use_markup(false)
        .visible(false)
        .build();
    let copy_code = icon_button(
        "edit-copy-symbolic",
        initial_language.text("copyPairingCode"),
    );
    pairing_code.add_suffix(&copy_code);
    pairing_group.add(&pairing_code);
    let pairing_link = adw::ActionRow::builder()
        .title(initial_language.text("pairingLink"))
        .subtitle_lines(1)
        .subtitle_selectable(true)
        .use_markup(false)
        .visible(false)
        .build();
    let copy_link = icon_button(
        "edit-copy-symbolic",
        initial_language.text("copyPairingLink"),
    );
    pairing_link.add_suffix(&copy_link);
    pairing_group.add(&pairing_link);

    let join_group = adw::PreferencesGroup::builder()
        .title(initial_language.text("addOtherInstance"))
        .sensitive(false)
        .build();
    let pairing_entry = adw::EntryRow::builder()
        .title(initial_language.text("pairingLink"))
        .input_purpose(gtk::InputPurpose::Url)
        .tooltip_text(initial_language.text("pairingLinkPlaceholder"))
        .build();
    join_group.add(&pairing_entry);
    let advanced = adw::ExpanderRow::builder()
        .title(initial_language.text("advanced"))
        .build();
    let pairing_origin = adw::EntryRow::builder()
        .title(initial_language.text("pairingOrigin"))
        .input_purpose(gtk::InputPurpose::Url)
        .build();
    advanced.add_row(&pairing_origin);
    let allow_public = adw::SwitchRow::builder()
        .title(initial_language.text("pairingPublicConfirm"))
        .build();
    advanced.add_row(&allow_public);
    join_group.add(&advanced);
    let pairing_error = adw::ActionRow::builder()
        .title(initial_language.text("pairingFailed"))
        .subtitle_selectable(true)
        .use_markup(false)
        .visible(false)
        .build();
    let error_icon = gtk::Image::from_icon_name("dialog-error-symbolic");
    error_icon.add_css_class("error");
    pairing_error.add_prefix(&error_icon);
    join_group.add(&pairing_error);
    let accept_pairing = gtk::Button::builder()
        .label(initial_language.text("pairingAccept"))
        .halign(gtk::Align::End)
        .margin_top(12)
        .sensitive(false)
        .build();
    accept_pairing.add_css_class("suggested-action");
    join_group.add(&accept_pairing);
    let connections_content = tab_content();
    connections_content.append(&connected_group);
    connections_content.append(&pairing_group);
    connections_content.append(&join_group);
    let settings_group = adw::PreferencesGroup::builder()
        .title(initial_language.text("session"))
        .build();
    let languages = gtk::StringList::new(&LANGUAGE_OPTIONS.map(|(_, name)| name));
    let selected_language = LANGUAGE_OPTIONS
        .iter()
        .position(|(code, _)| *code == initial_language.0)
        .expect("known interface language");
    let language_row = adw::ComboRow::builder()
        .title(initial_language.text("language"))
        .model(&languages)
        .selected(selected_language as u32)
        .build();
    settings_group.add(&language_row);
    let language_app = app.clone();
    let weak_window_for_language = window.downgrade();
    let language_client_for_switch = language_client.clone();
    language_row.connect_selected_notify(move |row| {
        let Some(&(locale, _)) = LANGUAGE_OPTIONS.get(row.selected() as usize) else {
            return;
        };
        let Some(window) = weak_window_for_language.upgrade() else {
            return;
        };
        if locale == initial_language.0 {
            return;
        }
        build_window(
            &language_app,
            language_client_for_switch.clone(),
            Language(locale),
        )
        .present();
        if let Some(dialog) = window.visible_dialog() {
            dialog.force_close();
        }
        window.close();
    });
    let disconnect = gtk::Button::builder()
        .label(initial_language.text("logout"))
        .halign(gtk::Align::Center)
        .css_classes(["destructive-action"])
        .build();
    let weak_window_for_disconnect = window.downgrade();
    let login_app = app.clone();
    let login_client = language_client.clone();
    disconnect.connect_clicked(move |_| {
        if let Some(window) = weak_window_for_disconnect.upgrade() {
            let login = build_login_window(&login_app, login_client.clone(), initial_language);
            login.present();
            if let Some(dialog) = window.visible_dialog() {
                dialog.force_close();
            }
            window.close();
        }
    });
    let cache_url = adw::ActionRow::builder()
        .title(initial_language.text("binaryCache"))
        .subtitle(initial_language.text("unavailable"))
        .subtitle_selectable(true)
        .use_markup(false)
        .build();
    settings_group.add(&cache_url);
    let cache_key = adw::ActionRow::builder()
        .title(initial_language.text("cachePublicKey"))
        .subtitle(initial_language.text("unavailable"))
        .subtitle_selectable(true)
        .use_markup(false)
        .build();
    settings_group.add(&cache_key);
    let settings_page = adw::PreferencesPage::new();
    settings_page.add(&settings_group);
    let session_actions = adw::PreferencesGroup::new();
    session_actions.add(&disconnect);
    settings_page.add(&session_actions);
    let settings = adw::PreferencesDialog::builder()
        .title(initial_language.text("settings"))
        .search_enabled(false)
        .build();
    settings.add(&settings_page);
    let settings_action = gio::SimpleAction::new("settings", None);
    let weak_window_for_settings = window.downgrade();
    settings_action.connect_activate(move |_, _| {
        if let Some(window) = weak_window_for_settings.upgrade() {
            settings.present(Some(&window));
        }
    });
    window.add_action(&settings_action);
    app.set_accels_for_action("win.settings", &[SETTINGS_ACCELERATOR]);

    let updating_pairing = Rc::new(Cell::new(false));
    let updating_pairing_for_callback = updating_pairing.clone();
    let sender = send.clone();
    pairing_switch.connect_active_notify(move |row| {
        if updating_pairing_for_callback.get() {
            return;
        }
        let _ = sender.send(Request::SetPairing(row.is_active()));
    });
    let sender = send.clone();
    share_pairing.connect_clicked(move |_| {
        let _ = sender.send(Request::MintPairing);
    });
    for (button, row) in [(copy_code, &pairing_code), (copy_link, &pairing_link)] {
        let row = row.downgrade();
        button.connect_clicked(move |button| {
            if let Some(value) = row.upgrade().and_then(|r| r.subtitle()) {
                button.clipboard().set_text(&value);
            }
        });
    }
    let sender = send.clone();
    let pairing_entry_for_accept = pairing_entry.downgrade();
    let pairing_origin_for_accept = pairing_origin.clone();
    let allow_public_for_accept = allow_public.clone();
    let pairing_error_for_accept = pairing_error.downgrade();
    accept_pairing.connect_clicked(move |_| {
        let Some(entry) = pairing_entry_for_accept.upgrade() else {
            return;
        };
        if let Some(row) = pairing_error_for_accept.upgrade() {
            row.set_visible(false);
        }
        let origin = pairing_origin_for_accept.text().trim().to_owned();
        let _ = sender.send(Request::AcceptPairing {
            link: entry.text().trim().to_owned(),
            origin: (!origin.is_empty()).then_some(origin),
            allow_public: allow_public_for_accept.is_active(),
        });
    });
    let accept_from_input = accept_pairing.clone();
    pairing_entry.connect_changed(move |entry| {
        accept_from_input.set_sensitive(!entry.text().trim().is_empty());
    });
    let accept_from_entry = accept_pairing.clone();
    pairing_entry.connect_entry_activated(move |entry| {
        if entry.is_sensitive() && !entry.text().trim().is_empty() {
            accept_from_entry.emit_clicked();
        }
    });
    for ((name, icon, _), page_content) in
        TABS.into_iter()
            .zip([&content, &history_content, &connections_content])
    {
        let clamp = adw::Clamp::builder()
            .maximum_size(1000)
            .child(page_content)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&clamp)
            .build();
        stack.add_titled_with_icon(&scroll, Some(name), initial_language.text(name), icon);
    }
    toast.set_child(Some(&stack));
    toolbar.set_content(Some(&toast));
    window.set_content(Some(&toolbar));

    let _ = send.send(Request::Refresh(None));
    let weak_window = window.downgrade();
    let mut last_history = String::new();
    let mut last_selection = None;
    let mut last_log = initial_language.text("noOperations").to_owned();
    let mut last_error = String::new();
    let mut last_hosts = String::new();
    let mut last_connections = String::new();
    let mut connection_rows = Vec::new();
    let mut last_notice = String::new();
    adw::glib::timeout_add_local(Duration::from_millis(150), move || {
        if weak_window.upgrade().is_none() {
            return adw::glib::ControlFlow::Break;
        }
        while let Ok(update) = updates.try_recv() {
            let lang = initial_language;
            let show_toast = |message: &str| {
                toast.add_toast(
                    adw::Toast::builder()
                        .title(message)
                        .use_markup(false)
                        .build(),
                );
            };
            match update {
                Update::Error(error) => {
                    let message = error_message(&error);
                    subtitle.set_text(lang.text("connectionError"));
                    cache_url.set_subtitle(lang.text("unavailable"));
                    cache_key.set_subtitle(lang.text("unavailable"));
                    for (_, button) in &buttons {
                        button.set_sensitive(false);
                    }
                    if message != last_error {
                        show_toast(&message);
                        last_error = message;
                    }
                }
                Update::Notice(notice) => {
                    let notice_text = match notice {
                        Notice::ConnectionRemoved => Some(lang.text("removeConnection").to_owned()),
                        Notice::PairingPaired => {
                            pairing_error.set_visible(false);
                            Some(lang.text("pairingPaired").to_owned())
                        }
                        Notice::PairingCode { code, link } => {
                            pairing_code.set_subtitle(&code);
                            pairing_code.set_visible(true);
                            pairing_link.set_subtitle(&link);
                            pairing_link.set_visible(true);
                            None
                        }
                        Notice::PairingFailed(error) => {
                            pairing_error.set_subtitle(&error_message(&error));
                            pairing_error.set_visible(true);
                            None
                        }
                        Notice::Error(error) => Some(error_message(&error)),
                    };
                    if let Some(notice_text) = notice_text {
                        if notice_text != last_notice {
                            show_toast(&notice_text);
                            last_notice = notice_text;
                        }
                    }
                }
                Update::Snapshot(snapshot) => {
                    last_error.clear();
                    if pairing_switch.is_active() != snapshot.pairing_enabled {
                        updating_pairing.set(true);
                        pairing_switch.set_active(snapshot.pairing_enabled);
                        updating_pairing.set(false);
                    }
                    share_pairing.set_sensitive(snapshot.pairing_enabled);
                    pairing_switch.set_sensitive(true);
                    join_group.set_sensitive(true);
                    if !snapshot.pairing_enabled {
                        for row in [&pairing_code, &pairing_link] {
                            row.set_visible(false);
                            row.set_subtitle("");
                        }
                    }
                    let hosts_key = snapshot
                        .hosts
                        .iter()
                        .map(|host| format!("{}\u{1f}{}", host.id, host.label))
                        .collect::<Vec<_>>()
                        .join("\u{1e}");
                    if hosts_key != last_hosts {
                        updating_hosts.set(true);
                        host_selector.remove_all();
                        for host_entry in &snapshot.hosts {
                            host_selector.append(Some(&host_entry.id), &host_entry.label);
                        }
                        host_selector.set_active_id(Some(&snapshot.selected_host));
                        updating_hosts.set(false);
                        last_hosts = hosts_key;
                    } else if host_selector.active_id().as_deref()
                        != Some(snapshot.selected_host.as_str())
                    {
                        updating_hosts.set(true);
                        host_selector.set_active_id(Some(&snapshot.selected_host));
                        updating_hosts.set(false);
                    }
                    host.set_subtitle(&format!(
                        "{} · {}",
                        snapshot.status.host,
                        match snapshot.status.config_kind {
                            crate::config::ConfigKind::Flake => lang.text("envFlake"),
                            crate::config::ConfigKind::Legacy => lang.text("envLegacy"),
                            crate::config::ConfigKind::Unknown =>
                                lang.text(if snapshot.status.config_error.is_some() {
                                    "configurationInaccessible"
                                } else {
                                    "missingConfiguration"
                                }),
                        }
                    ));
                    if let Some(gateway) = snapshot.hosts.iter().find(|host| host.local) {
                        local_connection.set_subtitle(&gateway.label);
                    }
                    source.set_subtitle(&snapshot.status.config_path.to_string_lossy());
                    current.set_subtitle(
                        &snapshot
                            .status
                            .current_system
                            .as_ref()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|| lang.text("unavailable").into()),
                    );
                    version.set_subtitle(&format!("v{}", snapshot.status.version));
                    subtitle.set_text(if snapshot.status.active.is_some() {
                        lang.text("desktopWorking")
                    } else {
                        lang.text("desktopReady")
                    });
                    if let Some(cache) = &snapshot.status.cache {
                        cache_url.set_subtitle(&cache.url);
                        cache_key.set_subtitle(
                            cache
                                .public_key
                                .as_deref()
                                .map(str::trim)
                                .filter(|key| !key.is_empty())
                                .unwrap_or(lang.text("unavailable")),
                        );
                    } else {
                        cache_url.set_subtitle(lang.text("unavailable"));
                        cache_key.set_subtitle(lang.text("unavailable"));
                    }
                    let connections_key =
                        serde_json::to_string(&snapshot.connections).unwrap_or_default();
                    if connections_key != last_connections {
                        for row in connection_rows.drain(..) {
                            connected_group.remove(&row);
                        }
                        no_connections.set_visible(snapshot.connections.is_empty());
                        for connection in &snapshot.connections {
                            let row = adw::ActionRow::builder()
                                .title(&connection.label)
                                .subtitle_selectable(true)
                                .subtitle_lines(2)
                                .title_lines(1)
                                .use_markup(false)
                                .build();
                            row.add_prefix(&gtk::Image::from_icon_name("network-server-symbolic"));
                            if connection.managed_by_nix {
                                row.set_tooltip_text(Some(lang.text("connectionManagedByNix")));
                            }
                            let check =
                                icon_button("view-refresh-symbolic", lang.text("testConnection"));
                            let sender = send.clone();
                            let id = connection.id.clone();
                            check.connect_clicked(move |_| {
                                let _ = sender.send(Request::CheckConnection(id.clone()));
                            });
                            row.add_suffix(&check);
                            if !connection.managed_by_nix {
                                let remove = icon_button(
                                    "user-trash-symbolic",
                                    lang.text("removeConnection"),
                                );
                                let weak_window = weak_window.clone();
                                let sender = send.clone();
                                let id = connection.id.clone();
                                let label = connection.label.clone();
                                remove.connect_clicked(move |_| {
                                    if let Some(window) = weak_window.upgrade() {
                                        let dialog = adw::AlertDialog::builder()
                                            .heading(lang.text("removeConnection"))
                                            .body(format!(
                                                "{}\n\n{}",
                                                label,
                                                lang.text("removeConnectionHelp")
                                            ))
                                            .default_response("cancel")
                                            .close_response("cancel")
                                            .build();
                                        dialog.add_responses(&[
                                            ("cancel", lang.text("cancel")),
                                            ("remove", lang.text("removeConnection")),
                                        ]);
                                        dialog.set_response_appearance(
                                            "remove",
                                            adw::ResponseAppearance::Destructive,
                                        );
                                        let sender = sender.clone();
                                        let id = id.clone();
                                        dialog.connect_response(None, move |_, response| {
                                            if response == "remove" {
                                                let _ = sender
                                                    .send(Request::RemoveConnection(id.clone()));
                                            }
                                        });
                                        dialog.present(Some(&window));
                                    }
                                });
                                row.add_suffix(&remove);
                            }
                            connected_group.add(&row);
                            connection_rows.push(row);
                        }
                        last_connections = connections_key;
                    }
                    for (row, connection) in connection_rows.iter().zip(&snapshot.connections) {
                        let subtitle = match snapshot.connection_health.get(&connection.id) {
                            Some(true) => {
                                format!("{}\n{}", connection.url, lang.text("connectionOnline"))
                            }
                            Some(false) => {
                                format!("{}\n{}", connection.url, lang.text("connectionOffline"))
                            }
                            None => connection.url.clone(),
                        };
                        row.set_subtitle(&subtitle);
                    }
                    for (action, button) in &buttons {
                        button.set_sensitive(
                            snapshot.status.active.is_none()
                                && (!action.privileged() || snapshot.status.system_changes_enabled)
                                && (!action.requires_configuration()
                                    || snapshot.status.config_kind
                                        != crate::config::ConfigKind::Unknown)
                                && (*action != Action::UpdateInputs
                                    || snapshot.status.config_kind
                                        == crate::config::ConfigKind::Flake),
                        );
                        if action.privileged() && !snapshot.status.system_changes_enabled {
                            button.set_tooltip_text(Some(lang.text("enableChanges")));
                        } else {
                            button.set_tooltip_text(Some(description(lang, *action)));
                        }
                    }
                    let history_key = serde_json::to_string(&snapshot.jobs).unwrap_or_default();
                    let history_changed = history_key != last_history;
                    if history_changed {
                        while let Some(child) = history_box.first_child() {
                            history_box.remove(&child);
                        }
                        if snapshot.jobs.is_empty() {
                            history_box.append(
                                &adw::ActionRow::builder()
                                    .title(lang.text("noOperations"))
                                    .subtitle(lang.text("startCheck"))
                                    .activatable(false)
                                    .selectable(false)
                                    .build(),
                            );
                        }
                        for job in &snapshot.jobs {
                            let row = adw::ActionRow::builder()
                                .title(label(lang, job.action))
                                .title_lines(1)
                                .subtitle(format!(
                                    "{} · {}",
                                    status_label(lang, &job.status),
                                    job.id
                                ))
                                .subtitle_lines(1)
                                .use_markup(false)
                                .activatable(true)
                                .build();
                            let id = job.id.clone();
                            let sender = send.clone();
                            row.connect_activated(move |_| {
                                let _ = sender.send(Request::Refresh(Some(id.clone())));
                            });
                            history_box.append(&row);
                        }
                        // Leave half of the fourth row visible to indicate that
                        // the list scrolls. Measure the rows in the current theme.
                        let height = if snapshot.jobs.len() > 3 {
                            let row_height = |index| {
                                history_box
                                    .row_at_index(index)
                                    .unwrap()
                                    .measure(gtk::Orientation::Vertical, -1)
                                    .1
                            };
                            row_height(0) + row_height(1) + row_height(2) + row_height(3) / 2
                        } else {
                            -1
                        };
                        if operations_scroll.max_content_height() != height {
                            operations_scroll.set_min_content_height(-1);
                            operations_scroll.set_max_content_height(height);
                            operations_scroll.set_min_content_height(height);
                        }
                        last_history = history_key;
                    }
                    let selected = snapshot
                        .jobs
                        .iter()
                        .enumerate()
                        .find(|(_, job)| Some(&job.id) == snapshot.selected.as_ref());
                    if let Some((index, job)) = selected {
                        if history_changed || snapshot.selected != last_selection {
                            let row = history_box.row_at_index(index as i32);
                            history_box.select_row(row.as_ref());
                        }
                        operation_status.set_subtitle(status_label(lang, &job.status));
                        operation_started.set_subtitle(&format_timestamp(lang, Some(job.started)));
                        operation_finished.set_subtitle(&format_timestamp(lang, job.finished));
                        operation_id.set_subtitle(&job.id);
                    } else {
                        operation_status.set_subtitle(lang.text("logHint"));
                        operation_started.set_subtitle(lang.text("unavailable"));
                        operation_finished.set_subtitle(lang.text("unavailable"));
                        operation_id.set_subtitle(lang.text("unavailable"));
                    }
                    let log_text = snapshot
                        .log
                        .as_deref()
                        .unwrap_or_else(|| lang.text("noOperations"));
                    let selection_changed = snapshot.selected != last_selection;
                    if log_text != last_log || selection_changed {
                        if selection_changed {
                            log_buffer.set_text(log_text);
                        } else if let Some(extra) = log_text.strip_prefix(&last_log) {
                            log_buffer.insert(&mut log_buffer.end_iter(), extra);
                        } else {
                            log_buffer.set_text(log_text);
                        }
                        log_buffer.move_mark(&log_end, &log_buffer.end_iter());
                        last_log = log_text.to_owned();
                        log.scroll_to_mark(&log_end, 0.0, true, 0.0, 1.0);
                        let adjustment = log_scroll.vadjustment();
                        adw::glib::idle_add_local_once(move || {
                            let end = (adjustment.upper() - adjustment.page_size())
                                .max(adjustment.lower());
                            adjustment.set_value(end);
                        });
                    }
                    last_selection = snapshot.selected;
                }
            }
        }
        adw::glib::ControlFlow::Continue
    });
    window
}

fn build_login_window(
    app: &adw::Application,
    client: Client,
    language: Language,
) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Yonix")
        .default_width(520)
        .default_height(420)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(32)
        .margin_bottom(32)
        .margin_start(32)
        .margin_end(32)
        .build();
    let title = gtk::Label::builder()
        .label(language.text("welcome"))
        .xalign(0.0)
        .build();
    title.add_css_class("title-1");
    content.append(&title);
    let description = gtk::Label::builder()
        .label(language.text("welcomeDescription"))
        .wrap(true)
        .xalign(0.0)
        .build();
    description.add_css_class("dim-label");
    content.append(&description);
    let token = gtk::Entry::builder()
        .placeholder_text(language.text("tokenPlaceholder"))
        .visibility(false)
        .hexpand(true)
        .build();
    content.append(&token);
    let error = gtk::Label::builder().xalign(0.0).wrap(true).build();
    error.add_css_class("error");
    content.append(&error);
    let connect = gtk::Button::with_label(language.text("connect"));
    connect.add_css_class("suggested-action");
    content.append(&connect);
    let weak_window = window.downgrade();
    let app = app.clone();
    let client_for_login = client.clone();
    let token_for_login = token.clone();
    let error_for_login = error.clone();
    let authenticate =
        Rc::new(
            move || match client_for_login.with_token(token_for_login.text().as_str()) {
                Ok(client) => {
                    let next = build_window(&app, client, language);
                    next.present();
                    if let Some(window) = weak_window.upgrade() {
                        window.close();
                    }
                }
                Err(error) => error_for_login.set_text(&error.to_string()),
            },
        );
    let authenticate_for_click = authenticate.clone();
    connect.connect_clicked(move |_| authenticate_for_click());
    token.connect_activate(move |_| authenticate());
    let clamp = adw::Clamp::builder()
        .maximum_size(620)
        .child(&content)
        .build();
    window.set_content(Some(&clamp));
    window
}

pub fn run(client: Client) {
    let app = adw::Application::builder()
        .application_id("com.alphas.yonix")
        .build();
    app.connect_activate(move |app| {
        if let Some(window) = app.active_window() {
            window.present();
        } else if client.has_token() {
            build_window(app, client.clone(), Language::system()).present();
        } else {
            build_login_window(app, client.clone(), Language::system()).present();
        }
    });
    app.run_with_args::<&str>(&[]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Cache, Config},
        engine::Engine,
    };

    #[test]
    fn chooses_supported_system_language() {
        for (code, _) in crate::i18n::LOCALES {
            assert_eq!(
                language_from_names([format!("{code}_XX.UTF-8").as_str()]),
                Language(code)
            );
        }
        assert_eq!(
            language_from_names(["ru_RU.UTF-8", "en_US"]),
            Language("ru")
        );
        assert_eq!(
            language_from_names(["de_DE", "en_GB", "ru_RU"]),
            Language("de")
        );
        assert_eq!(language_from_names(["sv_SE", "ru-RU"]), Language("ru"));
        assert_eq!(language_from_names(["C.UTF-8"]), Language("en"));
    }

    #[test]
    fn system_language_follows_locale_environment() {
        if let Ok(expected) = std::env::var("YONIX_TEST_EXPECTED_LANGUAGE") {
            assert_eq!(Language::system().0, expected);
            return;
        }
        for (locale, expected) in [
            ("ru_RU.UTF-8", "ru"),
            ("en_US.UTF-8", "en"),
            ("de_DE.UTF-8", "de"),
            ("es_ES.UTF-8", "es"),
            ("fr_FR.UTF-8", "fr"),
            ("it_IT.UTF-8", "it"),
            ("ja_JP.UTF-8", "ja"),
            ("ko_KR.UTF-8", "ko"),
            ("pl_PL.UTF-8", "pl"),
            ("pt_BR.UTF-8", "pt"),
            ("tr_TR.UTF-8", "tr"),
            ("uk_UA.UTF-8", "uk"),
            ("zh_CN.UTF-8", "zh"),
            ("C.UTF-8", "en"),
        ] {
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "gui::tests::system_language_follows_locale_environment",
                ])
                .env("LC_ALL", locale)
                .env_remove("LANGUAGE")
                .env("YONIX_TEST_EXPECTED_LANGUAGE", expected)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "locale {locale}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }

    #[test]
    #[ignore = "requires a display; run under xvfb-run"]
    fn native_window_renders_and_connects() {
        adw::init().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let cache_key_file = dir.path().join("cache-key");
        let config = Config {
            config_path: dir.path().join("nixos"),
            state_dir: dir.path().join("state"),
            host: "nas".into(),
            port,
            privileged_helper: None,
            source_mode: "git".into(),
            listen_address: std::net::Ipv4Addr::LOCALHOST.into(),
            peers: Vec::new(),
            cache: Some(Cache {
                url: "http://127.0.0.1:9848".into(),
                public_key_file: cache_key_file.clone(),
            }),
            shared_token: false,
        };
        std::fs::create_dir(&config.config_path).unwrap();
        let jobs_dir = config.state_dir.join("jobs");
        std::fs::create_dir_all(&jobs_dir).unwrap();
        let jobs: Vec<_> = (0..6)
            .map(|index| {
                let id = format!("test-{index}");
                std::fs::write(jobs_dir.join(format!("{id}.log")), format!("Log for {id}"))
                    .unwrap();
                Job {
                    id,
                    action: Action::Build,
                    status: "succeeded".into(),
                    started: 1_700_000_000,
                    finished: Some(1_700_000_020),
                    error: None,
                }
            })
            .collect();
        std::fs::write(
            config.state_dir.join("history.json"),
            serde_json::to_vec(&jobs).unwrap(),
        )
        .unwrap();
        let engine = Engine::open(config.clone()).unwrap();
        let token = crate::engine::ensure_token(&config).unwrap();
        let router = crate::server::router(engine, token);
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = std::thread::spawn(move || {
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                axum::serve(listener, router)
                    .with_graceful_shutdown(async {
                        let _ = stopped.await;
                    })
                    .await
                    .unwrap();
            });
        });
        let app = adw::Application::builder()
            .application_id("com.alphas.yonix.Test")
            .flags(adw::gio::ApplicationFlags::NON_UNIQUE)
            .build();
        app.register(None::<&adw::gio::Cancellable>).unwrap();
        let window = build_window(
            &app,
            Client::new(&config, None, None).unwrap(),
            Language("ru"),
        );
        window.present();
        let context = adw::glib::MainContext::default();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        fn collect(widget: &gtk::Widget, labels: &mut Vec<String>) {
            if let Some(label) = widget.downcast_ref::<gtk::Label>() {
                labels.push(label.text().into());
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                collect(&widget, labels);
                child = widget.next_sibling();
            }
        }
        fn find_widget<T: IsA<gtk::Widget>>(
            widget: &gtk::Widget,
            matches: &impl Fn(&T) -> bool,
        ) -> Option<T> {
            if let Some(found) = widget.downcast_ref::<T>() {
                if matches(found) {
                    return Some(found.clone());
                }
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if let Some(found) = find_widget(&widget, matches) {
                    return Some(found);
                }
                child = widget.next_sibling();
            }
            None
        }
        loop {
            while context.pending() {
                context.iteration(false);
            }
            let mut labels = Vec::new();
            collect(window.upcast_ref(), &mut labels);
            if labels
                .iter()
                .any(|s| s == &format!("nas · {}", Language("ru").text("missingConfiguration")))
            {
                assert!(labels.iter().any(|s| s == "Собрать"));
                assert!(labels.iter().any(|s| s == "Готово · test-5"));
                assert!(
                    labels
                        .iter()
                        .any(|s| s == &format!("v{}", env!("CARGO_PKG_VERSION")))
                );
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "GUI did not display the authenticated host status"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        WidgetExt::activate_action(&window, "win.settings", None).unwrap();
        let settings = window.visible_dialog().unwrap();
        let language_row = find_widget::<adw::ComboRow>(settings.upcast_ref(), &|_| true).unwrap();
        let selected = language_row
            .selected_item()
            .unwrap()
            .downcast::<gtk::StringObject>()
            .unwrap();
        assert_eq!(selected.string(), "Русский");
        assert_eq!(
            language_row.model().unwrap().n_items() as usize,
            crate::i18n::LOCALES.len()
        );
        let disconnect = find_widget::<gtk::Button>(settings.upcast_ref(), &|button| {
            button.label().as_deref() == Some(Language("ru").text("logout"))
        })
        .unwrap();
        assert_eq!(disconnect.halign(), gtk::Align::Center);
        assert!(disconnect.has_css_class("destructive-action"));
        assert!(disconnect.ancestor(gtk::ListBox::static_type()).is_none());
        let cache_url = find_widget::<adw::ActionRow>(settings.upcast_ref(), &|row| {
            row.title() == Language("ru").text("binaryCache")
        })
        .unwrap();
        let cache_key = find_widget::<adw::ActionRow>(settings.upcast_ref(), &|row| {
            row.title() == Language("ru").text("cachePublicKey")
        })
        .unwrap();
        assert_eq!(
            cache_url.subtitle().as_deref(),
            Some("http://127.0.0.1:9848")
        );
        let unavailable = Language("ru").text("unavailable");
        assert_eq!(cache_key.subtitle().as_deref(), Some(unavailable));
        for (contents, expected) in [
            ("test-cache:public\n", "test-cache:public"),
            ("test-cache:rotated\n", "test-cache:rotated"),
            ("  \n", unavailable),
        ] {
            std::fs::write(&cache_key_file, contents).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while cache_key.subtitle().as_deref() != Some(expected) {
                while context.pending() {
                    context.iteration(false);
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "Cache public key did not update"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        assert!(settings.close());
        if let Ok(path) = std::env::var("YONIX_GUI_SNAPSHOT") {
            let until = std::time::Instant::now() + Duration::from_millis(300);
            while std::time::Instant::now() < until {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(
                std::process::Command::new("import")
                    .args(["-window", "root", &path])
                    .status()
                    .unwrap()
                    .success()
            );
        }
        window.close();
        let english_window = build_window(
            &app,
            Client::new(&config, None, None).unwrap(),
            Language("en"),
        );
        english_window.present();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            while context.pending() {
                context.iteration(false);
            }
            let mut labels = Vec::new();
            collect(english_window.upcast_ref(), &mut labels);
            if labels
                .iter()
                .any(|s| s == &format!("nas · {}", Language("en").text("missingConfiguration")))
            {
                assert!(labels.iter().any(|s| s == "Build"));
                assert!(labels.iter().any(|s| s == "Succeeded · test-5"));
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "English GUI did not connect"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        fn operation_button(widget: &gtk::Widget, tooltip: &str) -> Option<gtk::Button> {
            if let Some(button) = widget.downcast_ref::<gtk::Button>() {
                if button.tooltip_text().as_deref() == Some(tooltip) {
                    return Some(button.clone());
                }
            }
            let mut child = widget.first_child();
            while let Some(widget) = child {
                if let Some(button) = operation_button(&widget, tooltip) {
                    return Some(button);
                }
                child = widget.next_sibling();
            }
            None
        }
        let language = Language("en");
        let switcher =
            find_widget::<adw::ViewSwitcher>(english_window.upcast_ref(), &|_| true).unwrap();
        let stack = switcher.stack().unwrap();
        let expected_policy = if english_window.width() <= 700 {
            adw::ViewSwitcherPolicy::Narrow
        } else {
            adw::ViewSwitcherPolicy::Wide
        };
        assert_eq!(switcher.policy(), expected_policy);
        assert!(stack.child_by_name("maintenance").is_none());
        WidgetExt::activate_action(&english_window, "win.connections", None).unwrap();
        assert_eq!(stack.visible_child_name().as_deref(), Some("connections"));
        let connections = stack.child_by_name("connections").unwrap();
        let pairing_entry = find_widget::<adw::EntryRow>(&connections, &|_| true).unwrap();
        assert!(pairing_entry.is_sensitive());
        switcher.set_policy(adw::ViewSwitcherPolicy::Narrow);
        assert_eq!(switcher.policy(), adw::ViewSwitcherPolicy::Narrow);
        assert!(switcher.is_mapped());
        assert_eq!(stack.visible_child_name().as_deref(), Some("connections"));
        pairing_entry.set_text("invalid demo code");
        pairing_entry.emit_by_name::<()>("entry-activated", &[]);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            while context.pending() {
                context.iteration(false);
            }
            let mut labels = Vec::new();
            collect(&connections, &mut labels);
            if labels
                .iter()
                .any(|label| label == "Pairing code must be 6 characters (Crockford base32)")
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "Pairing error did not appear in the form"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        WidgetExt::activate_action(&english_window, "win.history", None).unwrap();
        let history_page = stack.child_by_name("history").unwrap();
        let list =
            find_widget::<gtk::ListBox>(&history_page, &|list| list.row_at_index(3).is_some())
                .unwrap();
        let list_scroll = list
            .ancestor(gtk::ScrolledWindow::static_type())
            .unwrap()
            .downcast::<gtk::ScrolledWindow>()
            .unwrap();
        let adjustment = list_scroll.vadjustment();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while adjustment.page_size() == 0.0 {
            while context.pending() {
                context.iteration(false);
            }
            assert!(
                std::time::Instant::now() < deadline,
                "History was not allocated"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let row = list.row_at_index(3).unwrap();
        let bounds = row.compute_bounds(&list).unwrap();
        let midpoint = f64::from(bounds.y() + bounds.height() / 2.0);
        let page_size = adjustment.page_size();
        assert!(
            (page_size - midpoint).abs() <= 2.0,
            "Expected 3.5 operation rows, fourth midpoint {midpoint}, viewport {page_size}"
        );
        let mut labels = Vec::new();
        collect(&history_page, &mut labels);
        assert!(labels.contains(&format_timestamp(language, Some(jobs[5].started)).to_string()));
        assert!(labels.contains(&format_timestamp(language, jobs[5].finished).to_string()));
        WidgetExt::activate_action(&english_window, "win.system", None).unwrap();
        let system = stack.child_by_name("system").unwrap();
        let build = operation_button(&system, description(language, Action::Build)).unwrap();
        let update = operation_button(&system, label(language, Action::UpdateInputs)).unwrap();
        assert!(!build.is_sensitive());
        assert!(!update.is_sensitive());

        let wait_until_sensitive = |button: &gtk::Button| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while !button.is_sensitive() {
                while context.pending() {
                    context.iteration(false);
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "Button stayed disabled: {:?}",
                    button.tooltip_text()
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        let legacy = config.config_path.join("configuration.nix");
        let flake = config.config_path.join("flake.nix");
        std::fs::write(&legacy, "{}").unwrap();
        wait_until_sensitive(&build);
        assert!(
            !update.is_sensitive(),
            "Legacy configurations have no flake.lock"
        );

        std::fs::write(&flake, "{}").unwrap();
        wait_until_sensitive(&update);
        let (send, receive) = mpsc::channel();
        launch(&english_window, &send, Action::UpdateInputs, language);
        let dialog = english_window
            .visible_dialog()
            .unwrap()
            .downcast::<adw::AlertDialog>()
            .unwrap();
        assert_eq!(dialog.body(), language.text("confirmUpdate"));
        assert_eq!(dialog.default_response().as_deref(), Some("cancel"));
        let (responded, response) = mpsc::channel();
        dialog.connect_response(None, move |_, value| {
            responded.send(value.to_owned()).unwrap();
        });
        assert!(dialog.close());
        assert_eq!(response.try_recv().unwrap(), "cancel");
        assert!(matches!(receive.try_recv(), Err(mpsc::TryRecvError::Empty)));
        std::fs::remove_file(flake).unwrap();
        std::fs::remove_file(legacy).unwrap();
        english_window.close();
        for (code, _) in crate::i18n::LOCALES {
            if matches!(*code, "ru" | "en") {
                continue;
            }
            let window = build_window(
                &app,
                Client::new(&config, None, None).unwrap(),
                Language(code),
            );
            window.present();
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                while context.pending() {
                    context.iteration(false);
                }
                let mut labels = Vec::new();
                collect(window.upcast_ref(), &mut labels);
                if labels
                    .iter()
                    .any(|s| s == &format!("nas · {}", Language(code).text("missingConfiguration")))
                {
                    assert!(
                        labels.iter().any(|s| s == Language(code).text("build")),
                        "{code}: build label missing"
                    );
                    assert!(
                        labels
                            .iter()
                            .any(|s| s == &format!("{} · test-5", Language(code).text("succeeded"))),
                        "{code}: history label missing"
                    );
                    break;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "{code}: GUI did not connect"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            window.close();
        }
        let _ = stop.send(());
        server.join().unwrap();
    }
}
