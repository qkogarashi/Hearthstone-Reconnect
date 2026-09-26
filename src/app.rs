use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use slint::{
    ComponentHandle, LogicalPosition, Model, ModelRc, PhysicalPosition, Timer, TimerMode, VecModel,
};

use crate::config::Config;
use crate::net::{Connection, Outcome, do_smart_reconnect, get_hs_connections, kill_connection};
use crate::uac::relaunch_as_admin;
use crate::{ConnRow, Feedback, MainWindow};

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Idle,
    Working,
    Done(Outcome),
}

impl Status {
    fn feedback(self) -> Feedback {
        match self {
            Status::Idle => Feedback::None,
            Status::Working => Feedback::Working,
            Status::Done(Outcome::Reconnecting) => Feedback::Reconnecting,
            Status::Done(Outcome::NotInGame) => Feedback::NotInGame,
            Status::Done(Outcome::NotRunning) => Feedback::NotRunning,
            Status::Done(Outcome::Failed) => Feedback::Failed,
        }
    }
}

struct Shared {
    running: bool,
    connections: Vec<Connection>,
    status: Status,
    status_at: Option<Instant>,
}

impl Shared {
    fn set_status(&mut self, status: Status) {
        self.status = status;
        self.status_at = Some(Instant::now());
    }
}

pub fn run(elevated: bool) -> Result<(), slint::PlatformError> {
    let config = Config::load();

    let ui = MainWindow::new()?;
    ui.set_elevated(elevated);
    ui.set_compact(config.compact);

    let shared = Arc::new(Mutex::new(Shared {
        running: false,
        connections: vec![],
        status: Status::Idle,
        status_at: None,
    }));

    {
        let shared = Arc::clone(&shared);
        thread::spawn(move || {
            loop {
                let conns = get_hs_connections();
                let mut s = shared.lock().unwrap();
                s.running = conns.is_some();
                s.connections = conns.unwrap_or_default();
                drop(s);
                thread::sleep(Duration::from_secs(2));
            }
        });
    }

    let show_all = Rc::new(Cell::new(false));
    let visible: Rc<RefCell<Vec<Connection>>> = Rc::default();
    let rows = Rc::new(VecModel::<ConnRow>::default());
    ui.set_rows(ModelRc::from(rows.clone()));

    let sync: Rc<dyn Fn()> = {
        let (ui, shared, show_all, visible, rows) = (
            ui.as_weak(),
            Arc::clone(&shared),
            Rc::clone(&show_all),
            Rc::clone(&visible),
            Rc::clone(&rows),
        );
        Rc::new(move || {
            let Some(ui) = ui.upgrade() else { return };
            let mut s = shared.lock().unwrap();

            if s.status != Status::Working
                && s.status_at
                    .is_some_and(|t| t.elapsed() > Duration::from_secs(4))
            {
                s.status = Status::Idle;
                s.status_at = None;
            }

            let shown: Vec<Connection> = s
                .connections
                .iter()
                .filter(|c| show_all.get() || c.is_game_server)
                .cloned()
                .collect();
            let new_rows: Vec<ConnRow> = shown
                .iter()
                .map(|c| ConnRow {
                    ip: c.remote_ip.to_string().into(),
                    port: c.remote_port.to_string().into(),
                    game: c.is_game_server,
                })
                .collect();

            if !rows.iter().eq(new_rows.iter().cloned()) {
                rows.set_vec(new_rows);
            }

            *visible.borrow_mut() = shown;

            ui.set_feedback(s.status.feedback());
            ui.set_running(s.running);
            ui.set_show_all(show_all.get());
        })
    };
    sync();

    let timer = Timer::default();
    {
        let sync = Rc::clone(&sync);
        timer.start(TimerMode::Repeated, Duration::from_millis(250), move || {
            sync()
        });
    }

    {
        let (show_all, sync) = (Rc::clone(&show_all), Rc::clone(&sync));
        ui.on_toggle_show_all(move || {
            show_all.set(!show_all.get());
            sync();
        });
    }

    {
        let (shared, sync) = (Arc::clone(&shared), Rc::clone(&sync));
        ui.on_reconnect(move || {
            shared.lock().unwrap().set_status(Status::Working);
            sync();
            let shared = Arc::clone(&shared);
            thread::spawn(move || {
                let r = do_smart_reconnect();
                shared.lock().unwrap().set_status(Status::Done(r));
            });
        });
    }

    {
        let (shared, visible) = (Arc::clone(&shared), Rc::clone(&visible));
        ui.on_kill(move |i| {
            let Some(conn) = usize::try_from(i)
                .ok()
                .and_then(|i| visible.borrow().get(i).cloned())
            else {
                return;
            };
            let shared = Arc::clone(&shared);
            thread::spawn(move || {
                let r = kill_connection(&conn);
                shared.lock().unwrap().set_status(Status::Done(r));
            });
        });
    }

    ui.on_elevate(relaunch_as_admin);

    ui.on_close_window(|| {
        let _ = slint::quit_event_loop();
    });

    {
        let ui_weak = ui.as_weak();
        ui.on_drag(move |dx, dy| {
            let Some(ui) = ui_weak.upgrade() else { return };

            let window = ui.window();
            let scale = window.scale_factor();
            let pos = window.position();

            window.set_position(PhysicalPosition::new(
                pos.x + (dx * scale).round() as i32,
                pos.y + (dy * scale).round() as i32,
            ));
        });
    }

    ui.show()?;
    match config.visible_position() {
        Some((x, y)) => ui.window().set_position(PhysicalPosition::new(x, y)),
        None => ui.window().set_position(LogicalPosition::new(1620.0, 20.0)),
    }
    slint::run_event_loop()?;

    let pos = ui.window().position();
    Config {
        position: Some((pos.x, pos.y)),
        compact: ui.get_compact(),
    }
    .save();

    ui.hide()
}
