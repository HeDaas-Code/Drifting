//! Named versions of a whole project from Project Home.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use futures::{FutureExt as _, channel::oneshot};
use gpui::{App, Global};

use dr_export::book_file::{BookFileError, versions};
use dr_i18n::keys::{book_versions as copy, common};
use dr_shell::{Workspace, book::Book, dialogs, release, strings::tf, t};
use dr_store::versions::BookVersion;
use dr_ui::{
    Theme,
    components::{Button, Dialog},
    input::field::{FieldEvent, TextField},
};
use dr_workspace::{WeakWorkspace, Workspace as Library};
use gpui::{
    AnyElement, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement as _,
    Render, StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px,
};

pub fn entry(
    home: &super::v3::Home,
    book: &Book,
    cx: &mut Context<super::v3::Home>,
) -> impl IntoElement {
    let lib = home.lib.as_ref().expect("checked by Home").downgrade();
    let ws = home.ws.clone();
    let book = book.id.clone();
    Button::new("home-book-versions", t(cx, copy::TITLE)).on_click(move |_, window, cx| {
        if release::is_releasing(cx) {
            return;
        }
        let panel = cx.new(|cx| Versions::new(lib.clone(), ws.clone(), book.clone(), window, cx));
        dialogs::show_in(window.window_handle(), cx, panel.into());
    })
}

#[derive(Default)]
struct Jobs(Arc<Mutex<InFlight>>);
#[derive(Default)]
struct InFlight {
    active: usize,
    waiters: Vec<oneshot::Sender<()>>,
    cancels: Vec<std::sync::Weak<AtomicBool>>,
}
impl Global for Jobs {}

struct Lease {
    jobs: Arc<Mutex<InFlight>>,
    library: Option<Library>,
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.library.take();
        let mut state = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        state.active -= 1;
        if state.active == 0 {
            for tx in state.waiters.drain(..) {
                let _ = tx.send(());
            }
        }
    }
}

fn begin_job(lib: Library, cancel: &Arc<AtomicBool>, cx: &mut App) -> Lease {
    if !cx.has_global::<Jobs>() {
        cx.set_global(Jobs::default());
        release::before_release(cx, |cx| {
            let hub = cx.global::<Jobs>().0.clone();
            let mut state = hub.lock().unwrap_or_else(|e| e.into_inner());
            for cancel in state.cancels.iter().filter_map(std::sync::Weak::upgrade) {
                cancel.store(true, Ordering::Relaxed);
            }
            if state.active == 0 {
                return async { Ok(()) }.boxed_local();
            }
            let (tx, rx) = oneshot::channel();
            state.waiters.push(tx);
            async move {
                rx.await
                    .map_err(|_| "book version operation disappeared".into())
            }
            .boxed_local()
        });
    }
    let jobs = cx.global::<Jobs>().0.clone();
    {
        let mut state = jobs.lock().unwrap_or_else(|e| e.into_inner());
        state.active += 1;
        state.cancels.retain(|cancel| cancel.strong_count() != 0);
        state.cancels.push(Arc::downgrade(cancel));
    }
    Lease {
        jobs,
        library: Some(lib),
    }
}

enum Job {
    List,
    Save(String),
    Compare {
        selected: String,
        earlier: Option<String>,
    },
    Restore(String),
    Delete(String),
}

enum Done {
    Listed(Vec<BookVersion>),
    Compared(versions::Changes),
    Restored(String),
}

impl Job {
    fn run(self, ws: &Library, book: &str, cancel: &AtomicBool) -> Result<Done, BookFileError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(BookFileError::Cancelled);
        }
        match self {
            Self::List => Ok(Done::Listed(versions::list(ws, book)?)),
            Self::Save(name) => {
                versions::save(ws, book, &name)?;
                Ok(Done::Listed(versions::list(ws, book)?))
            }
            Self::Compare { selected, earlier } => Ok(Done::Compared(versions::compare(
                ws,
                book,
                &selected,
                earlier.as_deref(),
            )?)),
            Self::Restore(id) => Ok(Done::Restored(
                versions::restore(ws, book, &id, &|_| {}, &|| cancel.load(Ordering::Relaxed))?.book,
            )),
            Self::Delete(id) => {
                versions::delete(ws, book, &id)?;
                Ok(Done::Listed(versions::list(ws, book)?))
            }
        }
    }
}

