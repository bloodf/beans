//! `lorca`: keys, the local websocket for the app, the agent loop, and relay sync.

use std::path::PathBuf;

use usage::Subcommands;

use lorca::app::App;
use lorca::config::Config;
use lorca::{identity, keys, pairing, routines, runtime, sync, ws};

#[derive(usage::Cli, Debug)]
#[usage(bin = "lorca", version, about = "Lorca CLI: identity, local API, agent loop, relay sync", unknown_flags = "error")]
struct Cli {
    /// Data directory (default ~/.lorca).
    #[usage(long, env = "LORCA_HOME", global)]
    home: Option<PathBuf>,

    /// Local websocket port for the app.
    #[usage(long, env = "LORCA_PORT", global)]
    port: Option<u16>,

    #[usage(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommands, Debug)]
enum Command {
    /// Run the local API the app connects to.
    Serve {
        /// Exit when this process is gone. The app passes its own pid so a killed app never
        /// leaves a stale CLI holding the port.
        #[usage(long)]
        parent_pid: Option<u32>,
        /// Write a JSON readiness message to stdout once the local server is listening.
        #[usage(long)]
        ready_stdout: bool,
    },
    /// Manage this identity.
    Identity {
        #[usage(subcommand)]
        command: IdentityCommand,
    },
    /// Pair another Device.
    Pair {
        /// A pairing string from a paired Device. Omit to create one here.
        pairing_string: Option<String>,
        /// Name for this Device when joining.
        #[usage(long)]
        name: Option<String>,
    },
    /// Manage the account's provider credentials.
    Provider {
        #[usage(subcommand)]
        command: ProviderCommand,
    },
    /// Manage MCP servers in this Runner's mcp.json.
    Mcp {
        #[usage(subcommand)]
        command: McpCommand,
    },
    /// Manage model catalog updates from configured Beans relay.
    Models { #[usage(subcommand)] command: ReloadCommand },
    /// Manage marketplace updates from configured Beans relay.
    Marketplace { #[usage(subcommand)] command: ReloadCommand },
    /// Show identity, Devices, bots, and relay state.
    Status,
    /// Check the local setup.
    Doctor,
}

#[derive(Subcommands, Debug)]
enum IdentityCommand {
    /// Create a new identity on this Device and print the backup phrase.
    New {
        #[usage(long)]
        name: Option<String>,
    },
    /// Restore from a backup phrase. Needs a relay.
    Restore {
        phrase: Vec<String>,
        #[usage(long)]
        name: Option<String>,
    },
    /// Print the identity id and public keys.
    Show,
}

#[derive(Subcommands, Debug)]
enum ProviderCommand {
    /// Connect a provider: an API key for deepseek, anthropic, opencode, and opencode-go;
    /// a browser sign-in for chatgpt and grok.
    Set {
        /// deepseek, anthropic, opencode, opencode-go, chatgpt, or grok.
        kind: String,
        /// The API key. Omit to read it from stdin, which keeps it out of the shell history.
        api_key: Option<String>,
        /// The API root to call instead of the provider's own: a proxy or a compatible server.
        #[usage(long)]
        base_url: Option<String>,
    },
    /// Add a custom provider: any server that speaks OpenAI's Chat Completions or Responses, or
    /// Anthropic's Messages, such as a gateway or a model server on your network.
    Add {
        /// The name the apps show.
        name: String,
        /// The API root, such as https://openrouter.ai/api/v1 or http://localhost:11434/v1.
        base_url: String,
        /// The wire protocol it speaks.
        #[usage(long, choices("chat-completions", "responses", "messages"), default = "chat-completions")]
        api: String,
        /// A model id bots can pick; repeat for more. Omit to take every model the server lists.
        #[usage(long)]
        model: Vec<String>,
        /// Read an API key from stdin. Without it the server is called with no key.
        #[usage(long)]
        api_key_stdin: bool,
    },
    /// Disconnect a provider on every Device, or delete a custom one.
    Remove { kind: String },
    /// List the providers and what is connected.
    List,
}

#[derive(Subcommands, Debug)]
enum ReloadCommand {
    Reload,
}

#[derive(Subcommands, Debug)]
enum McpCommand {
    /// List server health; exits nonzero if an enabled server is unhealthy.
    List,
    Get { name: String },
    /// Add a command or remote URL. Command arguments follow the command (use -- if needed).
    Add {
        name: String,
        #[usage(double_dash = "automatic")]
        target: Vec<String>,
        #[usage(long, choices("stdio", "http"))]
        transport: Option<String>,
        /// Set NAME=VALUE in the command environment; repeat for more.
        #[usage(short = 'e', long)]
        env: Vec<String>,
        /// Set NAME=VALUE in HTTP headers; repeat for more.
        #[usage(short = 'H', long)]
        header: Vec<String>,
        #[usage(long)]
        description: Option<String>,
        /// Per-call timeout in seconds.
        #[usage(long)]
        timeout: Option<u64>,
    },
    AddJson { name: String, json: String },
    Remove { name: String },
    Enable { name: String },
    Disable { name: String },
    Hide { name: String, tool: String },
    Show { name: String, tool: String },
    SignIn { name: String },
    SignOut { name: String },
    Reload,
    /// Import installed apps' MCP servers, or servers from a specified file.
    Import { file: Option<PathBuf> },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        print!("{}", Cli::render_help(Cli::command(), false).unwrap_or_default());
        return Ok(());
    };
    let quiet = matches!(command, Command::Mcp { .. } | Command::Models { .. } | Command::Marketplace { .. });
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| if quiet { "lorca=warn,lorca_agent=warn".into() } else { "lorca=info,lorca_agent=info".into() }))
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
    let config = Config::load(cli.home, cli.port);
    let app = App::load(config)?;

