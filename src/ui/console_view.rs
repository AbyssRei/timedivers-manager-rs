use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::dialogs::{DialogSpec, show_dialog};
use gpui_kit::component::{
    ActiveTheme, Disableable as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariant, ButtonVariants as _},
    notification::Notification,
    v_flex,
};
use gpui_kit::*;

use crate::constants::NONE_BRANCH;
use crate::download::depot_watch::{
    DepotExpectation, DepotPhase, DepotSnapshot, DepotWatch, DepotWatcher, snapshot_depot,
};
use crate::download::steam_console;
use crate::manifest::ManifestEntry;
use crate::util::window_options;

enum ConsoleMsg {
    Progress(usize, usize),
    Done(anyhow::Result<()>),
}

/// Everything the import window needs for one version download session.
pub struct ConsoleSession {
    pub app_view: gpui_kit::WeakEntity<super::main_view::AppView>,
    pub game_name: String,
    pub app_id: String,
    pub depots: Vec<String>,
    pub version_date: String,
    pub branch: String,
    pub manifest_entry: ManifestEntry,
    pub steamapps_folders: Vec<String>,
    pub version_folder: PathBuf,
    pub data_dir: PathBuf,
    pub manifest_sizes: crate::manifest::ManifestSizes,
}

/// The Steam Console import window (740x520 modal), mirroring
/// `SteamConsoleWindow` with a live per-depot download state machine.
pub struct ConsoleView {
    app_id: String,
    depots: Vec<String>,
    manifest_entry: ManifestEntry,
    steamapps_folders: Vec<String>,
    version_folder: PathBuf,
    meta_path: PathBuf,
    app_view: gpui_kit::WeakEntity<super::main_view::AppView>,
    watcher: DepotWatcher,
    depot_watches: Vec<DepotWatch>,
    session_totals: BTreeMap<String, (u64, u64)>,
    console_states: BTreeMap<String, crate::steam::steam_log::ConsoleDepotDownload>,
    importing: bool,
    progress: Option<(usize, usize)>,
}

/// Linear sweeping light band used while a depot is downloading.
/// Decision: the shimmer sweeps the full track (it signals activity),
/// not just the filled portion.
fn progress_track(
    id: &'static str,
    fill: Option<f32>,
    active: bool,
    fill_color: gpui_kit::Hsla,
    track_color: gpui_kit::Hsla,
) -> impl IntoElement {
    let track = div()
        .id(id)
        .relative()
        .w_full()
        .h(px(6.))
        .rounded_full()
        .overflow_hidden()
        .bg(track_color);

    let track = match fill {
        Some(pct) => track.child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(relative(pct.clamp(0., 1.)))
                .rounded_full()
                .bg(fill_color),
        ),
        None => track,
    };

    if active {
        track
            .with_animation(
                gpui_kit::ElementId::Name(format!("shine-{id}").into()),
                gpui_kit::Animation::new(Duration::from_millis(1400))
                    .repeat()
                    .with_easing(|delta| delta),
                move |track, delta| {
                    track.child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .w(relative(0.3))
                            .left(relative((delta * 0.7).clamp(0., 1.)))
                            .rounded_full()
                            .bg(gpui_kit::linear_gradient(
                                90.,
                                gpui_kit::LinearColorStop {
                                    color: gpui_kit::transparent_white(),
                                    percentage: 0.,
                                },
                                gpui_kit::LinearColorStop {
                                    color: gpui_kit::white().opacity(0.35),
                                    percentage: 1.,
                                },
                            )),
                    )
                },
            )
            .into_any_element()
    } else {
        track.into_any_element()
    }
}

