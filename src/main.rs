use agent_flow::{
    app::App,
    demo,
    model::{Provider, Snapshot},
    source::{Collector, SourceConfig},
    ui,
};
use anyhow::{Context, Result, bail};
use clap::Parser;
use crossterm::event::{self, Event, KeyEventKind};
use std::{io::IsTerminal, path::PathBuf, sync::mpsc, thread, time::Duration};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Explore live Claude Code and Codex agent flows in your terminal"
)]
struct Args {
    /// Use safe built-in sample sessions without reading local transcripts
    #[arg(long)]
    demo: bool,
    /// Start in the agent/flow explorer instead of the timing dashboard
    #[arg(long)]
    flow: bool,
    /// Start with independent event lanes for the selected agent family
    #[arg(long, conflicts_with = "flow")]
    parallel: bool,
    /// Codex sessions directory (default: $CODEX_HOME/sessions or ~/.codex/sessions)
    #[arg(long)]
    codex_dir: Option<PathBuf>,
    /// Claude projects directory (default: $CLAUDE_CONFIG_DIR/projects or ~/.claude/projects)
    #[arg(long)]
    claude_dir: Option<PathBuf>,
    /// Most recently modified transcripts to retain; known parents are also included
    #[arg(long,default_value_t=120,value_parser=clap::value_parser!(u32).range(1..=10000))]
    max_sessions: u32,
    /// Retained events per agent (older events are counted, then evicted)
    #[arg(long,default_value_t=1500,value_parser=clap::value_parser!(u32).range(10..=100000))]
    max_events: u32,
    /// Poll interval in milliseconds
    #[arg(long,default_value_t=1000,value_parser=clap::value_parser!(u64).range(200..=60000))]
    interval: u64,
    /// Print one normalized JSON snapshot and exit (includes transcript content)
    #[arg(long, conflicts_with = "render")]
    snapshot: bool,
    /// Render the TUI as plain text and exit, for example --render 160x42
    #[arg(long)]
    render: Option<String>,
}

fn config(args: &Args) -> Result<SourceConfig> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let codex = args
        .codex_dir
        .clone()
        .or_else(|| std::env::var_os("CODEX_HOME").map(|p| PathBuf::from(p).join("sessions")))
        .or_else(|| home.as_ref().map(|p| p.join(".codex/sessions")))
        .context("Set --codex-dir or HOME")?;
    let claude = args
        .claude_dir
        .clone()
        .or_else(|| {
            std::env::var_os("CLAUDE_CONFIG_DIR").map(|p| PathBuf::from(p).join("projects"))
        })
        .or_else(|| home.as_ref().map(|p| p.join(".claude/projects")))
        .context("Set --claude-dir or HOME")?;
    Ok(SourceConfig {
        roots: vec![(Provider::Codex, codex), (Provider::Claude, claude)],
        max_sessions: args.max_sessions as usize,
        max_events: args.max_events as usize,
    })
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.snapshot || args.render.is_some() {
        let snapshot = if args.demo {
            demo::snapshot()
        } else {
            let mut collector = Collector::new(config(&args)?);
            let mut snapshot = collector.refresh();
            // Batch mode finishes the initial read instead of exposing a partial scan.
            for _ in 0..128 {
                if !snapshot.sessions.iter().any(|s| s.bytes_read < s.file_size) {
                    break;
                }
                snapshot = collector.refresh();
            }
            snapshot
        };
        if args.snapshot {
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
        } else if let Some(size) = args.render {
            let (width, height) = size
                .split_once('x')
                .context("Use --render WIDTHxHEIGHT, e.g. 160x42")?;
            let width: u16 = width.parse()?;
            let height: u16 = height.parse()?;
            if !(1..=500).contains(&width) || !(1..=200).contains(&height) {
                bail!("Render size must be within 1..500 by 1..200");
            }
            let mut app = App::new(snapshot, args.demo);
            app.dashboard.visible = !args.flow;
            if args.parallel {
                app.toggle_parallel();
            }
            println!("{}", ui::render_text(&mut app, width, height)?);
        }
        return Ok(());
    }
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        bail!(
            "Run agent-flow in an interactive terminal, or use --snapshot / --demo --render 160x42"
        );
    }
    let (tx, rx) = mpsc::sync_channel(1);
    let (command_tx, command_rx) = mpsc::channel();
    if !args.demo {
        let config = config(&args)?;
        let interval = Duration::from_millis(args.interval);
        thread::spawn(move || {
            let mut collector = Collector::new(config);
            loop {
                let snapshot = collector.refresh();
                if let Err(mpsc::TrySendError::Disconnected(_)) = tx.try_send(snapshot) {
                    break;
                }
                match command_rx.recv_timeout(interval) {
                    Ok(false) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    _ => {}
                }
            }
        });
    }
    let mut app = App::new(
        if args.demo {
            demo::snapshot()
        } else {
            Snapshot::default()
        },
        args.demo,
    );
    app.dashboard.visible = !args.flow;
    let mut start_parallel = args.parallel;
    let result = ratatui::run(|terminal| -> Result<()> {
        let mut deferred = None;
        loop {
            while let Ok(snapshot) = rx.try_recv() {
                deferred = Some(snapshot);
            }
            if !app.paused
                && let Some(snapshot) = deferred.take()
            {
                app.update(snapshot);
            }
            if start_parallel && app.selected_key.is_some() {
                app.toggle_parallel();
                start_parallel = false;
            }
            terminal.draw(|f| ui::draw(f, &mut app))?;
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind != KeyEventKind::Release
                && app.key(key)
            {
                break;
            }
            if app.refresh {
                let _ = command_tx.send(true);
                app.refresh = false;
            }
        }
        Ok(())
    });
    let _ = command_tx.send(false);
    result
}