    match command {
        Command::Serve { parent_pid, ready_stdout } => {
            app.close_orphan_proposals()?;
            runtime::resume_sent_jobs(&app);
            // A command a Lorca that quit left waiting went with it; its row says so now.
            {
                let app = app.clone();
                tokio::task::spawn_blocking(move || lorca::shell::close_stale_rows(&app));
            }
            #[cfg(unix)]
            tokio::spawn(stop_on_signal(app.clone()));
            // Installed plugins follow the last verified marketplace index.
            lorca::plugins::refresh_installed(&app, &lorca::marketplace::current(&app).plugins);
            lorca::plugins::mcp_json::start(&app);
            lorca::marketplace::check_in_background(&app);
            lorca::catalog::check_in_background(&app);
            if let Some(pid) = parent_pid {
                tokio::spawn(watch_parent(app.clone(), pid));
            }
            // Bots' commands and plugin servers start with it; read it while the rest starts.
            tokio::spawn(lorca_agent::login_shell::environment());
            tokio::spawn(sync::run(app.clone()));
            tokio::spawn(routines::run(app.clone()));
            #[cfg(feature = "provider-auth")]
            tokio::spawn(lorca::app::refresh_models_periodically(app.clone()));
            ws::serve(app, ready_stdout).await
        }
        Command::Identity { command } => match command {
            IdentityCommand::New { name } => {
                let phrase = identity::create(&app, name)?;
                println!("Identity created on this Device.\n");
                println!("Backup phrase (write it down; it is the identity):\n");
                println!("  {}\n", phrase.join(" "));
                if app.relay_url().is_none() {
                    println!("No relay configured. Set LORCA_RELAY_URL to sync with other Devices.");
                } else {
                    flush_outbox_once(&app).await;
                }
                Ok(())
            }
            IdentityCommand::Restore { phrase, name } => {
                identity::restore(&app, &phrase.join(" "), name).await?;
                println!("Identity restored. Run `lorca serve` to sync.");
                Ok(())
            }
            IdentityCommand::Show => {
                match app.machine_file() {
                    Some(machine) => {
                        println!("identity id:   {}", keys::identity_id(&machine.identity_pubkey));
                        println!("identity key:  {}", machine.identity_pubkey);
                        println!("machine key:   {}", machine.machine()?.pubkey());
                        println!("device name:   {} ({})", machine.name, machine.os);
                        println!("holds master:  {}", app.is_identity_device());
                        println!("registered:    {}", machine.registered);
                    }
                    None => println!("No identity on this Device. Run `lorca identity new`."),
                }
                Ok(())
            }
        },
        Command::Pair { pairing_string, name } => match pairing_string {
            Some(text) => {
                let device = pairing::accept(app.clone(), &text, name).await?;
                println!("Paired as {}. Run `lorca serve` to sync.", device["name"].as_str().unwrap_or("this Device"));
                // The machine blob goes up now, so the other Devices learn what joined.
                flush_outbox_once(&app).await;
                Ok(())
            }
            None => {
                let (nonce, pairing_string) = pairing::start(app.clone()).await?;
                println!("On the other Device run:\n\n  lorca pair '{pairing_string}'\n\nWaiting…");
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    let status = pairing::status(&app, &nonce);
                    match status["state"].as_str() {
                        Some("completed") => {
                            println!("Paired {}.", status["device"]["name"].as_str().unwrap_or("a Device"));
                            flush_outbox_once(&app).await;
                            return Ok(());
                        }
                        Some("failed") => anyhow::bail!("{}", status["error"].as_str().unwrap_or("pairing failed")),
                        _ => {}
                    }
                }
            }
        },
        Command::Provider { command } => provider(&app, command).await,
        Command::Mcp { command } => mcp(&app, command).await,
        Command::Models { command: ReloadCommand::Reload } => reload_feed(&app, "models.reload", lorca::catalog::enable, "model catalog").await,
        Command::Marketplace { command: ReloadCommand::Reload } => reload_feed(&app, "marketplace.reload", lorca::marketplace::enable, "marketplace").await,
        Command::Status => {
            let snapshot = app.snapshot();
            println!("{}", serde_json::to_string_pretty(&snapshot)?);
            Ok(())
        }
        Command::Doctor => {
            doctor(&app).await;
            Ok(())
        }
    }
}