impl ConsoleView {
    pub fn open(session: ConsoleSession, cx: &mut App) {
        let ConsoleSession {
            app_view,
            game_name,
            app_id,
            depots,
            version_date,
            branch,
            manifest_entry,
            steamapps_folders,
            version_folder,
            data_dir,
            manifest_sizes,
        } = session;
        let branch_label = if branch != NONE_BRANCH {
            format!(" [{branch}]")
        } else {
            String::new()
        };
        let title = format!("Steam Console Import — {game_name}{branch_label} {version_date}");
        let meta_path = data_dir.join("depot_meta").join(format!("{app_id}.ron"));

        // Take two quick rounds of snapshots so already-downloaded depots show
        // Complete immediately while mid-download ones enter Downloading.
        let content = steam_console::find_content_folder(&steamapps_folders, &app_id);
        let snap = |depot: &str| -> DepotSnapshot {
            let dir = content
                .as_ref()
                .map(|c| steam_console::depot_cache_dir(c, &app_id, depot))
                .unwrap_or_default();
            snapshot_depot(&dir)
        };
        // Expected size resolution per depot, most certain first:
        // 1. learned metadata imported from the same manifest (size + files),
        // 2. the download size SteamDB reports for the manifest (size only,
        //    matching what the Steam console itself displays),
        // 3. no expectation — the stability fallback decides completion.
        let meta = steam_console::load_depot_meta(&meta_path);
        let expected: BTreeMap<String, crate::download::depot_watch::DepotExpectation> = depots
            .iter()
            .filter_map(|depot| {
                let manifest_id = manifest_entry.get(depot)?;
                if let Some(learned) = meta
                    .get(depot)
                    .filter(|learned| learned.manifest.as_deref() == Some(manifest_id.as_str()))
                {
                    return Some((
                        depot.clone(),
                        crate::download::depot_watch::DepotExpectation {
                            bytes: learned.bytes,
                            files: Some(learned.files),
                        },
                    ));
                }
                let size = manifest_sizes.get(manifest_id)?;
                Some((
                    depot.clone(),
                    crate::download::depot_watch::DepotExpectation {
                        bytes: *size,
                        files: None,
                    },
                ))
            })
            .collect();
        let mut watcher = DepotWatcher::new(expected);
        let initial: Vec<DepotSnapshot> = depots.iter().map(|d| snap(d)).collect();
        let t0 = Instant::now();
        watcher.seed(&depots, &initial, t0);
        let depot_watches = watcher.poll(&depots, &initial, t0 + Duration::from_millis(300));
        let (any_handle, view) = open_window(
            window_options(title, (120., 120.), (740., 520.), true),
            cx,
            |_, cx| {
                cx.new(|_| Self {
                    app_id,
                    depots,
                    manifest_entry,
                    steamapps_folders,
                    version_folder,
                    meta_path,
                    app_view,
                    watcher,
                    depot_watches,
                    session_totals: BTreeMap::new(),
                    console_states: BTreeMap::new(),
                    importing: false,
                    progress: None,
                })
            },
        )
        .expect("open steam console window");

        // Poll the depot cache folders once a second so the state machine and
        // progress bars track the Steam console download without any manual
        // refresh. Exits when the window is closed.
        cx.spawn(async move |cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let (dirs, app_id, depots) = view.read_with(cx, |view, _| {
                    (view.depot_dirs(), view.app_id.clone(), view.depots.clone())
                });
                let snaps = cx
                    .background_spawn(async move {
                        let snapshots: Vec<(String, DepotSnapshot)> = dirs
                            .into_iter()
                            .map(|(depot, dir)| (depot, snapshot_depot(&dir)))
                            .collect();
                        let session_totals =
                            crate::steam::steam_log::session_totals_for(&app_id, &depots);
                        let console_states =
                            crate::steam::steam_log::console_download_states_for(&app_id, &depots);
                        (snapshots, session_totals, console_states)
                    })
                    .await;
                let (snaps, session_totals, console_states) = snaps;
                let applied = any_handle.update(cx, |_root, _window, cx| {
                    view.update(cx, |view, cx| {
                        view.apply_poll(snaps, session_totals, console_states, cx);
                    })
                });
                if applied.is_err() {
                    break; // window closed
                }
            }
        })
        .detach();
    }

    fn content_folder(&self) -> Option<PathBuf> {
        steam_console::find_content_folder(&self.steamapps_folders, &self.app_id)
    }

    /// (depot id, cache dir) pairs; the dir is a placeholder while Steam has
    /// not created the content folder yet.
    fn depot_dirs(&self) -> Vec<(String, PathBuf)> {
        let content = self.content_folder();
        self.depots
            .iter()
            .map(|depot| {
                let dir = content
                    .as_ref()
                    .map(|c| steam_console::depot_cache_dir(c, &self.app_id, depot))
                    .unwrap_or_default();
                (depot.clone(), dir)
            })
            .collect()
    }

    fn apply_poll(
        &mut self,
        snaps: Vec<(String, DepotSnapshot)>,
        session_totals: BTreeMap<String, (u64, u64)>,
        console_states: BTreeMap<String, crate::steam::steam_log::ConsoleDepotDownload>,
        cx: &mut Context<Self>,
    ) {
        // Steam-log session totals supply each depot's staged (on-disk)
        // expectation — the exact denominator for our byte counts, even on
        // the first download of a depot. The stage size also gives a
        // deterministic size threshold for the state machine.
        for (depot, (download_total, stage_total)) in &session_totals {
            let entry = self
                .watcher
                .expected
                .entry(depot.clone())
                .or_insert(DepotExpectation {
                    bytes: *stage_total,
                    files: None,
                });
            entry.bytes = *stage_total;
            let _ = download_total;
        }
        // The console log provides the exact file count per depot — feed it
        // into the thresholds as well.
        for (depot, state) in &console_states {
            if let Some(entry) = self.watcher.expected.get_mut(depot) {
                entry.files = Some(state.files);
            }
        }
        self.session_totals = session_totals;
        self.console_states = console_states;
        let snapshot_list: Vec<DepotSnapshot> =
            snaps.iter().map(|(_, snapshot)| snapshot.clone()).collect();
        self.depot_watches = self
            .watcher
            .poll(&self.depots, &snapshot_list, Instant::now());
        // `Depot download complete` events are authoritative per-depot
        // completion — but they are historical log lines that persist after
        // the folder is deleted, so only honor them while the depot cache
        // still holds content.
        for watch in &mut self.depot_watches {
            if watch.bytes > 0
                && self
                    .console_states
                    .get(&watch.depot)
                    .is_some_and(|s| s.complete)
            {
                watch.phase = DepotPhase::Complete;
            }
        }
        cx.notify();
    }

    fn all_complete(&self) -> bool {
        !self.depot_watches.is_empty()
            && self
                .depot_watches
                .iter()
                .all(|w| w.phase == DepotPhase::Complete)
    }

    fn on_refresh(
        &mut self,
        _: &gpui_kit::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirs = self.depot_dirs();
        let snaps: Vec<(String, crate::download::depot_watch::DepotSnapshot)> = dirs
            .into_iter()
            .map(|(depot, dir)| (depot, snapshot_depot(&dir)))
            .collect();
        let session_totals =
            crate::steam::steam_log::session_totals_for(&self.app_id, &self.depots);
        let console_states =
            crate::steam::steam_log::console_download_states_for(&self.app_id, &self.depots);
        self.apply_poll(snaps, session_totals, console_states, cx);
    }

    fn on_open_console(
        &mut self,
        _: &gpui_kit::ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.open_url("steam://open/console");
    }

    fn on_copy(&mut self, text: &str, _window: &mut Window, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(text.to_string()));
    }

    fn on_delete_depot(&mut self, depot_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(content) = self.content_folder() else {
            return;
        };
        let path = steam_console::depot_cache_dir(&content, &self.app_id, depot_id);
        let message = rust_i18n::t!(
            "console.delete_depot_message",
            path = path.display().to_string()
        )
        .to_string();
        show_dialog(
            window,
            cx,
            DialogSpec::confirm(
                rust_i18n::t!("console.delete_depot_title").to_string(),
                message,
                move |_, window, cx| {
                    if let Err(err) = std::fs::remove_dir_all(&path) {
                        window.push_notification(
                            Notification::error(
                                rust_i18n::t!(
                                    "console.delete_depot_failed",
                                    error = err.to_string()
                                )
                                .to_string(),
                            ),
                            cx,
                        );
                    }
                },
            )
            .ok_text(rust_i18n::t!("common.delete").to_string())
            .ok_variant(ButtonVariant::Danger),
        );
    }

    fn on_import(&mut self, _: &gpui_kit::ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(content_folder) = self.content_folder() else {
            return;
        };
        if self.version_folder.exists() {
            let message = rust_i18n::t!(
                "console.import_dest_exists",
                dest = self.version_folder.display().to_string()
            )
            .to_string();
            show_dialog(
                window,
                cx,
                DialogSpec::info(rust_i18n::t!("console.import_failed").to_string(), message),
            );
            return;
        }

        self.importing = true;
        // Learn the exact size and file count of the freshly downloaded
        // depots (keyed by manifest id) so later downloads of the same
        // depot+manifest can complete deterministically.
        let learned: BTreeMap<String, steam_console::DepotMetaEntry> = self
            .depots
            .iter()
            .enumerate()
            .filter_map(|(ix, depot)| {
                self.depot_watches.get(ix).map(|w| {
                    (
                        depot.clone(),
                        steam_console::DepotMetaEntry {
                            bytes: w.bytes,
                            files: w.files,
                            manifest: self.manifest_entry.get(depot).cloned(),
                        },
                    )
                })
            })
            .collect();

        let (tx, rx) = async_channel::unbounded::<ConsoleMsg>();
        let content = content_folder.clone();
        let app_id = self.app_id.clone();
        let depots = self.depots.clone();
        let dest = self.version_folder.clone();
        let meta_path = self.meta_path.clone();

        std::thread::spawn(move || {
            let result =
                steam_console::import_version(&content, &app_id, &depots, &dest, |done, total| {
                    let _ = tx.try_send(ConsoleMsg::Progress(done, total));
                })
                .map(|_| {
                    let mut meta = steam_console::load_depot_meta(&meta_path);
                    for (depot, entry) in &learned {
                        meta.insert(depot.clone(), entry.clone());
                    }
                    let _ = steam_console::save_depot_meta(&meta_path, &meta);
                    steam_console::clear_depot_folders(&content, &app_id, &depots);
                });
            let _ = tx.try_send(ConsoleMsg::Done(result));
        });

        cx.spawn_in(window, async move |this, cx| {
            while let Ok(msg) = rx.recv().await {
                let outcome = match msg {
                    ConsoleMsg::Progress(done, total) => this
                        .update(cx, |view, cx| {
                            view.progress = Some((done, total));
                            cx.notify();
                        })
                        .map(|_| ()),
                    ConsoleMsg::Done(result) => this.update_in(cx, |view, window, cx| {
                        view.importing = false;
                        view.progress = None;
                        match result {
                            Ok(()) => {
                                view.app_view
                                    .update(cx, |app, cx| app.refresh_version_list(cx))
                                    .ok();
                                window.push_notification(
                                    Notification::success(
                                        rust_i18n::t!("console.import_complete").to_string(),
                                    ),
                                    cx,
                                );
                                window.remove_window();
                            }
                            Err(err) => {
                                cx.notify();
                                window.push_notification(
                                    Notification::error(format!(
                                        "{}: {err:#}",
                                        rust_i18n::t!("console.import_failed")
                                    )),
                                    cx,
                                );
                            }
                        }
                    }),
                };
                if outcome.is_err() {
                    break; // window closed
                }
            }
        })
        .detach();
        cx.notify();
    }
}

