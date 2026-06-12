//! `cmux` — control a running cmux-ubuntu app over its Unix socket.
//!
//! Inside a pane, `$CMUX_PANE_ID` / `$CMUX_SOCKET` are preset, so commands
//! like `cmux read-screen` or `cmux notify` need no arguments — which is
//! exactly what agent hooks want.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};

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
        /// Split side-by-side (default)
        #[arg(long, conflicts_with = "down")]
        right: bool,
        /// Split stacked
        #[arg(long)]
        down: bool,
    },
    /// Send literal text to a pane
    Send {
        text: String,
        /// Target pane (defaults to $CMUX_PANE_ID)
        #[arg(long)]
        pane: Option<String>,
        /// Append Enter after the text
        #[arg(long)]
        enter: bool,
    },
    /// Send named keys to a pane (Enter, C-c, Up, ...)
    SendKeys {
        keys: Vec<String>,
        /// Target pane (defaults to $CMUX_PANE_ID)
        #[arg(long)]
        pane: Option<String>,
    },
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

struct Client {
    stream: UnixStream,
    next_id: u64,
}

impl Client {
    fn connect(path: &std::path::Path) -> anyhow::Result<Self> {
        let stream = UnixStream::connect(path).with_context(|| {
            format!(
                "cmux 앱에 연결할 수 없습니다 ({}). 앱이 실행 중인지 확인하세요.",
                path.display()
            )
        })?;
        Ok(Self { stream, next_id: 1 })
    }

    fn call(&mut self, method: &str, params: Value) -> anyhow::Result<Value> {
        let request = json!({
            "jsonrpc": "2.0",
            "id": self.next_id,
            "method": method,
            "params": params,
        });
        self.next_id += 1;
        let mut line = serde_json::to_vec(&request)?;
        line.push(b'\n');
        self.stream.write_all(&line)?;

        let mut response_line = String::new();
        BufReader::new(&self.stream).read_line(&mut response_line)?;
        let response: Value = serde_json::from_str(&response_line)
            .context("invalid response from cmux app")?;
        if let Some(error) = response.get("error").filter(|e| !e.is_null()) {
            bail!(
                "{} (code {})",
                error["message"].as_str().unwrap_or("unknown error"),
                error["code"]
            );
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }
}

fn short(uuid: &str, prefix: &str) -> String {
    format!("{}-{}", prefix, uuid.replace('-', "").chars().take(6).collect::<String>())
}

fn pane_or_env(pane: Option<String>) -> anyhow::Result<String> {
    pane.or_else(|| std::env::var("CMUX_PANE_ID").ok())
        .context("pane 인자를 주거나 cmux pane 안에서 실행하세요 ($CMUX_PANE_ID)")
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let socket = cli.socket.unwrap_or_else(cmux_protocol::default_socket_path);
    let mut client = Client::connect(&socket)?;

    let result = match cli.command {
        Command::Ls => {
            let workspaces = client.call("workspace.list", Value::Null)?;
            let panes = client.call("pane.list", Value::Null)?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "workspaces": workspaces,
                        "panes": panes,
                    }))?
                );
                return Ok(());
            }
            for ws in workspaces.as_array().unwrap_or(&Vec::new()) {
                let ws_id = ws["id"].as_str().unwrap_or_default();
                println!("{}  {}", short(ws_id, "w"), ws["name"].as_str().unwrap_or(""));
                for pane in panes.as_array().unwrap_or(&Vec::new()) {
                    if pane["workspace"] != ws["id"] {
                        continue;
                    }
                    let pane_id = pane["id"].as_str().unwrap_or_default();
                    let active = if pane["id"] == ws["active_pane"] { "*" } else { " " };
                    let meta = &pane["meta"];
                    let branch = meta["git_branch"].as_str().map(|b| format!("⎇ {b} ")).unwrap_or_default();
                    let ports: Vec<String> = meta["listening_ports"]
                        .as_array()
                        .map(|a| a.iter().filter_map(|p| p.as_u64()).map(|p| format!(":{p}")).collect())
                        .unwrap_or_default();
                    println!(
                        "  {active} {}  {}{}  {}",
                        short(pane_id, "p"),
                        branch,
                        meta["cwd"].as_str().unwrap_or(""),
                        ports.join(" "),
                    );
                }
            }
            return Ok(());
        }

        Command::Ws(WsCommand::Create { name, cwd }) => client.call(
            "workspace.create",
            json!({ "name": name, "cwd": cwd.map(|p| p.to_string_lossy().into_owned()) }),
        )?,

        Command::Ws(WsCommand::Focus { workspace }) => {
            client.call("workspace.focus", json!({ "workspace": workspace }))?
        }

        Command::Split { pane, down, .. } => {
            let pane = pane_or_env(pane)?;
            let axis = if down { "vertical" } else { "horizontal" };
            client.call("pane.split", json!({ "pane": pane, "axis": axis }))?
        }

        Command::Send { text, pane, enter } => {
            let pane = pane_or_env(pane)?;
            let text = if enter { format!("{text}\r") } else { text };
            client.call("pane.send_text", json!({ "pane": pane, "text": text }))?
        }

        Command::SendKeys { keys, pane } => {
            let pane = pane_or_env(pane)?;
            client.call("pane.send_keys", json!({ "pane": pane, "keys": keys }))?
        }

        Command::ReadScreen { pane } => {
            let pane = pane_or_env(pane)?;
            let result = client.call("pane.read_screen", json!({ "pane": pane }))?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result)?);
            } else {
                print!("{}", result["text"].as_str().unwrap_or(""));
            }
            return Ok(());
        }

        Command::Focus { pane } => client.call("pane.focus", json!({ "pane": pane }))?,

        Command::Notify { kind, title, body, from_claude_hook } => {
            let pane = std::env::var("CMUX_PANE_ID").ok();
            let (title, body) = if from_claude_hook {
                // Claude Code hooks pipe a JSON payload to stdin; pull the
                // human-readable message out without requiring jq.
                let mut input = String::new();
                std::io::stdin().read_to_string(&mut input).ok();
                let payload: Value = serde_json::from_str(&input).unwrap_or(Value::Null);
                let message = payload["message"].as_str().map(String::from);
                let event = payload["hook_event_name"].as_str().map(String::from);
                (title.or(event).or(Some("Claude Code".into())), body.or(message))
            } else {
                (title, body)
            };
            client.call(
                "notify.set",
                json!({ "pane": pane, "kind": kind, "title": title, "body": body }),
            )?
        }
    };

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if !result.is_null() {
        println!("{}", serde_json::to_string(&result)?);
    }
    Ok(())
}