/// Runs a provider command in the running `lorca serve` when there is one, so the app sees the
/// change at once; otherwise here, followed by one sync pass.
async fn provider(app: &std::sync::Arc<App>, command: ProviderCommand) -> anyhow::Result<()> {
    let (method, params) = match command {
        ProviderCommand::Set { kind, api_key, base_url } => {
            if !lorca::credentials::PROVIDER_KINDS.contains(&kind.as_str()) {
                anyhow::bail!("Unknown provider {kind}. Use one of: {}", lorca::credentials::PROVIDER_KINDS.join(", "));
            }
            let params = if matches!(kind.as_str(), "chatgpt" | "grok") {
                if api_key.is_some() || base_url.is_some() {
                    anyhow::bail!("{kind} connects with a browser sign-in and takes no API key");
                }
                println!("Finish the sign-in in the browser…");
                serde_json::json!({})
            } else {
                let api_key = match api_key {
                    Some(api_key) => api_key,
                    None => read_api_key()?,
                };
                serde_json::json!({ "api_key": api_key, "base_url": base_url })
            };
            let method_kind = if kind == "opencode-go" { "opencode_go" } else { &kind };
            (format!("providers.connect_{method_kind}"), params)
        }
        ProviderCommand::Add { name, base_url, api, model, api_key_stdin } => {
            let api_key = if api_key_stdin { read_api_key()? } else { String::new() };
            let params = serde_json::json!({ "name": name, "base_url": base_url, "api": api, "models": model, "api_key": api_key });
            ("providers.connect_custom".to_string(), params)
        }
        ProviderCommand::Remove { kind } => ("providers.disconnect".to_string(), serde_json::json!({ "kind": kind })),
        ProviderCommand::List => {
            // A running serve holds the same set: both load and save the one credentials file.
            print_providers(&serde_json::to_value(app.credentials.lock().unwrap().statuses())?);
            return Ok(());
        }
    };
    let result = match serve_call(app.config.port, &method, &params).await? {
        Some(result) => result,
        None => {
            let result = lorca::api::dispatch(app, &method, params).await;
            flush_outbox_once(app).await;
            result
        }
    };
    let result = result.map_err(|message| anyhow::anyhow!(message))?;
    print_providers(&result["providers"]);
    Ok(())
}

fn read_api_key() -> anyhow::Result<String> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        eprint!("API key: ");
    }
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn print_providers(providers: &serde_json::Value) {
    for provider in providers.as_array().into_iter().flatten() {
        let mut detail = provider["detail"].as_str().unwrap_or_default().to_string();
        if let Some(name) = provider["name"].as_str() {
            let models: Vec<&str> = provider["models"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str()).collect();
            let mut shown = models.iter().take(5).copied().collect::<Vec<_>>().join(", ");
            if models.len() > 5 {
                shown.push_str(&format!(", and {} more", models.len() - 5));
            }
            detail = format!("{name} · {} · {detail} · {shown}", provider["api"].as_str().unwrap_or_default());
        }
        println!("{:<10} {detail}", provider["kind"].as_str().unwrap_or_default());
    }
}