impl Render for ConsoleView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content_folder = self.content_folder();
        let all_complete = self.all_complete();
        let any_downloading = self
            .depot_watches
            .iter()
            .any(|w| w.phase == DepotPhase::Downloading);

        let mut depot_rows = v_flex().gap_3();
        for (ix, depot_id) in self.depots.iter().enumerate() {
            let watch = self.depot_watches.get(ix).cloned();
            let depot_id = depot_id.clone();

            // status line + mini progress bar
            let (status, status_color, fill, shimmer) = match &watch {
                None
                | Some(DepotWatch {
                    phase: DepotPhase::Pending,
                    ..
                }) => (
                    rust_i18n::t!("console.state_pending").to_string(),
                    cx.theme().muted_foreground,
                    None,
                    false,
                ),
                Some(DepotWatch {
                    phase: DepotPhase::Downloading,
                    bytes,
                    expected_bytes,
                    ..
                }) => {
                    // 分母 = Steam 日志的 stage 总量（与磁盘观测同单位）
                    let text = match expected_bytes {
                        Some(total) => rust_i18n::t!(
                            "console.state_downloading_full",
                            done = steam_console::format_bytes(*bytes),
                            total = steam_console::format_bytes(*total)
                        )
                        .to_string(),
                        None => format!(
                            "{} · {}",
                            rust_i18n::t!("console.state_downloading"),
                            steam_console::format_bytes(*bytes)
                        ),
                    };
                    (
                        text,
                        cx.theme().info,
                        expected_bytes.map(|total| (*bytes as f32 / total as f32).clamp(0., 1.)),
                        true,
                    )
                }
                Some(DepotWatch {
                    bytes,
                    files,
                    expected_files,
                    ..
                }) => {
                    // Steam 措辞：Downloaded X (Y bytes uncompressed)
                    let watch_ref = watch.as_ref().expect("watch present in Complete arm");
                    let download_total = self
                        .console_states
                        .get(&watch_ref.depot)
                        .map(|s| s.download_bytes);
                    let file_count = (*files).max((*expected_files).unwrap_or(*files));
                    let text = match download_total {
                        Some(dl) => rust_i18n::t!(
                            "console.state_complete_full",
                            dl = steam_console::format_bytes(dl),
                            disk = steam_console::format_bytes(*bytes),
                            files = file_count
                        )
                        .to_string(),
                        None => format!(
                            "{} · {} {}",
                            rust_i18n::t!("console.state_complete"),
                            steam_console::format_bytes(*bytes),
                            file_count
                        ),
                    };
                    (text, cx.theme().success, Some(1.), false)
                }
            };

            let mini_bar = progress_track(
                "depot-bar",
                fill,
                shimmer,
                cx.theme().success,
                cx.theme().input,
            );

            let row = gpui_kit::component::h_flex()
                .items_start()
                .gap_2()
                .child(
                    div()
                        .w(px(120.))
                        .pt(px(2.))
                        .child(SharedString::from(format!("Depot {depot_id}:"))),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(status_color)
                                .child(status),
                        )
                        .child(mini_bar),
                );

            let row = if matches!(
                watch,
                Some(DepotWatch {
                    phase: DepotPhase::Complete,
                    ..
                })
            ) {
                row.child(
                    Button::new(SharedString::from(format!("delete-{depot_id}")))
                        .label(rust_i18n::t!("common.delete").to_string())
                        .danger()
                        .small()
                        .disabled(self.importing)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.on_delete_depot(&depot_id, window, cx)
                        })),
                )
            } else {
                let manifest_id = self
                    .manifest_entry
                    .get(&depot_id)
                    .cloned()
                    .unwrap_or_else(|| "UNKNOWN".to_string());
                let command = SharedString::from(steam_console::download_command(
                    &self.app_id,
                    &depot_id,
                    &manifest_id,
                ));
                let command_for_copy = command.clone();
                row.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(cx.theme().input)
                        .border_1()
                        .border_color(cx.theme().border)
                        .font_family("Consolas")
                        .text_size(px(12.))
                        .overflow_hidden()
                        .child(command),
                )
                .child(
                    Button::new(SharedString::from(format!("copy-{depot_id}")))
                        .label(rust_i18n::t!("console.copy").to_string())
                        .small()
                        .disabled(self.importing)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.on_copy(&command_for_copy, window, cx)
                        })),
                )
            };
            depot_rows = depot_rows.child(row);
        }

        let header = match &content_folder {
            Some(folder) => {
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(SharedString::from(format!(
                        "{}: {}",
                        rust_i18n::t!("console.content"),
                        folder.display()
                    )))
            }
            None => div()
                .text_color(cx.theme().warning)
                .child(rust_i18n::t!("console.content_not_found").to_string()),
        };

        // Overall progress: byte total across depots; percentage only when
        // every depot has a known expected size.
        let total_bytes: u64 = self.depot_watches.iter().map(|w| w.bytes).sum();
        let expected_total: Option<u64> = {
            let expected: Vec<u64> = self
                .depot_watches
                .iter()
                .filter_map(|w| w.expected_bytes)
                .collect();
            (expected.len() == self.depots.len() && !expected.is_empty())
                .then(|| expected.iter().sum())
        };
        // 会话内各 depot 的 stage 总量（来自 Steam 日志）是磁盘字节数的
        // 精确分母；无会话数据时回退到学习/SteamDB 期望。
        let overall_denominator: Option<u64> = if self.session_totals.is_empty() {
            expected_total
        } else {
            Some(self.session_totals.values().map(|(_, stage)| stage).sum())
        };
        let overall_fill = overall_denominator
            .map(|expected| (total_bytes as f64 / expected.max(1) as f64).clamp(0., 1.) as f32);
        let overall_text = match overall_denominator {
            Some(expected) => format!(
                "{} / {}",
                steam_console::format_bytes(total_bytes),
                steam_console::format_bytes(expected)
            ),
            None => steam_console::format_bytes(total_bytes),
        };

        let overall_bar = progress_track(
            "overall",
            overall_fill,
            any_downloading,
            cx.theme().primary,
            cx.theme().input,
        );

        let progress = self.progress.map(|(done, total)| {
            let percent = (done as f32 / total.max(1) as f32) * 100.0;
            gpui_kit::component::progress::Progress::new("import").value(percent)
        });

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .p_3()
            .gap_2()
            .child(header)
            .child(div().h(px(1.)).w_full().bg(cx.theme().border))
            .child(
                gpui_kit::component::h_flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().child(overall_bar))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child(SharedString::from(format!(
                                "{} · {overall_text}",
                                rust_i18n::t!("console.total_downloaded")
                            ))),
                    ),
            )
            .child(
                div()
                    .id("depot-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(depot_rows),
            )
            .children(progress)
            .child(div().h(px(1.)).w_full().bg(cx.theme().border))
            .child(
                gpui_kit::component::h_flex()
                    .justify_between()
                    .child(
                        gpui_kit::component::h_flex()
                            .gap_2()
                            .child(
                                Button::new("open-console")
                                    .label(rust_i18n::t!("console.open_steam_console").to_string())
                                    .small()
                                    .disabled(self.importing)
                                    .on_click(cx.listener(Self::on_open_console)),
                            )
                            .child(
                                Button::new("refresh")
                                    .label(rust_i18n::t!("common.refresh").to_string())
                                    .small()
                                    .disabled(self.importing)
                                    .on_click(cx.listener(Self::on_refresh)),
                            ),
                    )
                    .child(
                        Button::new("import")
                            .label(rust_i18n::t!("console.import_version").to_string())
                            .primary()
                            .small()
                            .disabled(self.importing || !all_complete)
                            .on_click(cx.listener(Self::on_import)),
                    ),
            )
    }
}
