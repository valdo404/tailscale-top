mod api;
mod app;
mod ui;

use app::{App, SortMode};
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use std::io;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "tailscale-top", about = "TUI for monitoring Tailscale network traffic")]
struct Cli {
    /// Refresh interval in seconds
    #[arg(short, long, default_value = "5")]
    interval: u64,

    /// Log mode: stream connection/disconnection events to stdout (no TUI)
    #[arg(long)]
    log: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let api_key = std::env::var("TAILSCALE_API_KEY").unwrap_or_else(|_| {
        eprintln!("Error: TAILSCALE_API_KEY environment variable not set");
        eprintln!("Get an API key from https://login.tailscale.com/admin/settings/keys");
        std::process::exit(1);
    });

    let mut app = App::new(api_key, cli.interval);

    if cli.log {
        run_log_mode(&mut app).await
    } else {
        run_tui_mode(&mut app).await
    }
}

async fn run_log_mode(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("tailscale-top — log mode (refresh every {}s, Ctrl+C to quit)", app.refresh_interval_secs);
    eprintln!("---");

    let mut printed_count = 0usize;

    loop {
        app.refresh().await;

        // Print any new log entries
        let entries = &app.log_entries;
        for entry in entries.iter().skip(printed_count) {
            println!("[{}] {}", entry.timestamp, entry.message);
        }
        printed_count = entries.len();

        tokio::time::sleep(Duration::from_secs(app.refresh_interval_secs)).await;
    }
}

async fn run_tui_mode(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Initial refresh
    app.refresh().await;

    let result = event_loop(&mut terminal, app).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<(), Box<dyn std::error::Error>> {
    let refresh_duration = Duration::from_secs(app.refresh_interval_secs);
    let mut last_refresh = tokio::time::Instant::now();

    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;

        // Poll for events with a short timeout so we can check refresh timer
        let timeout = refresh_duration
            .checked_sub(last_refresh.elapsed())
            .unwrap_or(Duration::ZERO);

        if event::poll(timeout.min(Duration::from_millis(250)))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('1') | KeyCode::Char('n') => {
                        app.set_sort_mode(SortMode::Name);
                    }
                    KeyCode::Char('2') | KeyCode::Char('t') => {
                        app.set_sort_mode(SortMode::TxDesc);
                    }
                    KeyCode::Char('3') | KeyCode::Char('x') => {
                        app.set_sort_mode(SortMode::RxDesc);
                    }
                    KeyCode::Char('r') => {
                        app.loading = true;
                        terminal.draw(|frame| ui::draw(frame, app))?;
                        app.refresh().await;
                        last_refresh = tokio::time::Instant::now();
                    }
                    _ => {}
                }
            }
        }

        // Auto-refresh
        if last_refresh.elapsed() >= refresh_duration {
            app.refresh().await;
            last_refresh = tokio::time::Instant::now();
        }
    }
}