async fn reload_feed(app: &std::sync::Arc<App>, method: &str, enable: fn(&App), what: &str) -> anyhow::Result<()> {
    let (reply, live) = match serve_call(app.config.port, method, &serde_json::json!({})).await? {
        Some(reply) => (reply, true),
        None => {
            enable(app);
            (Box::pin(lorca::api::dispatch(app, method, serde_json::json!({}))).await, false)
        }
    };
    let reply = reply.map_err(anyhow::Error::msg)?;
    let updated = reply["updated"].as_str().unwrap_or_default();
    match (reply["changed"] == true, live) {
        (true, true) => println!("lorca serve now uses the {what} of {updated}."),
        (true, false) => println!("Saved the {what} of {updated}; lorca serve uses it when it starts."),
        (false, _) => println!("The {what} of {updated} is the latest."),
    }
    Ok(())
}

async fn mcp_call(app: &std::sync::Arc<App>, verb: &str, body: serde_json::Value) -> anyhow::Result<serde_json::Value> {
    let result = match serve_call(app.config.port, verb, &body).await? {
        Some(answer) => answer,
        None => lorca::api::dispatch(app, verb, body).await,
    };
    result.map_err(anyhow::Error::msg)
}

async fn mcp(app: &std::sync::Arc<App>, command: McpCommand) -> anyhow::Result<()> {
    use serde_json::{json, Value};
    let reload = matches!(command, McpCommand::Reload);
    let enable = matches!(command, McpCommand::Enable { .. });
    let hide = matches!(command, McpCommand::Hide { .. });
    match command {
        McpCommand::List | McpCommand::Reload => {
            let verb = if reload { "mcp.reload" } else { "mcp.list" };
            let mut result = mcp_call(app, verb, json!({})).await?;
            if let Some(error) = result["error"].as_str() { anyhow::bail!("{error}"); }
            let names: Vec<String> = result["servers"].as_array().into_iter().flatten()
                .filter(|server| server["enabled"] == true && server["problem"].is_null())
                .filter_map(|server| server["name"].as_str().map(str::to_owned)).collect();
            for name in names {
                mcp_call(app, "mcp.reconnect", json!({"name":name,"fresh":false})).await?;
            }
            result = mcp_call(app, "mcp.list", json!({})).await?;
            if let Some(error) = result["error"].as_str() { anyhow::bail!("{error}"); }
            let mut unhealthy = false;
            for server in result["servers"].as_array().into_iter().flatten() {
                let state = if server["enabled"] != true { "off" } else {
                    server["problem"].as_str().or(server["status"]["state"].as_str()).unwrap_or("error")
                };
                println!("{}: {state}", server["name"].as_str().unwrap_or("?"));
                unhealthy |= server["enabled"] == true && state != "ready";
            }
            if unhealthy { anyhow::bail!("One or more enabled MCP servers are not ready."); }
        }
        McpCommand::Get { name } => {
            mcp_call(app, "mcp.reconnect", json!({"name":name, "fresh":false})).await?;
            let result = mcp_call(app, "mcp.get", json!({"name":name})).await?;
            let server = &result["server"];
            println!("{}: {}", name, server["status"]["state"].as_str().or(server["problem"].as_str()).unwrap_or("off"));
            for tool in server["tools"].as_array().into_iter().flatten() {
                println!("  {}{}", tool["name"].as_str().unwrap_or("?"), if tool["hidden"] == true { " (hidden)" } else { "" });
            }
        }
        McpCommand::Add { name, target, transport, env, header, description, timeout } => {
            let first = target.first().ok_or_else(|| anyhow::anyhow!("Give command or URL"))?;
            let remote = transport.as_deref() == Some("http") || (transport.is_none() && (first.starts_with("http://") || first.starts_with("https://")));
            let mut config = if remote {
                if target.len() != 1 { anyhow::bail!("Remote MCP server takes one URL"); }
                json!({"type":"http", "url":first})
            } else { json!({"command":first,"args": &target[1..]}) };
            if remote && !env.is_empty() { anyhow::bail!("--env requires stdio transport"); }
            if !remote && !header.is_empty() { anyhow::bail!("--header requires HTTP transport"); }
            if !env.is_empty() { config["env"] = Value::Object(parse_mcp_pairs(env, "env")?); }
            if !header.is_empty() { config["headers"] = Value::Object(parse_mcp_pairs(header, "header")?); }
            if let Some(description) = description { config["description"] = json!(description); }
            if let Some(timeout) = timeout {
                if !(1..1000).contains(&timeout) { anyhow::bail!("--timeout must be 1–999 seconds"); }
                config["timeout"] = json!(timeout);
            }
            mcp_call(app, "mcp.save", json!({"name":name,"config":config})).await?;
            println!("Saved {name}.");
        }
        McpCommand::AddJson { name, json: text } => {
            let parsed = lorca::plugins::mcp_json::parse_servers(&text).map_err(anyhow::Error::msg)?;
            let [(_, Ok(entry))] = parsed.as_slice() else { anyhow::bail!("Expected one valid MCP server") };
            mcp_call(app, "mcp.save", json!({"name":name,"config":Value::from(entry)})).await?;
            println!("Saved {name}.");
        }
        McpCommand::Remove { name } => { mcp_call(app, "mcp.remove", json!({"name":name})).await?; println!("Removed {name}."); }
        McpCommand::Enable { name } | McpCommand::Disable { name } => {
            let enabled = enable;
            mcp_call(app, "mcp.set_enabled", json!({"name":name,"enabled":enabled})).await?;
            println!("{} {name}.", if enabled { "Enabled" } else { "Disabled" });
        }
        McpCommand::Hide { name, tool } | McpCommand::Show { name, tool } => {
            let hidden = hide;
            mcp_call(app, "mcp.hide_tool", json!({"name":name,"tool":tool,"hidden":hidden})).await?;
            println!("{} {tool}.", if hidden { "Hid" } else { "Showed" });
        }
        McpCommand::SignIn { name } => { let result = mcp_call(app, "mcp.sign_in", json!({"name":name,"wait":true})).await?; println!("{}", result["message"].as_str().unwrap_or("Signed in.")); }
        McpCommand::SignOut { name } => { mcp_call(app, "mcp.sign_out", json!({"name":name})).await?; println!("Signed out of {name}."); }
        McpCommand::Import { file } => {
            let explicit = file.is_some();
            let sources = if let Some(file) = file { vec![(file.display().to_string(), file)] } else {
                lorca::plugins::mcp_json::known_sources().into_iter()
                    .map(|(label, path)| (label.to_owned(), path)).collect()
            };
            let existing = mcp_call(app, "mcp.list", json!({})).await?;
            if let Some(error) = existing["error"].as_str() { anyhow::bail!("{error}"); }
            let mut names: std::collections::HashSet<String> = existing["servers"].as_array().into_iter().flatten()
                .filter_map(|server| server["name"].as_str().map(str::to_owned)).collect();
            for (source, path) in sources {
                let text = match std::fs::read_to_string(&path) {
                    Ok(text) => text,
                    Err(error) if !explicit => { eprintln!("Skipped {source}: {error}"); continue; }
                    Err(error) => return Err(error.into()),
                };
                let parsed = if explicit {
                    lorca::plugins::mcp_json::parse_servers(&text)
                } else {
                    lorca::plugins::mcp_json::servers_in_app_config(&text)
                };
                let parsed = match parsed {
                    Ok(parsed) => parsed,
                    Err(error) if !explicit => { eprintln!("Skipped {source}: {error}"); continue; }
                    Err(error) => anyhow::bail!("{source}: {error}"),
                };
                for (name, config) in parsed {
                    let Some(name) = name else { continue };
                    if names.contains(&name) { println!("Skipped {name} (already exists)."); continue; }
                    let config = match config {
                        Ok(config) => Value::from(&config),
                        Err(error) if !explicit => { eprintln!("Skipped {name} from {source}: {error}"); continue; }
                        Err(error) => anyhow::bail!("{source}: {name}: {error}"),
                    };
                    mcp_call(app, "mcp.save", json!({"name":name,"config":config})).await?;
                    println!("Imported {name} from {source}.");
                    names.insert(name);
                }
            }
        }
    }
    Ok(())
}

