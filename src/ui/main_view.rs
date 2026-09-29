use super::browser_select::{BrowserSelectView, start_scrape};
use super::dialogs::{DialogSpec, show_dialog};
use super::{push_notification, t};
use gpui_kit::component::{
    ActiveTheme, Disableable as _, IndexPath, Sizable as _,
    button::Button,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState},
    menu::{DropdownMenu as _, PopupMenu, PopupMenuItem},
    notification::Notification,
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::*;

use crate::config::AppConfig;
use crate::constants::{NONE_BRANCH, STEAM_VERSION};
use crate::download::depot_downloader;
use crate::manifest::{
    ManifestEntry, ManifestMap, ManifestSizes, load_manifest_sizes, load_manifests, patch_title_of,
};
use crate::paths::AppPaths;
use crate::scraper;
use crate::steam::detect::{SteamGame, detect_games, sorted_by_name};
use crate::steam::{junctions, paths};
use crate::ui::console_view::{ConsoleSession, ConsoleView};

#[derive(Clone)]
pub struct VersionRow {
    pub date: String,
    pub branch: String,
    pub label: SharedString,
    pub active: bool,
    pub downloaded: bool,
}

/// The main window view, mirroring `VersionManagerApp`.
pub struct AppView {
    pub(crate) paths: AppPaths,
    pub config: AppConfig,
    games: Vec<SteamGame>,
    manifests: ManifestMap,
    manifest_sizes: ManifestSizes,
    game_display_names: Vec<String>,
    current_app: Option<usize>,
    game_select: Option<Entity<SelectState<Vec<String>>>>,
    select_sub: Option<Subscription>,
    language_select: Option<Entity<SelectState<Vec<String>>>>,
    language_sub: Option<Subscription>,
    language: String,
    username_input: Entity<InputState>,
    remember_password: bool,
    folder_selected: Option<usize>,
    sort_downloaded: bool,
    versions: Vec<VersionRow>,
    selected_version: Option<usize>,
    managed: bool,
    needs_select_rebuild: bool,
    pub(crate) weak: gpui_kit::WeakEntity<Self>,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let paths = AppPaths::resolve().expect("resolve app data directories");
        let config = AppConfig::load(&paths.config_file());
        let remember_password = config.remember_password;
        let username = config.username.clone();

        let language = config
            .language
            .clone()
            .filter(|s| s == "en" || s == "zh-CN")
            .unwrap_or_else(|| "en".to_string());