struct Versions {
    lib: WeakWorkspace,
    ws: WeakEntity<Workspace>,
    book: String,
    name: gpui::Entity<TextField>,
    rows: Vec<BookVersion>,
    selected: Option<String>,
    summary: Option<versions::Changes>,
    confirm_delete: Option<String>,
    busy: bool,
    failed: bool,
    cancel_restore: Option<Arc<AtomicBool>>,
    _field: Subscription,
}

impl Drop for Versions {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancel_restore {
            cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl Versions {
    fn new(
        lib: WeakWorkspace,
        ws: WeakEntity<Workspace>,
        book: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = t(cx, copy::NAME);
        let name =
            cx.new(|cx| TextField::new("book-version-name", window, cx).placeholder(placeholder));
        let sub = cx.subscribe_in(&name, window, |panel, _, event, window, cx| match event {
            FieldEvent::Submitted(_) => panel.save(window, cx),
            FieldEvent::Changed(_) => cx.notify(),
            _ => {}
        });
        let mut panel = Self {
            lib,
            ws,
            book,
            name,
            rows: Vec::new(),
            selected: None,
            summary: None,
            confirm_delete: None,
            busy: false,
            failed: false,
            cancel_restore: None,
            _field: sub,
        };
        panel.run(Job::List, window, cx);
        panel
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if !self.busy {
            let id = cx.entity_id();
            dialogs::hide(cx, id);
        }
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).text().trim().to_owned();
        if !name.is_empty() {
            self.run(Job::Save(name), window, cx);
        }
    }

    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(row) = self.rows.get(index) else {
            return;
        };
        self.selected = Some(row.id.clone());
        self.summary = None;
        self.confirm_delete = None;
        self.run(
            Job::Compare {
                selected: row.id.clone(),
                earlier: self.rows.get(index + 1).map(|v| v.id.clone()),
            },
            window,
            cx,
        );
    }