fn parse_mcp_pairs(items: Vec<String>, flag: &str) -> anyhow::Result<serde_json::Map<String, serde_json::Value>> {
    let mut values = serde_json::Map::new();
    for item in items {
        let (key, value) = item.split_once('=').filter(|(key, _)| !key.is_empty())
            .ok_or_else(|| anyhow::anyhow!("--{flag} requires NAME=VALUE"))?;
        values.insert(key.to_owned(), serde_json::Value::String(value.to_owned()));
    }
    Ok(values)
}

/// One request to the `lorca serve` on `port`; `None` when nothing listens there.
async fn serve_call(port: u16, method: &str, params: &serde_json::Value) -> anyhow::Result<Option<Result<serde_json::Value, String>>> {
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let Ok((mut socket, _)) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}/ws")).await else { return Ok(None) };
    socket.send(Message::Text(serde_json::json!({ "id": 1, "method": method, "params": params }).to_string().into())).await?;
    // Events share the socket; the reply is the message with our id.
    while let Some(message) = socket.next().await {
        let Message::Text(text) = message? else { continue };
        let value: serde_json::Value = serde_json::from_str(&text)?;
        if value["id"] != 1 {
            continue;
        }
        return Ok(Some(match value.get("error") {
            Some(error) => Err(error["message"].as_str().unwrap_or("request failed").to_string()),
            None => Ok(value["result"].clone()),
        }));
    }
    anyhow::bail!("lorca serve closed the connection")
}

