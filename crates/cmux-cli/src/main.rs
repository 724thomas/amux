//! `cmux` — control a running cmux-ubuntu app over its Unix socket.
//!
//! M0 skeleton: command surface is declared so `--help` documents the tool;
//! socket transport lands in M3.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "cmux",
    version,
    about = "Control a running cmux-ubuntu app (panes, workspaces, notifications) over its Unix socket"
)]
struct Cli {
    /// Socket path (defaults to $CMUX_SOCKET, then $XDG_RUNTIME_DIR/cmux/cmux.sock)
    #[arg(long, global = true)]
    socket: Option<std::path::PathBuf>,

    /// Print raw JSON responses instead of human-readable tables
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List workspaces and panes
    Ls,
    /// Workspace operations
    #[command(subcommand)]
    Ws(WsCommand),
    /// Split a pane (defaults to the calling pane via $CMUX_PANE_ID)
    Split {
        pane: Option<String>,
        #[arg(long, conflicts_with = "down")]
        right: bool,
        #[arg(long)]
        down: bool,
    },
    /// Send literal text to a pane
    Send {
        pane: Option<String>,
        text: String,
        /// Append Enter after the text
        #[arg(long)]
        enter: bool,
    },
    /// Send named keys to a pane (Enter, C-c, Up, ...)
    SendKeys { pane: Option<String>, keys: Vec<String> },
    /// Read a pane's current screen contents
    ReadScreen { pane: Option<String> },
    /// Focus a pane
    Focus { pane: String },
    /// Raise a notification for a pane (for agent hooks)
    Notify {
        #[arg(long, value_parser = ["attention", "done", "progress"], default_value = "attention")]
        kind: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        body: Option<String>,
        /// Read a Claude Code hook JSON payload from stdin and extract the message
        #[arg(long)]
        from_claude_hook: bool,
    },
}

#[derive(Subcommand)]
enum WsCommand {
    /// Create a workspace
    Create {
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        cwd: Option<std::path::PathBuf>,
    },
    /// Focus a workspace
    Focus { workspace: String },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let socket = cli
        .socket
        .unwrap_or_else(cmux_protocol::default_socket_path);

    // M3 wires these to the JSON-RPC socket transport.
    match cli.command {
        _ => anyhow::bail!(
            "not implemented yet (M3): would connect to {}",
            socket.display()
        ),
    }
}