    fn run(&mut self, job: Job, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || release::is_releasing(cx) {
            return;
        }
        let Some(lib) = self.lib.upgrade() else {
            return;
        };
        let instance = lib.instance_id();
        let book = self.book.clone();
        self.failed = false;
        self.busy = true;
        let saving = matches!(&job, Job::Save(_));
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel_restore = matches!(&job, Job::Restore(_)).then(|| cancel.clone());
        let settled = saving.then(|| release::settle(cx));
        let lease = begin_job(lib, &cancel, cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = if let Some(settled) = settled
                && let Err(error) = settled.await
            {
                Err(BookFileError::Damaged(error))
            } else {
                cx.background_spawn(async move {
                    job.run(
                        lease.library.as_ref().expect("active version job"),
                        &book,
                        &cancel,
                    )
                })
                .await
            };
            this.update_in(cx, |panel, window, cx| {
                panel.busy = false;
                panel.cancel_restore = None;
                match result {
                    Ok(Done::Listed(rows)) => {
                        panel.rows = rows;
                        panel.summary = None;
                        panel.selected = None;
                        panel.confirm_delete = None;
                        if saving {
                            panel.name.update(cx, |name, cx| name.set_text("", cx));
                        }
                    }
                    Ok(Done::Compared(changes)) => panel.summary = Some(changes),
                    Ok(Done::Restored(id)) => {
                        let view = cx.entity_id();
                        panel
                            .ws
                            .update(cx, |ws, cx| {
                                if ws
                                    .dialog
                                    .as_ref()
                                    .is_some_and(|dialog| dialog.entity_id() == view)
                                    && ws
                                        .library
                                        .as_ref()
                                        .is_some_and(|lib| lib.instance_id() == instance)
                                {
                                    ws.dialog = None;
                                    ws.open_book(&id, window, cx);
                                }
                            })
                            .ok();
                    }
                    Err(BookFileError::Cancelled) => {}
                    Err(error) => {
                        log::warn!("project versions: {error}");
                        panel.failed = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

impl Render for Versions {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Theme::of(window, cx).palette;
        let me = cx.entity();
        let list = div()
            .id("book-versions-list")
            .max_h(px(240.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(4.))
            .when(self.rows.is_empty() && !self.busy, |el| {
                el.child(t(cx, copy::EMPTY))
            })
            .children(self.rows.iter().enumerate().map(|(index, row)| {
                let me = me.clone();
                let stamp = chrono::DateTime::from_timestamp_millis(row.created_at)
                    .map(|at| {
                        at.with_timezone(&chrono::Local)
                            .format("%Y-%m-%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default();
                let selected = self.selected.as_ref() == Some(&row.id);
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        Button::new(("book-version-select", index), row.name.clone())
                            .variant(if selected {
                                dr_ui::components::Variant::Primary
                            } else {
                                dr_ui::components::Variant::Secondary
                            })
                            .truncate()
                            .disabled(self.busy)
                            .on_click(move |_, window, cx| {
                                me.update(cx, |panel, cx| panel.select(index, window, cx))
                            }),
                    )
                    .child(div().text_color(p.ink(3).solid()).child(stamp))
            }));
        let save = me.clone();
        let actions = self.selected.as_ref().map(|id| {
            let restoring = me.clone();
            let deleting = me.clone();
            let restore_id = id.clone();
            let delete_id = id.clone();
            let confirm = self.confirm_delete.as_ref() == Some(id);
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .child(
                    Button::new("book-version-restore", t(cx, copy::RESTORE))
                        .disabled(self.busy || self.summary.is_none())
                        .on_click(move |_, window, cx| {
                            restoring.update(cx, |panel, cx| {
                                panel.run(Job::Restore(restore_id.clone()), window, cx)
                            })
                        }),
                )
                .child(
                    Button::new(
                        "book-version-delete",
                        t(
                            cx,
                            if confirm {
                                copy::CONFIRM_DELETE
                            } else {
                                common::DELETE
                            },
                        ),
                    )
                    .danger()
                    .disabled(self.busy)
                    .on_click(move |_, window, cx| {
                        deleting.update(cx, |panel, cx| {
                            if confirm {
                                panel.run(Job::Delete(delete_id.clone()), window, cx);
                            } else {
                                panel.confirm_delete = Some(delete_id.clone());
                                cx.notify();
                            }
                        })
                    }),
                )
        });
        let summary: Option<AnyElement> = self.summary.as_ref().map(|summary| {
            div()
                .child(tf(
                    cx,
                    copy::PROSE_CHANGES,
                    &[
                        ("added", &summary.prose_added),
                        ("changed", &summary.prose_changed),
                        ("removed", &summary.prose_removed),
                    ],
                ))
                .child(div().child(t(
                    cx,
                    if summary.structure_changed {
                        copy::STRUCTURE_CHANGED
                    } else {
                        copy::STRUCTURE_SAME
                    },
                )))
                .child(div().child(tf(
                    cx,
                    copy::ASSETS_CHANGED,
                    &[("count", &summary.asset_files_changed)],
                )))
                .into_any_element()
        });
        Dialog::new("book-versions", t(cx, copy::TITLE))
            .body(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().child(self.name.clone()))
                    .child(
                        Button::new("book-version-save", t(cx, copy::SAVE))
                            .primary()
                            .busy(self.busy)
                            .disabled(self.busy || self.name.read(cx).text().trim().is_empty())
                            .on_click(move |_, window, cx| {
                                save.update(cx, |panel, cx| panel.save(window, cx))
                            }),
                    ),
            )
            .body(list)
            .body(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .children(summary)
                    .children(actions)
                    .when(self.failed, |el| {
                        el.child(
                            div()
                                .text_color(p.danger.solid())
                                .child(t(cx, copy::FAILED)),
                        )
                    }),
            )
            .action(
                Button::new("book-version-close", t(cx, common::CLOSE))
                    .disabled(self.busy)
                    .on_click({
                        let me = me.clone();
                        move |_, _, cx| me.update(cx, |panel, cx| panel.close(cx))
                    }),
            )
            .action(
                Button::new("book-version-cancel", t(cx, common::CANCEL))
                    .disabled(self.cancel_restore.is_none())
                    .on_click({
                        let cancel = self.cancel_restore.clone();
                        move |_, _, _| {
                            if let Some(cancel) = &cancel {
                                cancel.store(true, Ordering::Relaxed);
                            }
                        }
                    }),
            )
            .on_close(move |_, cx| me.update(cx, |panel, cx| panel.close(cx)))
            .on_mist(|_, _| {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dr_shell::{Services, SettingsFile, ShellInfo};
    use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
    use std::time::{Duration, Instant};

    fn wait(
        cx: &mut VisualTestContext,
        label: &str,
        done: impl Fn(&mut VisualTestContext) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            cx.executor().advance_clock(Duration::from_millis(30));
            cx.run_until_parked();
            cx.update(|window, app| window.draw(app).clear(app));
            if done(cx) {
                return;
            }
            assert!(Instant::now() < deadline, "{label}");
            std::thread::sleep(Duration::from_millis(3));
        }
    }

    fn panel(workspace: &Entity<Workspace>, cx: &VisualTestContext) -> Option<Entity<Versions>> {
        workspace.read_with(cx, |ws, _| ws.dialog.clone()?.downcast::<Versions>().ok())
    }

    #[gpui::test]
    fn home_entry_saves_compares_and_restores_a_version_as_a_new_project(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let temp = tempfile::tempdir().unwrap();
        let library =
            dr_store::Library::open(temp.path(), dr_workspace::library_options("test")).unwrap();
        let seed = dr_testkit::seed::generate_v3(
            dr_testkit::seed::Preset::Tiny,
            dr_testkit::seed::DEFAULT_SEED,
        )
        .unwrap();
        let book = seed.books[0].book_id().to_string();
        seed.write_to(&library).unwrap();
        library.close().unwrap();
        let lib = Library::open_dir(temp.path(), "test").unwrap();
        cx.update(|cx| {
            dr_shell::init(
                ShellInfo {
                    app_name: "Drifting Next",
                    bundle_id: "cc.drifting.next.dev",
                    version: "test",
                    url_scheme: "drifting-next-dev",
                    forced_appearance: None,
                },
                Services {
                    library: Some(lib.clone()),
                    book: Some(book.clone()),
                    ..Default::default()
                },
                SettingsFile::in_memory(),
                cx,
            );
            crate::init(cx);
        });
        let (workspace, cx) = cx.add_window_view(Workspace::new);
        wait(cx, "Project Home opens version entry", |cx| {
            cx.debug_bounds("home-book-versions").is_some()
        });
        let target = cx.debug_bounds("home-book-versions").unwrap();
        cx.simulate_click(target.center(), Modifiers::none());
        wait(cx, "version list opens", |cx| {
            panel(&workspace, cx).is_some_and(|panel| !panel.read_with(cx, |p, _| p.busy))
        });
        let versions = panel(&workspace, cx).unwrap();
        versions.update_in(cx, |panel, window, cx| {
            panel
                .name
                .update(cx, |name, cx| name.set_text("Before revision", cx));
            panel.save(window, cx);
        });
        wait(cx, "named version is saved", |cx| {
            versions.read_with(cx, |p, _| p.rows.len() == 1 && !p.busy)
        });
        versions.update_in(cx, |panel, window, cx| panel.select(0, window, cx));
        wait(cx, "version summary is read", |cx| {
            versions.read_with(cx, |p, _| p.summary.is_some() && !p.busy)
        });
        let id = versions.read_with(cx, |p, _| p.rows[0].id.clone());
        versions.update_in(cx, |panel, window, cx| {
            panel.run(Job::Restore(id), window, cx)
        });
        wait(cx, "restored copy opens", |cx| {
            workspace.read_with(cx, |ws, _| {
                ws.dialog.is_none()
                    && !ws.book_id.is_empty()
                    && ws.book_id != book
                    && ws.book.id == ws.book_id
            })
        });
        assert_eq!(versions::list(&lib, &book).unwrap().len(), 1);
        // The original project remains independently readable.
        assert!(lib.observe(&format!("book:{book}")).is_ok());
        cx.update(|_, cx| {
            cx.global_mut::<Services>().library = None;
            dr_view_stats::release(cx);
        });
        lib.close();
    }

    #[gpui::test]
    fn version_worker_lease_releases_library_before_its_shutdown_waiter(cx: &mut TestAppContext) {
        let temp = tempfile::tempdir().unwrap();
        let lib = Library::open_dir(temp.path(), "test").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let lease = cx.update(|cx| begin_job(lib.clone(), &cancel, cx));
        let state = lease.jobs.clone();
        let (tx, mut rx) = oneshot::channel();
        state.lock().unwrap().waiters.push(tx);
        assert_eq!(state.lock().unwrap().active, 1);
        drop(lease);
        assert_eq!(state.lock().unwrap().active, 0);
        assert_eq!(rx.try_recv().unwrap(), Some(()));
        lib.shutdown().unwrap();
        let reopened = Library::open_dir(temp.path(), "test").unwrap();
        reopened.shutdown().unwrap();
    }
}