/// Exits once the parent process is gone.
async fn watch_parent(app: std::sync::Arc<App>, pid: u32) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if !process_alive(pid) {
            tracing::info!(pid, "parent exited; stopping");
            app.shell_sessions.shutdown(&app);
            std::process::exit(0);
        }
    }
}

/// Quitting (the app stopping its CLI, Ctrl-C, a closed terminal) first stops the commands bots
/// left running in their terminals, so none outlives Lorca and their rows say so; then the
/// signal ends the process as it would have.
#[cfg(unix)]
async fn stop_on_signal(app: std::sync::Arc<App>) {
    use tokio::signal::unix::{signal, SignalKind};
    let (Ok(mut terminate), Ok(mut interrupt), Ok(mut hangup)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt()), signal(SignalKind::hangup())) else {
        return;
    };
    let number = tokio::select! {
        _ = terminate.recv() => libc::SIGTERM,
        _ = interrupt.recv() => libc::SIGINT,
        _ = hangup.recv() => libc::SIGHUP,
    };
    app.shell_sessions.shutdown(&app);
    unsafe {
        libc::signal(number, libc::SIG_DFL);
        libc::raise(number);
    }
}

#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    let signaled = unsafe { libc::kill(pid as i32, 0) } == 0;
    signaled || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// A process handle is signaled once the process has exited.
#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE};
    unsafe {
        let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
        CloseHandle(handle);
        running
    }
}

/// One sync pass so CLI-only flows upload what they queued.
async fn flush_outbox_once(app: &std::sync::Arc<App>) {
    let sync = sync::run(app.clone());
    let _ = tokio::time::timeout(std::time::Duration::from_secs(8), sync).await;
}

async fn doctor(app: &std::sync::Arc<App>) {
    let ok = |label: &str, good: bool, detail: String| println!("{} {label}: {detail}", if good { "✔" } else { "✘" });
    ok("home", app.config.home.is_dir(), app.config.home.display().to_string());
    ok("identity", app.has_identity(), if app.has_identity() { "present".into() } else { "run `lorca identity new`".into() });
    let port_free = std::net::TcpListener::bind(("127.0.0.1", app.config.port)).is_ok();
    ok("port", port_free, if port_free { format!("{} free", app.config.port) } else { format!("{} busy (lorca serve running?)", app.config.port) });
    match app.relay_url() {
        Some(url) => {
            let reachable = app.relay.health(&url).await.is_ok();
            ok("relay", reachable, if reachable { url } else { format!("{url} unreachable") });
        }
        None => ok("relay", false, "not configured (LORCA_RELAY_URL); single-Device mode".into()),
    }
    let credentials = app.credentials.lock().unwrap().connected_kinds();
    ok("providers", !credentials.is_empty(), if credentials.is_empty() { "none connected".into() } else { credentials.join(", ") });
}
