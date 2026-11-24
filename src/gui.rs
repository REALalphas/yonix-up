use crate::{
    client::Client,
    engine::{Action, Job, Status},
};
use adw::{gtk, prelude::*};
use std::{
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
    Start(Action),
}
struct Snapshot {
    status: Status,
    jobs: Vec<Job>,
    selected: Option<String>,
    log: Option<String>,
}

fn worker(
    client: Client,
    requests: mpsc::Receiver<Request>,
    updates: mpsc::Sender<Result<Snapshot, String>>,
) {
    let mut selected = None;
    loop {
        let request = requests.recv_timeout(Duration::from_millis(1500));
        match request {
            Ok(Request::Start(action)) => match client.start(action, true) {
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
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        let result = (|| -> anyhow::Result<Snapshot> {
            let status = client.status()?;
            let jobs = client.jobs()?;
            if selected.is_none() {
                selected = jobs.first().map(|job| job.id.clone());
            }
            let log = if let Some(id) = &selected {
                let mut log = client.log(id)?;
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
                status,
                jobs,
                selected: selected.clone(),
                log,
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
fn launch(
    window: &adw::ApplicationWindow,
    sender: &Sender<Request>,
    action: Action,
    language: Language,
) {
    if !action.privileged() && action != Action::UpdateInputs {
        let _ = sender.send(Request::Start(action));
        return;
    }
    let body = match action {
        Action::Switch => language.text("confirmSwitch"),
        Action::Rollback => language.text("confirmRollback"),
        Action::Clean => language.text("confirmClean"),
        _ => language.text("confirmUpdate"),
    };
    let dialog = adw::AlertDialog::builder()
        .heading(label(language, action))
        .body(body)
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
    let (send, receive) = mpsc::channel();
    let (update_send, updates) = mpsc::channel();
    std::thread::spawn(move || worker(client, receive, update_send));
    let mut buttons = vec![];
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
        button.connect_clicked(move |_| {
            if let Some(window) = weak_window.upgrade() {
                launch(&window, &sender, action, initial_language);
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
        button.connect_clicked(move |_| {
            if let Some(window) = weak_window.upgrade() {
                launch(&window, &sender, action, initial_language);
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

pub fn run(client: Client) {
    let app = adw::Application::builder()
        .application_id("com.alphas.yonix")
        .build();
    app.connect_activate(move |app| {
        if let Some(window) = app.active_window() {
            window.present();
        } else {
            build_window(app, client.clone(), Language::system()).present();
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