        let username_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx);
            state.set_value(username, window, cx);
            state
        });

        let mut view = Self {
            paths,
            config,
            games: Vec::new(),
            manifests: Default::default(),
            manifest_sizes: Default::default(),
            game_display_names: Vec::new(),
            current_app: None,
            game_select: None,
            select_sub: None,
            language_select: None,
            language_sub: None,
            language: language.clone(),
            username_input,
            remember_password,
            folder_selected: None,
            sort_downloaded: false,
            versions: Vec::new(),
            selected_version: None,
            managed: false,
            needs_select_rebuild: false,
            weak: cx.entity().downgrade(),
        };

        gpui_kit::component::set_locale(&language);
        view.rebuild_language_select(window, cx);
        view.refresh_games(window, cx);

        view
    }

    pub fn current_game(&self) -> Option<&SteamGame> {
        self.current_app.and_then(|ix| self.games.get(ix))
    }

    fn refresh_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_setup_state(cx);
        self.rebuild_game_select(window, cx);
        self.refresh_version_list(cx);
    }

    /// Window-free refresh for callbacks that run inside a window update
    /// (dialog `on_ok` etc.), where `update_in`/`update_window` on the same
    /// window would be rejected as re-entrant. The game Select is rebuilt on
    /// the next render via `needs_select_rebuild`.
    fn refresh_after_flow(&mut self, cx: &mut Context<Self>) {
        self.refresh_setup_state(cx);
        self.refresh_version_list(cx);
        self.needs_select_rebuild = true;
        cx.notify();
    }

    // region: language / game select

    fn rebuild_language_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names = vec!["English".to_string(), "简体中文".to_string()];
        let selected = match self.language.as_str() {
            "zh-CN" => Some(IndexPath::new(1)),
            _ => Some(IndexPath::new(0)),
        };
        let select = cx.new(|cx| SelectState::new(names, selected, window, cx));
        self.language_sub = Some(cx.subscribe_in(
            &select,
            window,
            |this, _entity, event: &SelectEvent<Vec<String>>, _window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event {
                    let locale = if value == "简体中文" {
                        "zh-CN"
                    } else {
                        "en"
                    };
                    if this.language != locale {
                        this.language = locale.to_string();
                        this.config.language = Some(locale.to_string());
                        let _ = this.config.save(&this.paths.config_file());
                        gpui_kit::component::set_locale(locale);
                        this.refresh_version_list(cx);
                        cx.refresh_windows();
                    }
                }
            },
        ));
        self.language_select = Some(select);
    }

    fn game_display_name(game: &SteamGame, managed: bool) -> String {
        format!("{}{}", if managed { "✓  " } else { "    " }, game.name)
    }

    fn rebuild_game_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.game_display_names = self
            .games
            .iter()
            .map(|g| Self::game_display_name(g, junctions::is_junction(&paths::active_folder(g))))
            .collect();
        let selected = self.current_app.map(IndexPath::new);
        let display_names = self.game_display_names.clone();
        let select = cx.new(|cx| SelectState::new(display_names, selected, window, cx));
        self.select_sub = Some(cx.subscribe_in(
            &select,
            window,
            |this, _entity, event: &SelectEvent<Vec<String>>, window, cx| {
                if let SelectEvent::Confirm(Some(value)) = event
                    && let Some(ix) = this
                        .game_display_names
                        .iter()
                        .position(|name| name == value)
                {
                    this.load_game(ix, window, cx);
                }
            },
        ));
        self.game_select = Some(select);
    }

    pub fn refresh_games(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_games_inner(cx);
        self.rebuild_game_select(window, cx);
    }

    fn refresh_games_inner(&mut self, cx: &mut Context<Self>) {
        self.games = sorted_by_name(detect_games(&self.config.steamapps_folders));
        let selected_app = self.config.current_app_id.clone();
        self.current_app = match selected_app
            .as_deref()
            .and_then(|id| self.games.iter().position(|g| g.app_id == id))
        {
            Some(ix) => Some(ix),
            None if !self.games.is_empty() => Some(0),
            _ => None,
        };

        if let Some(ix) = self.current_app {
            self.manifests = load_manifests(&self.paths.data_dir, &self.games[ix].app_id);
            self.manifest_sizes = load_manifest_sizes(&self.paths.data_dir, &self.games[ix].app_id);
        } else {
            self.manifests = Default::default();
        }
        self.refresh_setup_state(cx);
        self.refresh_version_list(cx);
    }

    fn load_game(&mut self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.games.len() {
            return;
        }
        self.current_app = Some(ix);
        let game = &self.games[ix];
        self.config.current_app_id = Some(game.app_id.clone());
        let _ = self.config.save(&self.paths.config_file());
        self.manifests = load_manifests(&self.paths.data_dir, &game.app_id);
        self.manifest_sizes = load_manifest_sizes(&self.paths.data_dir, &game.app_id);
        self.selected_version = None;
        self.refresh_setup_state(cx);
        self.refresh_version_list(cx);
    }

    pub fn refresh_setup_state(&mut self, cx: &mut Context<Self>) {
        self.managed = false;
        if let Some(game) = self.current_game().cloned() {
            let active = paths::active_folder(&game);
            let steam = paths::steam_folder(&game);
            // Self-heal: junction deleted but `_steam` still present.
            if !junctions::is_junction(&active) && !active.is_dir() && steam.is_dir() {
                let _ = junctions::create_junction(&active, &steam);
            }
            self.managed = junctions::is_junction(&active);
        }
        cx.notify();
    }

    pub fn refresh_version_list(&mut self, cx: &mut Context<Self>) {
        self.versions.clear();
        if let Some(game) = self.current_game().cloned() {
            let game_cfg = self.config.game_config(&game.app_id);
            let active_date = game_cfg.active_date().to_string();
            let active_branch = game_cfg.active_branch().to_string();
            let branch = game.beta.clone();

            self.versions.push(VersionRow {
                date: STEAM_VERSION.to_string(),
                branch: NONE_BRANCH.to_string(),
                label: t("version.steam").into(),
                active: active_date == STEAM_VERSION,
                downloaded: paths::steam_folder(&game).exists(),
            });

            let branch_data: Vec<(String, ManifestEntry)> = self
                .manifests
                .get(&branch)
                .map(|b| b.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                .unwrap_or_default();

            let mut dates: Vec<String> = branch_data.iter().map(|(d, _)| d.clone()).collect();
            if self.sort_downloaded {
                let (mut downloaded, mut rest): (Vec<String>, Vec<String>) = dates
                    .into_iter()
                    .partition(|d| paths::version_folder(&game, d, &branch).exists());
                downloaded.sort_by(|a, b| b.cmp(a));
                rest.sort_by(|a, b| b.cmp(a));
                dates = downloaded;
                dates.extend(rest);
            } else {
                dates.sort_by(|a, b| b.cmp(a));
            }

            for date in dates {
                let entry = branch_data
                    .iter()
                    .find(|(d, _)| *d == date)
                    .map(|(_, e)| e.clone())
                    .unwrap_or_default();
                let label = match patch_title_of(&entry) {
                    Some(title) if !title.is_empty() => format!("{date}  —  {title}"),
                    _ => date.clone(),
                };
                self.versions.push(VersionRow {
                    date: date.clone(),
                    branch: branch.clone(),
                    label: label.into(),
                    active: active_date == date && active_branch == branch,
                    downloaded: paths::version_folder(&game, &date, &branch).exists(),
                });
            }
        } else {
            self.selected_version = None;
        }
        cx.notify();
    }

    // endregion

    // region: steamapps folders

    fn on_add_folder(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_add_folder_flow(window, cx);
    }

    fn start_add_folder_flow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Run the native folder picker asynchronously: a blocking modal dialog
        // inside an event handler re-enters the message pump while GPUI holds
        // its App borrow, which can panic ("RefCell already borrowed") when a
        // pending async task is polled.
        let dialog = rfd::AsyncFileDialog::new().set_title(t("folders.pick_title"));
        cx.spawn_in(window, async move |this, cx| {
            let Some(picked) = dialog.pick_folder().await else {
                return;
            };
            let folder = picked.path().to_string_lossy().into_owned();
            let _ = this.update_in(cx, |view, window, cx| {
                view.add_folder_picked(folder, window, cx);
            });
        })
        .detach();
    }

    fn add_folder_picked(&mut self, folder: String, window: &mut Window, cx: &mut Context<Self>) {
        let basename = folder
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("");
        if !basename.eq_ignore_ascii_case("steamapps") {
            let message = rust_i18n::t!("folders.unusual", folder = folder.clone()).to_string();
            let weak = self.weak.clone();
            show_dialog(
                window,
                cx,
                DialogSpec::confirm(
                    t("folders.unusual_title"),
                    message,
                    move |_event, _window, cx| {
                        weak.update(cx, |view, cx| {
                            view.append_folder(folder.clone(), cx);
                        })
                        .ok();
                    },
                ),
            );
            return;
        }
        self.append_folder(folder, cx);
    }

    fn append_folder(&mut self, folder: String, cx: &mut Context<Self>) {
        if !self.config.steamapps_folders.contains(&folder) {
            self.config.steamapps_folders.push(folder);
            let _ = self.config.save(&self.paths.config_file());
            self.refresh_games_inner(cx);
            self.needs_select_rebuild = true;
        }
    }

    fn on_remove_folder(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.folder_selected else {
            return;
        };
        if ix < self.config.steamapps_folders.len() {
            self.config.steamapps_folders.remove(ix);
            let _ = self.config.save(&self.paths.config_file());
            self.folder_selected = None;
            self.refresh_games(window, cx);
        }
    }

    // endregion

    // region: version actions

    fn selected_key(&self) -> Option<(String, String)> {
        self.selected_version.and_then(|ix| {
            self.versions
                .get(ix)
                .map(|r| (r.date.clone(), r.branch.clone()))
        })
    }

    fn begin_download(&mut self, window: &mut Window, cx: &mut Context<Self>, via_console: bool) {
        let Some((date, branch)) = self.selected_key() else {
            return;
        };
        let Some(game) = self.current_game().cloned() else {
            return;
        };
        if date == STEAM_VERSION {
            show_dialog(
                window,
                cx,
                DialogSpec::info(t("common.not_allowed"), t("warn.steam_client")),
            );
            return;
        }
        let game_cfg = self.config.game_config(&game.app_id);
        let is_active = date == game_cfg.active_date() && branch == game_cfg.active_branch();

        if via_console {
            if is_active {
                show_dialog(
                    window,
                    cx,
                    DialogSpec::info(t("common.not_allowed"), t("warn.active_import")),
                );
                return;
            }
            if paths::version_folder(&game, &date, &branch).exists() {
                show_dialog(
                    window,
                    cx,
                    DialogSpec::info(
                        t("common.already_downloaded"),
                        rust_i18n::t!("warn.already_downloaded", date = date, branch = branch)
                            .to_string(),
                    ),
                );
                return;
            }
            let entry = self
                .manifests
                .get(&branch)
                .and_then(|b| b.get(&date))
                .cloned()
                .unwrap_or_default();
            let folder = paths::version_folder(&game, &date, &branch);
            ConsoleView::open(
                ConsoleSession {
                    app_view: self.weak.clone(),
                    game_name: game.name.clone(),
                    app_id: game.app_id.clone(),
                    depots: game.depots.clone(),
                    version_date: date,
                    branch,
                    manifest_entry: entry,
                    steamapps_folders: self.config.steamapps_folders.clone(),
                    version_folder: folder,
                    manifest_sizes: self.manifest_sizes.clone(),
                    data_dir: self.paths.data_dir.clone(),
                },
                cx,
            );
        } else {
            if is_active {
                show_dialog(
                    window,
                    cx,
                    DialogSpec::info(t("common.not_allowed"), t("warn.active_download")),
                );
                return;
            }
            let folder = paths::version_folder(&game, &date, &branch);
            let _ = std::fs::create_dir_all(&folder);
            let entry = self
                .manifests
                .get(&branch)
                .and_then(|b| b.get(&date))
                .cloned()
                .unwrap_or_default();
            let username = self.username_input.read(cx).value().to_string();
            self.config.username = username.clone();
            self.config.remember_password = self.remember_password;
            let _ = self.config.save(&self.paths.config_file());

            let lines = depot_downloader::build_batch_lines(
                &game.app_id,
                &game.depots,
                &branch,
                &entry,
                &username,
                self.remember_password,
                &folder,
            );
            if !depot_downloader::downloader_exe(&self.paths.exe_dir).is_file() {
                push_notification(
                    window,
                    cx,
                    Notification::warning(t("downloader.missing_exe")),
                );
            }
            let _ = depot_downloader::run_downloader(&self.paths.exe_dir, &lines);
            self.refresh_version_list(cx);
        }
    }

    fn on_set_active(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((date, branch)) = self.selected_key() else {
            return;
        };
        let Some(game) = self.current_game().cloned() else {
            return;
        };
        let target = paths::version_target_folder(&game, &date, &branch);
        if !target.exists() {
            show_dialog(
                window,
                cx,
                DialogSpec::info(
                    t("common.switch_failed"),
                    rust_i18n::t!("err.missing_folder", path = target.display().to_string())
                        .to_string(),
                ),
            );
            return;
        }
        let active_folder = paths::active_folder(&game);
        if (junctions::is_junction(&active_folder) || active_folder.exists())
            && let Err(err) = junctions::remove_junction(&active_folder)
        {
            show_dialog(
                window,
                cx,
                DialogSpec::info(
                    t("common.switch_failed"),
                    rust_i18n::t!("err.switch_remove", error = err.to_string()).to_string(),
                ),
            );
            return;
        }
        if let Err(err) = junctions::create_junction(&active_folder, &target) {
            show_dialog(
                window,
                cx,
                DialogSpec::info(
                    t("common.switch_failed"),
                    rust_i18n::t!("err.switch_create", error = err.to_string()).to_string(),
                ),
            );
            return;
        }
        {
            let gc = self.config.games.entry(game.app_id.clone()).or_default();
            gc.active_date = Some(date);
            gc.active_branch = Some(branch);
            gc.steamapps_folder = Some(game.steamapps_folder.to_string_lossy().into_owned());
        }
        let _ = self.config.save(&self.paths.config_file());
        self.refresh_version_list(cx);
    }

    fn on_delete_version(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((date, branch)) = self.selected_key() else {
            return;
        };
        let Some(game) = self.current_game().cloned() else {
            return;
        };
        if date == STEAM_VERSION {
            show_dialog(
                window,
                cx,
                DialogSpec::info(t("common.not_allowed"), t("warn.delete_steam")),
            );
            return;
        }
        let game_cfg = self.config.game_config(&game.app_id);
        if date == game_cfg.active_date() && branch == game_cfg.active_branch() {
            show_dialog(
                window,
                cx,
                DialogSpec::info(t("common.not_allowed"), t("warn.active_delete")),
            );
            return;
        }
        let folder = paths::version_folder(&game, &date, &branch);
        if !folder.exists() {
            show_dialog(
                window,
                cx,
                DialogSpec::info(
                    t("common.missing_folder"),
                    rust_i18n::t!("err.missing_folder", path = folder.display().to_string())
                        .to_string(),
                ),
            );
            return;
        }
        let message = rust_i18n::t!(
            "dlg.confirm_delete",
            date = date,
            branch = branch,
            path = folder.display().to_string()
        )
        .to_string();
        let weak = self.weak.clone();
        show_dialog(
            window,
            cx,
            DialogSpec::confirm(
                t("dlg.confirm_delete_title"),
                message,
                move |_event, window, cx| {
                    if let Err(err) = std::fs::remove_dir_all(&folder) {
                        show_dialog(
                            window,
                            cx,
                            DialogSpec::info(t("common.delete_failed"), format!("{err:#}")),
                        );
                        return;
                    }
                    weak.update(cx, |view, cx| view.refresh_version_list(cx))
                        .ok();
                },
            ),
        );
    }

    // endregion

    // region: setup / revert

    fn on_setup_or_revert(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.managed {
            self.revert_folders(window, cx);
        } else {
            self.setup_folders(window, cx);
        }
    }

    fn setup_folders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.current_game().cloned() else {
            return;
        };
        let active = paths::active_folder(&game);
        let steam = paths::steam_folder(&game);
        let common = paths::common_dir(&game);

        if !active.is_dir() {
            if steam.is_dir() {
                match junctions::create_junction(&active, &steam) {
                    Ok(()) => self.refresh_all(window, cx),
                    Err(err) => show_dialog(
                        window,
                        cx,
                        DialogSpec::info(
                            t("common.setup_failed"),
                            rust_i18n::t!("err.setup_junction", error = err.to_string())
                                .to_string(),
                        ),
                    ),
                }
                return;
            }
            show_dialog(
                window,
                cx,
                DialogSpec::info(
                    t("common.setup_failed"),
                    rust_i18n::t!(
                        "dlg.setup_game_missing",
                        path = active.display().to_string()
                    )
                    .to_string(),
                ),
            );
            return;
        }
        if steam.exists() {
            show_dialog(
                window,
                cx,
                DialogSpec::info(
                    t("common.setup_warning"),
                    rust_i18n::t!(
                        "dlg.setup_both_exist",
                        active = active.display().to_string(),
                        steam = steam.display().to_string()
                    )
                    .to_string(),
                ),
            );
            return;
        }

        let message = rust_i18n::t!("dlg.setup_message", installdir = game.installdir).to_string();
        let weak = self.weak.clone();
        let installdir = game.installdir.clone();
        show_dialog(
            window,
            cx,
            DialogSpec::confirm(
                t("app.setup_folders"),
                message,
                move |_event, window, cx| match junctions::rename_dir(&active, &steam) {
                    Err(_) => {
                        show_dialog(
                            window,
                            cx,
                            DialogSpec::info(
                                t("common.manual_step"),
                                rust_i18n::t!(
                                    "dlg.manual_step",
                                    installdir = installdir,
                                    common = common.display().to_string()
                                )
                                .to_string(),
                            ),
                        );
                        cx.reveal_path(&common);
                    }
                    Ok(()) => match junctions::create_junction(&active, &steam) {
                        Err(err) => {
                            let _ = junctions::rename_dir(&steam, &active);
                            show_dialog(
                                window,
                                cx,
                                DialogSpec::info(
                                    t("common.setup_failed"),
                                    rust_i18n::t!("err.junction_failed", error = err.to_string())
                                        .to_string(),
                                ),
                            );
                            cx.reveal_path(&common);
                        }
                        Ok(()) => {
                            weak.update(cx, |view, cx| view.refresh_after_flow(cx)).ok();
                        }
                    },
                },
            ),
        );
    }

    fn revert_folders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(game) = self.current_game().cloned() else {
            return;
        };
        let active = paths::active_folder(&game);
        let steam = paths::steam_folder(&game);
        let common = paths::common_dir(&game);
        let game_cfg = self.config.game_config(&game.app_id);
        let active_date = game_cfg.active_date().to_string();
        let installdir = game.installdir.clone();

        let message = rust_i18n::t!("dlg.revert_message", installdir = installdir).to_string();
        let weak = self.weak.clone();
        show_dialog(
            window,
            cx,
            DialogSpec::confirm(
                t("app.revert_folders"),
                message,
                move |_event, window, cx| {
                    if active_date != STEAM_VERSION {
                        if !steam.is_dir() {
                            show_dialog(
                                window,
                                cx,
                                DialogSpec::info(
                                    t("common.revert_failed"),
                                    rust_i18n::t!(
                                        "dlg.revert_no_steam",
                                        path = steam.display().to_string()
                                    )
                                    .to_string(),
                                ),
                            );
                            return;
                        }
                        if let Err(err) = junctions::remove_junction(&active)
                            .and_then(|_| junctions::create_junction(&active, &steam))
                        {
                            show_dialog(
                                window,
                                cx,
                                DialogSpec::info(
                                    t("common.revert_failed"),
                                    rust_i18n::t!("err.revert_redirect", error = err.to_string())
                                        .to_string(),
                                ),
                            );
                            return;
                        }
                    }
                    if let Err(err) = junctions::remove_junction(&active) {
                        show_dialog(
                            window,
                            cx,
                            DialogSpec::info(
                                t("common.revert_failed"),
                                rust_i18n::t!("err.revert_remove", error = err.to_string())
                                    .to_string(),
                            ),
                        );
                        return;
                    }
                    if !steam.is_dir() {
                        show_dialog(
                            window,
                            cx,
                            DialogSpec::info(
                                t("common.revert_failed"),
                                rust_i18n::t!(
                                    "dlg.revert_no_folder",
                                    path = steam.display().to_string(),
                                    installdir = installdir
                                )
                                .to_string(),
                            ),
                        );
                        cx.reveal_path(&common);
                        return;
                    }
                    if let Err(err) = junctions::rename_dir(&steam, &active) {
                        show_dialog(
                            window,
                            cx,
                            DialogSpec::info(
                                t("common.revert_failed"),
                                rust_i18n::t!(
                                    "err.rename_failed",
                                    error = err.to_string(),
                                    installdir = installdir
                                )
                                .to_string(),
                            ),
                        );
                        cx.reveal_path(&common);
                        return;
                    }
                    show_dialog(
                        window,
                        cx,
                        DialogSpec::info(t("common.revert_complete"), t("info.revert_complete")),
                    );
                    weak.update(cx, |view, cx| view.refresh_after_flow(cx)).ok();
                },
            ),
        );
    }

    // endregion

    // region: scraper

    fn on_update_manifests(
        &mut self,
        _: &gpui_kit::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(game) = self.current_game().cloned() else {
            return;
        };
        if !self.managed {
            return;
        }
        let depots_list = game
            .depots
            .iter()
            .map(|d| format!("  {d}"))
            .collect::<Vec<_>>()
            .join("\n");
        let message = rust_i18n::t!(
            "scraper.confirm_message",
            count = game.depots.len(),
            name = game.name,
            depots = depots_list
        )
        .to_string();
        let weak = self.weak.clone();
        show_dialog(
            window,
            cx,
            DialogSpec::confirm(
                t("scraper.confirm_title"),
                message,
                move |_event, window, cx| {
                    let browsers = scraper::detect_browsers();
                    match browsers.len() {
                        0 => show_dialog(
                            window,
                            cx,
                            DialogSpec::info(t("common.error"), t("scraper.no_browsers")),
                        ),
                        1 => start_scrape(&weak, browsers.into_iter().next().unwrap(), window, cx),
                        _ => BrowserSelectView::open(weak.clone(), browsers, window, cx),
                    }
                },
            ),
        );
    }

    pub fn on_scraper_finished(&mut self, cx: &mut Context<Self>) {
        if let Some(game) = self.current_game().cloned() {
            self.manifests = load_manifests(&self.paths.data_dir, &game.app_id);
            self.manifest_sizes = load_manifest_sizes(&self.paths.data_dir, &game.app_id);
        }
        self.refresh_version_list(cx);
    }

    // endregion
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.needs_select_rebuild {
            self.needs_select_rebuild = false;
            self.rebuild_game_select(window, cx);
        }
        let has_game = self.current_app.is_some();
        let managed = self.managed;
        let background = cx.theme().background;
        let border = cx.theme().border;
        let muted = cx.theme().muted_foreground;
        let foreground = cx.theme().foreground;

        let view = cx.entity();

        // Steamapps folders list
        let folders_view = view.clone();
        let folders_hover = cx.theme().list_hover;
        let folders_selection = cx.theme().selection;
        let folder_count = self.config.steamapps_folders.len();
        let folders_list = uniform_list("folders", folder_count, move |range, _window, cx| {
            let this = folders_view.read(cx);
            range
                .map(|ix| {
                    let selected = this.folder_selected == Some(ix);
                    let row_view = folders_view.clone();
                    let folder = this.config.steamapps_folders[ix].clone();
                    div()
                        .id(ix)
                        .w_full()
                        .h(px(20.))
                        .px_2()
                        .flex()
                        .items_center()
                        .rounded_sm()
                        .text_size(px(12.))
                        .text_color(foreground)
                        .bg(if selected {
                            folders_selection
                        } else {
                            hsla(0., 0., 0., 0.)
                        })
                        .hover(move |style| {
                            if selected {
                                style.bg(folders_selection)
                            } else {
                                style.bg(folders_hover)
                            }
                        })
                        .on_click(move |_, _window, cx| {
                            row_view.update(cx, |v, cx| {
                                v.folder_selected = Some(ix);
                                cx.notify();
                            });
                        })
                        .child(SharedString::from(folder))
                })
                .collect()
        })
        .flex_1()
        .min_h_0();

        // Version list
        let versions_view = view.clone();
        let versions_hover = cx.theme().list_hover;
        let versions_selection = cx.theme().selection;
        let success = cx.theme().success;
        let info = cx.theme().info;
        let version_count = self.versions.len();
        let versions_list = uniform_list("versions", version_count, move |range, _window, cx| {
            let this = versions_view.read(cx);
            range
                .filter_map(|ix| {
                    let row = this.versions.get(ix)?;
                    let selected = this.selected_version == Some(ix);
                    let row_view = versions_view.clone();
                    let label = row.label.clone();
                    let color = if row.active {
                        success
                    } else if row.downloaded {
                        info
                    } else {
                        foreground
                    };
                    Some(
                        div()
                            .id(ix)
                            .w_full()
                            .h(px(26.))
                            .px_2()
                            .flex()
                            .items_center()
                            .rounded_sm()
                            .text_color(color)
                            .bg(if selected {
                                versions_selection
                            } else {
                                hsla(0., 0., 0., 0.)
                            })
                            .hover(move |style| {
                                if selected {
                                    style.bg(versions_selection)
                                } else {
                                    style.bg(versions_hover)
                                }
                            })
                            .on_click(move |_, _window, cx| {
                                row_view.update(cx, |v, cx| {
                                    v.selected_version = Some(ix);
                                    cx.notify();
                                });
                            })
                            .child(label),
                    )
                })
                .collect()
        })
        .flex_1()
        .min_h_0();

        let beta_label = self
            .current_game()
            .filter(|g| g.has_beta())
            .map(|g| format!("beta: {}", g.beta))
            .unwrap_or_default();

        let game_select_el = match &self.game_select {
            Some(state) => Select::new(state).w_full().into_any_element(),
            None => div().flex_1().into_any_element(),
        };
        let language_select_el = match &self.language_select {
            Some(state) => div()
                .w(px(110.))
                .child(Select::new(state).small())
                .into_any_element(),
            None => div().w(px(110.)).into_any_element(),
        };

        // Bottom action buttons
        let download_menu_weak = self.weak.clone();
        let download_button = Button::new("download")
            .label(t("app.download_version"))
            .small()
            .disabled(!managed)
            .dropdown_menu(move |menu: PopupMenu, _window, _cx| {
                let console_weak = download_menu_weak.clone();
                let dd_weak = download_menu_weak.clone();
                menu.item(PopupMenuItem::new(t("app.using_steam_console")).on_click(
                    move |_, window, cx| {
                        console_weak
                            .update(cx, |v, cx| v.begin_download(window, cx, true))
                            .ok();
                    },
                ))
                .item(
                    PopupMenuItem::new(t("app.using_depot_downloader")).on_click(
                        move |_, window, cx| {
                            dd_weak
                                .update(cx, |v, cx| v.begin_download(window, cx, false))
                                .ok();
                        },
                    ),
                )
            });

        v_flex()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .p_3()
            .gap_2()
            // Steamapps folders row
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(140.)).child(t("app.folders_label")))
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_h_0()
                            .h(px(44.))
                            .rounded_sm()
                            .bg(cx.theme().input)
                            .border_1()
                            .border_color(cx.theme().border)
                            .overflow_hidden()
                            .child(folders_list),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                Button::new("add-folder")
                                    .label(t("common.add"))
                                    .small()
                                    .on_click(cx.listener(Self::on_add_folder)),
                            )
                            .child(
                                Button::new("remove-folder")
                                    .label(t("common.remove"))
                                    .small()
                                    .on_click(cx.listener(Self::on_remove_folder)),
                            ),
                    )
                    .child(language_select_el),
            )
            // Game row
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(140.)).child(t("app.game")))
                    .child(game_select_el)
                    .child(
                        div()
                            .italic()
                            .text_color(muted)
                            .child(SharedString::from(beta_label)),
                    ),
            )
            // Depot Downloader header
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .mt_1()
                    .child(
                        div()
                            .italic()
                            .text_size(px(12.))
                            .text_color(muted)
                            .child(t("app.depot_downloader")),
                    )
                    .child(div().flex_1().h(px(1.)).bg(border)),
            )
            // Credentials row
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().w(px(140.)).child(t("app.username_label")))
                    .child(Input::new(&self.username_input).small().w(px(220.)))
                    .child(
                        Checkbox::new("remember")
                            .checked(self.remember_password)
                            .label(t("app.remember_password"))
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                this.remember_password = *checked;
                                cx.notify();
                            })),
                    ),
            )
            .child(div().h(px(1.)).w_full().bg(border))
            // Version list header
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .child(div().child(t("app.available_versions")))
                    .child(
                        Checkbox::new("sort")
                            .checked(self.sort_downloaded)
                            .label(t("app.sort_by_downloaded"))
                            .on_change(cx.listener(|this, checked: &bool, _, cx| {
                                this.sort_downloaded = *checked;
                                this.refresh_version_list(cx);
                            })),
                    ),
            )
            .child(versions_list)
            .child(div().h(px(1.)).w_full().bg(border))
            // Action buttons
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(download_button)
                            .child(
                                Button::new("set-active")
                                    .label(t("app.set_active_version"))
                                    .small()
                                    .disabled(!managed)
                                    .on_click(cx.listener(Self::on_set_active)),
                            )
                            .child(
                                Button::new("delete-version")
                                    .label(t("app.delete_version"))
                                    .small()
                                    .disabled(!managed)
                                    .on_click(cx.listener(Self::on_delete_version)),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("update-manifests")
                                    .label(t("app.update_manifests"))
                                    .small()
                                    .disabled(!managed)
                                    .on_click(cx.listener(Self::on_update_manifests)),
                            )
                            .child(
                                Button::new("setup-folders")
                                    .label(if managed {
                                        t("app.revert_folders")
                                    } else {
                                        t("app.setup_folders")
                                    })
                                    .small()
                                    .disabled(!has_game)
                                    .on_click(cx.listener(Self::on_setup_or_revert)),
                            ),
                    ),
            )
    }
}
