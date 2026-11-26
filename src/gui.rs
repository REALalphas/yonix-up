use crate::{
    client::Client,
    engine::{Action, Job, Status},
    peers::{Connection, Host},
};
use adw::{gtk, prelude::*};
use std::{
    cell::Cell,
    rc::Rc,
    sync::mpsc::{self, Sender},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Language(&'static str);

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
    notice: Option<Notice>,
    pairing_enabled: bool,
}

enum Notice {
    ConnectionRemoved,
    PairingCode { code: String, link: String },
    PairingPaired,
    Error(String),
}

fn worker(
    client: Client,
    requests: mpsc::Receiver<Request>,
    updates: mpsc::Sender<Result<Snapshot, String>>,
) {
    let mut selected = None;
    let mut selected_host = "local".to_owned();
    let mut connection_health = std::collections::HashMap::new();
    let mut notice = None;
    loop {
        let request = requests.recv_timeout(Duration::from_millis(1500));
        match request {
            Ok(Request::Start(action)) => match client.start_for(&selected_host, action, true) {
                Ok(job) => selected = Some(job.id),
                Err(error) => {
                    let _ = updates.send(Err(error.to_string()));
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
                    Err(error) => notice = Some(Notice::Error(error.to_string())),
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
            if selected.is_none() {
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
                notice: notice.take(),
                pairing_enabled,
            })
        })();
        if updates.send(result.map_err(|e| e.to_string())).is_err() {
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
    let header_title = adw::WindowTitle::new("Yonix", initial_language.text("companion"));
    header.set_title_widget(Some(&header_title));
    let (send, receive) = mpsc::channel();
    let (update_send, updates) = mpsc::channel();
    let host_selector = gtk::ComboBoxText::new();
    host_selector.set_tooltip_text(Some(initial_language.text("managedHost")));
    host_selector.set_valign(gtk::Align::Center);
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
    toolbar.add_top_bar(&header);
    let toast = adw::ToastOverlay::new();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(24)
        .margin_top(32)
        .margin_bottom(32)
        .margin_start(24)
        .margin_end(24)
        .build();
    let title = gtk::Label::builder()
        .label(initial_language.text("gtkTitle"))
        .xalign(0.0)
        .wrap(true)
        .build();
    title.add_css_class("title-1");
    content.append(&title);
    let subtitle = gtk::Label::builder()
        .label(initial_language.text("connecting"))
        .xalign(0.0)
        .wrap(true)
        .build();
    subtitle.add_css_class("dim-label");
    content.append(&subtitle);
    let system = adw::PreferencesGroup::builder()
        .title(initial_language.text("system"))
        .build();
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
    let history_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    history_box.add_css_class("boxed-list");
    history.add(&history_box);
    content.append(&history);
    let operation_details = adw::PreferencesGroup::builder()
        .title(initial_language.text("history"))
        .build();
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
    content.append(&operation_details);
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
    log.buffer().set_text(initial_language.text("noOperations"));
    let log_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(220)
        .max_content_height(380)
        .propagate_natural_height(true)
        .child(&log)
        .build();
    log_scroll.add_css_class("card");
    log_group.add(&log_scroll);
    content.append(&log_group);

    let connections_group = adw::PreferencesGroup::builder()
        .title(initial_language.text("connections"))
        .description(initial_language.text("connectionsDescription"))
        .build();
    let connections_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    connections_group.add(&connections_box);
    let pairing_row = adw::ActionRow::builder()
        .title(initial_language.text("pairingAllow"))
        .subtitle(initial_language.text("addConnectionHelp"))
        .build();
    let pairing_switch = gtk::Switch::builder().valign(gtk::Align::Center).build();
    pairing_switch.set_sensitive(false);
    let updating_pairing = Rc::new(Cell::new(false));
    let updating_pairing_for_callback = updating_pairing.clone();
    pairing_row.add_suffix(&pairing_switch);
    let share_pairing = gtk::Button::with_label(initial_language.text("addConnection"));
    share_pairing.add_css_class("suggested-action");
    share_pairing.set_sensitive(false);
    pairing_row.add_suffix(&share_pairing);
    connections_group.add(&pairing_row);
    let accept_row = adw::ActionRow::builder()
        .title(initial_language.text("addOtherInstance"))
        .build();
    let pairing_entry = gtk::Entry::builder()
        .placeholder_text(initial_language.text("pairingPasteLink"))
        .hexpand(true)
        .build();
    let accept_pairing = gtk::Button::with_label(initial_language.text("pairingAccept"));
    accept_pairing.set_sensitive(false);
    accept_row.add_prefix(&pairing_entry);
    accept_row.add_suffix(&accept_pairing);
    connections_group.add(&accept_row);
    let pairing_origin = gtk::Entry::builder()
        .placeholder_text(initial_language.text("pairingOriginPlaceholder"))
        .hexpand(true)
        .build();
    let origin_row = adw::ActionRow::builder()
        .title(initial_language.text("pairingOrigin"))
        .build();
    origin_row.add_suffix(&pairing_origin);
    connections_group.add(&origin_row);
    let allow_public = gtk::CheckButton::with_label(initial_language.text("pairingPublicConfirm"));
    let public_row = adw::ActionRow::builder().build();
    public_row.add_suffix(&allow_public);
    connections_group.add(&public_row);
    let pairing_result = gtk::Entry::builder()
        .editable(false)
        .placeholder_text(initial_language.text("pairingCode"))
        .hexpand(true)
        .build();
    let copy_pairing = gtk::Button::with_label(initial_language.text("copyPairingCode"));
    copy_pairing.set_sensitive(false);
    let pairing_result_row = adw::ActionRow::builder()
        .title(initial_language.text("pairingCode"))
        .build();
    pairing_result_row.add_prefix(&pairing_result);
    pairing_result_row.add_suffix(&copy_pairing);
    connections_group.add(&pairing_result_row);
    content.append(&connections_group);

    let settings_group = adw::PreferencesGroup::builder()
        .title(initial_language.text("session"))
        .build();
    let language_selector = gtk::ComboBoxText::new();
    for (code, _) in crate::i18n::LOCALES {
        language_selector.append(Some(code), code);
    }
    language_selector.set_active_id(Some(initial_language.0));
    language_selector.set_valign(gtk::Align::Center);
    let language_row = adw::ActionRow::builder()
        .title(initial_language.text("language"))
        .subtitle(initial_language.0)
        .build();
    language_row.add_suffix(&language_selector);
    settings_group.add(&language_row);
    let language_app = app.clone();
    let weak_window_for_language = window.downgrade();
    let language_client_for_switch = language_client.clone();
    language_selector.connect_changed(move |selector| {
        let Some(code) = selector.active_id() else {
            return;
        };
        let Some((locale, _)) = crate::i18n::LOCALES
            .iter()
            .find(|(locale, _)| *locale == code.as_str())
        else {
            return;
        };
        if *locale == initial_language.0 {
            return;
        }
        if let Some(window) = weak_window_for_language.upgrade() {
            window.close();
        }
        build_window(
            &language_app,
            language_client_for_switch.clone(),
            Language(locale),
        )
        .present();
    });
    let disconnect = gtk::Button::with_label(initial_language.text("logout"));
    disconnect.add_css_class("flat");
    let weak_window_for_disconnect = window.downgrade();
    let login_app = app.clone();
    let login_client = language_client.clone();
    disconnect.connect_clicked(move |_| {
        if let Some(window) = weak_window_for_disconnect.upgrade() {
            let login = build_login_window(&login_app, login_client.clone(), initial_language);
            window.close();
            login.present();
        }
    });
    let disconnect_row = adw::ActionRow::builder()
        .title(initial_language.text("session"))
        .build();
    disconnect_row.add_suffix(&disconnect);
    settings_group.add(&disconnect_row);
    let cache_url = adw::ActionRow::builder()
        .title(initial_language.text("binaryCache"))
        .subtitle(initial_language.text("unavailable"))
        .subtitle_selectable(true)
        .build();
    settings_group.add(&cache_url);
    let cache_key = adw::ActionRow::builder()
        .title(initial_language.text("cachePublicKey"))
        .subtitle(initial_language.text("unavailable"))
        .subtitle_selectable(true)
        .build();
    settings_group.add(&cache_key);
    content.append(&settings_group);

    let sender = send.clone();
    let pairing_switch_for_callback = pairing_switch.clone();
    pairing_switch.connect_active_notify(move |_| {
        if updating_pairing_for_callback.get() {
            return;
        }
        let _ = sender.send(Request::SetPairing(pairing_switch_for_callback.is_active()));
    });
    let sender = send.clone();
    share_pairing.connect_clicked(move |_| {
        let _ = sender.send(Request::MintPairing);
    });
    let pairing_result_for_copy = pairing_result.clone();
    copy_pairing.connect_clicked(move |_| {
        if let Some(display) = gtk::gdk::Display::default() {
            display
                .clipboard()
                .set_text(&pairing_result_for_copy.text());
        }
    });
    let sender = send.clone();
    let pairing_entry_for_accept = pairing_entry.clone();
    let pairing_origin_for_accept = pairing_origin.clone();
    let allow_public_for_accept = allow_public.clone();
    accept_pairing.connect_clicked(move |_| {
        let link = pairing_entry_for_accept.text().trim().to_owned();
        if !link.is_empty() {
            let origin = pairing_origin_for_accept.text().trim().to_owned();
            let _ = sender.send(Request::AcceptPairing {
                link,
                origin: (!origin.is_empty()).then_some(origin),
                allow_public: allow_public_for_accept.is_active(),
            });
        }
    });
    let clamp = adw::Clamp::builder()
        .maximum_size(1000)
        .child(&content)
        .build();
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&clamp)
        .build();
    toast.set_child(Some(&scroll));
    toolbar.set_content(Some(&toast));
    window.set_content(Some(&toolbar));

    let _ = send.send(Request::Refresh(None));
    let weak_window = window.downgrade();
    let mut last_history = String::new();
    let mut last_log = String::new();
    let mut last_error = String::new();
    let mut last_hosts = String::new();
    let mut last_connections = String::new();
    let mut last_notice = String::new();
    adw::glib::timeout_add_local(Duration::from_millis(150), move || {
        if weak_window.upgrade().is_none() {
            return adw::glib::ControlFlow::Break;
        }
        while let Ok(update) = updates.try_recv() {
            let lang = initial_language;
            match update {
                Err(error) => {
                    subtitle.set_text(lang.text("connectionError"));
                    for (_, button) in &buttons {
                        button.set_sensitive(false);
                    }
                    if error != last_error {
                        toast.add_toast(adw::Toast::new(&error));
                        last_error = error;
                    }
                }
                Ok(snapshot) => {
                    last_error.clear();
                    if let Some(notice) = snapshot.notice {
                        let notice_text = match notice {
                            Notice::ConnectionRemoved => lang.text("removeConnection").to_owned(),
                            Notice::PairingPaired => lang.text("pairingPaired").to_owned(),
                            Notice::PairingCode { code, link } => {
                                pairing_result.set_text(&format!(
                                    "{}: {code} · {link}",
                                    lang.text("pairingCode")
                                ));
                                copy_pairing.set_sensitive(true);
                                format!("{}: {code}", lang.text("pairingCode"))
                            }
                            Notice::Error(error) => error,
                        };
                        if notice_text != last_notice {
                            toast.add_toast(adw::Toast::new(&notice_text));
                            last_notice = notice_text;
                        }
                    }
                    if pairing_switch.is_active() != snapshot.pairing_enabled {
                        updating_pairing.set(true);
                        pairing_switch.set_active(snapshot.pairing_enabled);
                        updating_pairing.set(false);
                    }
                    share_pairing.set_sensitive(snapshot.pairing_enabled);
                    pairing_switch.set_sensitive(true);
                    accept_pairing.set_sensitive(true);
                    if !snapshot.pairing_enabled {
                        copy_pairing.set_sensitive(false);
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
                    source.set_subtitle(&snapshot.status.config_path.to_string_lossy());
                    current.set_subtitle(
                        &snapshot
                            .status
                            .current_system
                            .as_ref()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|| lang.text("unavailable").into()),
                    );
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
                                .unwrap_or(lang.text("unavailable")),
                        );
                    } else {
                        cache_url.set_subtitle(lang.text("unavailable"));
                        cache_key.set_subtitle(lang.text("unavailable"));
                    }
                    let connections_key = snapshot
                        .connections
                        .iter()
                        .map(|connection| {
                            format!(
                                "{}\u{1f}{}\u{1f}{:?}",
                                connection.id,
                                connection.url,
                                snapshot.connection_health.get(&connection.id)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\u{1e}");
                    if connections_key != last_connections {
                        while let Some(child) = connections_box.first_child() {
                            connections_box.remove(&child);
                        }
                        if snapshot.connections.is_empty() {
                            connections_box.append(
                                &gtk::Label::builder()
                                    .label(lang.text("noConnections"))
                                    .xalign(0.0)
                                    .build(),
                            );
                        }
                        for connection in &snapshot.connections {
                            let row = adw::ActionRow::builder()
                                .title(&connection.label)
                                .subtitle(&connection.url)
                                .subtitle_selectable(true)
                                .build();
                            let status = if snapshot.connection_health.get(&connection.id)
                                == Some(&true)
                            {
                                lang.text("connectionOnline")
                            } else if snapshot.connection_health.get(&connection.id) == Some(&false)
                            {
                                lang.text("connectionOffline")
                            } else {
                                lang.text("connectingShort")
                            };
                            row.set_subtitle(&format!("{} · {status}", connection.url));
                            if connection.managed_by_nix {
                                row.set_tooltip_text(Some(lang.text("connectionManagedByNix")));
                            }
                            let check = gtk::Button::with_label(lang.text("testConnection"));
                            check.add_css_class("flat");
                            let sender = send.clone();
                            let id = connection.id.clone();
                            check.connect_clicked(move |_| {
                                let _ = sender.send(Request::CheckConnection(id.clone()));
                            });
                            row.add_suffix(&check);
                            if !connection.managed_by_nix {
                                let remove = gtk::Button::with_label(lang.text("removeConnection"));
                                remove.add_css_class("flat");
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
                            connections_box.append(&row);
                        }
                        last_connections = connections_key;
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
                    let history_key = format!(
                        "{lang:?}:{}:{:?}",
                        serde_json::to_string(&snapshot.jobs).unwrap_or_default(),
                        snapshot.selected
                    );
                    if history_key != last_history {
                        while let Some(child) = history_box.first_child() {
                            history_box.remove(&child);
                        }
                        if snapshot.jobs.is_empty() {
                            history_box.append(
                                &adw::ActionRow::builder()
                                    .title(lang.text("noOperations"))
                                    .subtitle(lang.text("startCheck"))
                                    .build(),
                            );
                        }
                        for job in &snapshot.jobs {
                            let row = adw::ActionRow::builder()
                                .title(label(lang, job.action))
                                .subtitle(format!(
                                    "{} · {}",
                                    status_label(lang, &job.status),
                                    job.id
                                ))
                                .activatable(true)
                                .build();
                            if snapshot.selected.as_deref() == Some(&job.id) {
                                row.add_css_class("accent");
                            }
                            let id = job.id.clone();
                            let sender = send.clone();
                            row.connect_activated(move |_| {
                                let _ = sender.send(Request::Refresh(Some(id.clone())));
                            });
                            history_box.append(&row);
                        }
                        last_history = history_key;
                    }
                    if let Some(selected) = snapshot.selected.as_deref() {
                        if let Some(job) = snapshot.jobs.iter().find(|job| job.id == selected) {
                            operation_status.set_subtitle(status_label(lang, &job.status));
                            operation_started.set_subtitle(&job.started.to_string());
                            operation_finished.set_subtitle(
                                &job.finished
                                    .map(|finished| finished.to_string())
                                    .unwrap_or_else(|| lang.text("unavailable").to_owned()),
                            );
                            operation_id.set_subtitle(&job.id);
                        }
                    } else {
                        operation_status.set_subtitle(lang.text("logHint"));
                        operation_started.set_subtitle(lang.text("unavailable"));
                        operation_finished.set_subtitle(lang.text("unavailable"));
                        operation_id.set_subtitle(lang.text("unavailable"));
                    }
                    let log_text = snapshot
                        .log
                        .as_deref()
                        .unwrap_or_else(|| lang.text("logHint"));
                    if log_text != last_log {
                        log.buffer().set_text(log_text);
                        last_log = log_text.to_owned();
                    }
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
    use crate::{config::Config, engine::Engine};

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
        let config = Config {
            config_path: dir.path().join("nixos"),
            state_dir: dir.path().join("state"),
            host: "nas".into(),
            port,
            privileged_helper: None,
            source_mode: "git".into(),
            listen_address: std::net::Ipv4Addr::LOCALHOST.into(),
            peers: Vec::new(),
            cache: None,
            shared_token: false,
        };
        std::fs::create_dir(&config.config_path).unwrap();
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
                assert!(labels.iter().any(|s| s == "Операций пока нет"));
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "GUI did not display the authenticated host status"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
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
                assert!(labels.iter().any(|s| s == "No operations yet"));
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
        let build = operation_button(
            english_window.upcast_ref(),
            description(language, Action::Build),
        )
        .unwrap();
        let update = operation_button(
            english_window.upcast_ref(),
            label(language, Action::UpdateInputs),
        )
        .unwrap();
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
                            .any(|s| s == Language(code).text("noOperations")),
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
