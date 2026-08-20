//! Unified event loop.
//!
//! Merges three event sources into a single stream consumed by the main loop:
//!   * crossterm keyboard events (via `event-stream`),
//!   * a periodic UI tick (animation + snapshot refresh),
//!   * a network event wakeup (so the UI reacts promptly to peer changes).

use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyEvent};
use futures::StreamExt;
use tokio::sync::mpsc;

/// Events the main loop reacts to.
#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
    /// Animation / refresh tick (~16ms for ~60fps).
    Tick,
    /// Network state changed; drain + refresh.
    Network,
    /// The loop should terminate.
    Quit,
}

/// Spawn the unified event producer. Returns a receiver the main loop awaits.
pub fn spawn() -> (
    mpsc::UnboundedSender<AppEvent>,
    mpsc::UnboundedReceiver<AppEvent>,
) {
    let (tx, rx) = mpsc::unbounded_channel::<AppEvent>();

    // Keyboard + tick stream.
    let tx_keys = tx.clone();
    tokio::spawn(async move {
        let mut events = EventStream::new();
        let mut ticker = tokio::time::interval(Duration::from_millis(16));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                maybe_ev = events.next() => {
                    match maybe_ev {
                        Some(Ok(Event::Key(k))) => {
                            if tx_keys.send(AppEvent::Key(k)).is_err() { break; }
                        }
                        Some(Ok(_)) => {} // ignore mouse/resize for now
                        Some(Err(_)) => break,
                        None => break,
                    }
                }
                _ = ticker.tick() => {
                    if tx_keys.send(AppEvent::Tick).is_err() { break; }
                }
            }
        }
    });

    (tx, rx)
}

/// Run the main render+input loop. Restores the terminal on exit.
pub async fn run(mut app: crate::app::App) -> Result<()> {
    use ratatui::backend::CrosstermBackend;
    use ratatui::Terminal;
    use std::io::stdout;

    let (tx, mut rx) = spawn();

    // Subscribe to network events so we can wake the loop promptly.
    let mut net_rx = app.network.events_tx.subscribe();
    let tx_net = tx.clone();
    tokio::spawn(async move {
        loop {
            match net_rx.recv().await {
                Ok(_) => {
                    if tx_net.send(AppEvent::Network).is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    loop {
        // Render. The closure borrows `app` mutably for the duration of the draw.
        if let Err(e) = terminal.draw(|f| crate::ui::draw(f, &mut app)) {
            return Err(anyhow::anyhow!("draw: {e}"));
        }
        // Wait for the next event.
        let ev = match rx.recv().await {
            Some(ev) => ev,
            None => break,
        };
        match ev {
            AppEvent::Key(k) => {
                let action = app.handle_key(k);
                if matches!(action, crate::app::AppAction::Quit) || app.quit {
                    break;
                }
            }
            AppEvent::Tick => {
                app.tick_anim();
                app.refresh_snapshot();
                app.drain_network_events();
            }
            AppEvent::Network => {
                app.drain_network_events();
                app.refresh_snapshot();
            }
            AppEvent::Quit => break,
        }
    }

    Ok(())
}
