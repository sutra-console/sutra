//! Embedded MCP server — lets an LLM read the target's serial console and drive
//! it. Streamable-HTTP transport on localhost; point an MCP client at the URL.

use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
    transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    },
    ServerHandler,
};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::protocol::msg;
use crate::serial::{self, Shared};

#[derive(Clone)]
pub struct TtlTools {
    shared: Arc<Shared>,
    tool_router: ToolRouter<TtlTools>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadArgs {
    /// Max bytes of recent console output to return (default 4000).
    pub max_bytes: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WriteArgs {
    /// Text to send to the target console.
    pub text: String,
    /// Append a newline (press Enter) after the text.
    pub newline: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetOutputArgs {
    /// Output index: 0 = Relay 1, 1 = Relay 2, 2 = Aux LED.
    pub index: u8,
    /// true to turn on, false to turn off.
    pub on: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RunMacroArgs {
    /// The exact name of the macro to run (see list_macros).
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateMacroArgs {
    /// Unique name for the macro (re-using a name overwrites it).
    pub name: String,
    /// The text to send to the target console when the macro runs.
    pub text: String,
    /// Mark as secret (sensitive). It still cannot be read back via MCP either way.
    pub secret: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConnectPortArgs {
    /// Serial port name, e.g. "COM23" or "/dev/ttyUSB0".
    pub port: String,
    /// Baud rate (omit to keep current).
    pub baud: Option<u32>,
    /// Parity: "none", "odd", or "even".
    pub parity: Option<String>,
    /// Stop bits: 1 or 2.
    pub stop_bits: Option<u8>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetSerialArgs {
    /// Baud rate.
    pub baud: u32,
    /// Parity: "none", "odd", or "even".
    pub parity: Option<String>,
    /// Stop bits: 1 or 2.
    pub stop_bits: Option<u8>,
}

#[tool_router]
impl TtlTools {
    pub fn new(shared: Arc<Shared>) -> Self {
        // Hide tool groups the user disabled in Settings (disabled routes vanish
        // from list_all and are rejected by call).
        let mut router = Self::tool_router();
        let f = serial::get_mcp_tools(&shared);
        if !f.console_read {
            router.remove_route("read_console");
        }
        if !f.console_write {
            router.remove_route("write_console");
        }
        if !f.outputs {
            for n in ["get_outputs", "set_output", "device_info"] {
                router.remove_route(n);
            }
        }
        if !f.macros_run {
            for n in ["list_macros", "run_macro"] {
                router.remove_route(n);
            }
        }
        if !f.macros_create {
            router.remove_route("create_macro");
        }
        if !f.connection {
            for n in [
                "list_serial_ports", "connect_buddy", "connect_port", "disconnect_port",
                "set_serial", "connection_status",
            ] {
                router.remove_route(n);
            }
        }
        Self { shared, tool_router: router }
    }

    #[tool(
        description = "Read the most recent output from the target device's serial console (the DATA port). Secret-macro contents that echo back are replaced with <REDACTED>."
    )]
    async fn read_console(
        &self,
        Parameters(ReadArgs { max_bytes }): Parameters<ReadArgs>,
    ) -> String {
        let mut text = serial::read_console(&self.shared, max_bytes.unwrap_or(4000) as usize);
        for sec in serial::secret_literals(&self.shared) {
            if text.contains(&sec) {
                text = text.replace(&sec, "<REDACTED>");
            }
        }
        text
    }

    #[tool(
        description = "Send text to the target device's serial console (DATA port). Set newline=true to press Enter after."
    )]
    async fn write_console(
        &self,
        Parameters(WriteArgs { text, newline }): Parameters<WriteArgs>,
    ) -> String {
        let mut bytes = text.into_bytes();
        if newline.unwrap_or(false) {
            bytes.push(b'\n');
        }
        let shared = self.shared.clone();
        match tokio::task::spawn_blocking(move || serial::data_write(&shared, &bytes)).await {
            Ok(Ok(())) => "ok".into(),
            Ok(Err(e)) => format!("error: {e}"),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "Get Duta device info (firmware version, capabilities, output count).")]
    async fn device_info(&self) -> String {
        match self.cmd(msg::INFO, vec![]).await {
            Ok(r) => format!("status={:?} body={:?}", r.status, r.body),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "Get relay/LED output states as a bitmap (bit0=Relay1, bit1=Relay2, bit2=Aux LED)."
    )]
    async fn get_outputs(&self) -> String {
        match self.cmd(msg::OUTPUT_GET, vec![]).await {
            Ok(r) => format!("bitmap={}", r.body.get(1).copied().unwrap_or(0)),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "Set an output. index: 0=Relay1, 1=Relay2, 2=Aux LED. on: true/false.")]
    async fn set_output(
        &self,
        Parameters(SetOutputArgs { index, on }): Parameters<SetOutputArgs>,
    ) -> String {
        match self.cmd(msg::OUTPUT_SET, vec![index, on as u8]).await {
            Ok(_) => "ok".into(),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "List available macro NAMES. Macro contents are never returned — secrets stay hidden."
    )]
    async fn list_macros(&self) -> String {
        let metas = serial::macro_metas(&self.shared);
        if metas.is_empty() {
            return "(no macros)".into();
        }
        metas
            .iter()
            .map(|m| if m.secret { format!("{} [secret]", m.name) } else { m.name.clone() })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tool(
        description = "Run a stored macro by name — sends its text to the target console. Use this to apply secrets (passwords/keys) WITHOUT seeing them. Returns 'applied', not the content."
    )]
    async fn run_macro(
        &self,
        Parameters(RunMacroArgs { name }): Parameters<RunMacroArgs>,
    ) -> String {
        let shared = self.shared.clone();
        match tokio::task::spawn_blocking(move || serial::run_macro(&shared, &name)).await {
            Ok(Ok(())) => "applied".into(),
            Ok(Err(e)) => format!("error: {e}"),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "Create or update a reusable macro (name + text). The text is a Bash Bunny / DuckyScript + expect macro, ONE COMMAND PER LINE: STRING/STRINGLN <t>, ENTER, DELAY <ms>, CTRL <c>, TAB, ESC, HEX, REPEAT <n>, REM; plus WAITFOR <text> (wait until text appears), RUN <cmd> (run + wait for completion, capture exit code), WAITOK (abort if last RUN failed), IF OK|FAIL ... ELSE ... END, TIMEOUT <ms>, SET <output> <0|1> (drive a relay/LED by name), WAITIO <input> <op> <value> (wait on a sensor, e.g. WAITIO LDR > 124), $Name (run another macro inline). A bare line is typed verbatim then Enter. RUN needs a POSIX shell on the target. secret=true for sensitive content."
    )]
    async fn create_macro(
        &self,
        Parameters(CreateMacroArgs { name, text, secret }): Parameters<CreateMacroArgs>,
    ) -> String {
        let shared = self.shared.clone();
        let rec = serial::MacroRec { name: name.clone(), text, secret: secret.unwrap_or(false), set: String::new() };
        let _ = tokio::task::spawn_blocking(move || serial::macro_upsert(&shared, rec)).await;
        format!("saved macro '{name}'")
    }

    #[tool(description = "List serial ports available on this machine (Duta ports are tagged).")]
    async fn list_serial_ports(&self) -> String {
        let ports = serial::list_ports();
        if ports.is_empty() {
            return "(no serial ports)".into();
        }
        ports
            .iter()
            .map(|p| {
                let tag = if p.is_duta {
                    " [Duta]".to_string()
                } else if let Some(pr) = &p.product {
                    format!(" [{pr}]")
                } else {
                    String::new()
                };
                format!("{}{}", p.name, tag)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tool(description = "Auto-detect and connect to a Duta (opens both DATA and CMD ports).")]
    async fn connect_buddy(&self) -> String {
        let shared = self.shared.clone();
        match tokio::task::spawn_blocking(move || {
            let (data, cmd) = serial::autodetect()?;
            serial::mcp_connect(&shared, &data, Some(&cmd))
        })
        .await
        {
            Ok(Ok(())) => "connected".into(),
            Ok(Err(e)) => format!("error: {e}"),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "Connect to a serial port by name as a console (DATA only). Optionally set baud/parity/stop."
    )]
    async fn connect_port(
        &self,
        Parameters(ConnectPortArgs { port, baud, parity, stop_bits }): Parameters<ConnectPortArgs>,
    ) -> String {
        let shared = self.shared.clone();
        let p2 = port.clone();
        match tokio::task::spawn_blocking(move || {
            if baud.is_some() || parity.is_some() || stop_bits.is_some() {
                let mut p = serial::get_params(&shared);
                if let Some(b) = baud {
                    p.baud = b;
                }
                if let Some(x) = parity {
                    p.parity = x;
                }
                if let Some(x) = stop_bits {
                    p.stop_bits = x;
                }
                serial::store_params(&shared, p);
            }
            serial::mcp_connect(&shared, &p2, None)
        })
        .await
        {
            Ok(Ok(())) => format!("connected to {port}"),
            Ok(Err(e)) => format!("error: {e}"),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "Disconnect the current serial connection.")]
    async fn disconnect_port(&self) -> String {
        serial::disconnect(&self.shared);
        "disconnected".into()
    }

    #[tool(
        description = "Change the DATA serial params (baud + optional parity/stop); reconnects if connected."
    )]
    async fn set_serial(
        &self,
        Parameters(SetSerialArgs { baud, parity, stop_bits }): Parameters<SetSerialArgs>,
    ) -> String {
        let shared = self.shared.clone();
        match tokio::task::spawn_blocking(move || {
            let mut p = serial::get_params(&shared);
            p.baud = baud;
            if let Some(x) = parity {
                p.parity = x;
            }
            if let Some(x) = stop_bits {
                p.stop_bits = x;
            }
            serial::mcp_set_params(&shared, p)
        })
        .await
        {
            Ok(Ok(())) => "ok".into(),
            Ok(Err(e)) => format!("error: {e}"),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(description = "Report the current connection status (port, baud, whether a Duta).")]
    async fn connection_status(&self) -> String {
        let s = serial::state(&self.shared);
        if !s.connected {
            return "not connected".into();
        }
        format!(
            "connected: DATA={} CMD={} buddy={} baud={} parity={} stop={}",
            s.data_port.unwrap_or_default(),
            s.cmd_port.unwrap_or_default(),
            s.has_cmd,
            s.params.baud,
            s.params.parity,
            s.params.stop_bits
        )
    }
}

impl TtlTools {
    async fn cmd(&self, typ: u8, body: Vec<u8>) -> Result<serial::RespFrame, String> {
        let shared = self.shared.clone();
        tokio::task::spawn_blocking(move || serial::send_cmd(&shared, typ, body))
            .await
            .map_err(|e| e.to_string())?
    }
}

#[tool_handler]
impl ServerHandler for TtlTools {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(
                "Sutra bridges a target device's serial console. Use read_console to see recent \
                 output and write_console to type commands/keystrokes. set_output toggles the \
                 relays and aux LED."
                    .into(),
            ),
            ..Default::default()
        }
    }
}

/// Start the MCP server on 127.0.0.1:<port>/mcp. Binds synchronously so the
/// caller learns immediately if the port is taken; returns a token to cancel it.
pub fn start(shared: Arc<Shared>, port: u16) -> Result<CancellationToken, String> {
    // std bind is synchronous -> the real error (e.g. EADDRINUSE) surfaces now,
    // not in a detached task. Hand the socket to tokio for serving.
    let std_listener =
        std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    std_listener.set_nonblocking(true).map_err(|e| e.to_string())?;

    let ct = CancellationToken::new();
    let serve_ct = ct.clone();
    let service = StreamableHttpService::new(
        move || Ok(TtlTools::new(shared.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );
    let router = axum::Router::new().nest_service("/mcp", service);
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(std_listener) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Sutra MCP listener: {e}");
                return;
            }
        };
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(async move { serve_ct.cancelled().await })
            .await;
    });
    Ok(ct)
}
