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
pub struct RunSnippetArgs {
    /// The exact name of the snippet to run (see list_snippets).
    pub name: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CreateSnippetArgs {
    /// Unique name for the snippet (re-using a name overwrites it).
    pub name: String,
    /// The text to send to the target console when the snippet runs.
    pub text: String,
    /// Mark as secret (sensitive). It still cannot be read back via MCP either way.
    pub secret: Option<bool>,
}

#[tool_router]
impl TtlTools {
    pub fn new(shared: Arc<Shared>) -> Self {
        Self { shared, tool_router: Self::tool_router() }
    }

    #[tool(
        description = "Read the most recent output from the target device's serial console (the DATA port)."
    )]
    async fn read_console(
        &self,
        Parameters(ReadArgs { max_bytes }): Parameters<ReadArgs>,
    ) -> String {
        serial::read_console(&self.shared, max_bytes.unwrap_or(4000) as usize)
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

    #[tool(description = "Get sutra device info (firmware version, capabilities, output count).")]
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
        description = "List available snippet NAMES. Snippet contents are never returned — secrets stay hidden."
    )]
    async fn list_snippets(&self) -> String {
        let metas = serial::snippet_metas(&self.shared);
        if metas.is_empty() {
            return "(no snippets)".into();
        }
        metas
            .iter()
            .map(|m| if m.secret { format!("{} [secret]", m.name) } else { m.name.clone() })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tool(
        description = "Run a stored snippet by name — sends its text to the target console. Use this to apply secrets (passwords/keys) WITHOUT seeing them. Returns 'applied', not the content."
    )]
    async fn run_snippet(
        &self,
        Parameters(RunSnippetArgs { name }): Parameters<RunSnippetArgs>,
    ) -> String {
        let shared = self.shared.clone();
        match tokio::task::spawn_blocking(move || serial::run_snippet(&shared, &name)).await {
            Ok(Ok(())) => "applied".into(),
            Ok(Err(e)) => format!("error: {e}"),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "Create or update a reusable snippet (name + text). The text is a macro: literal text (with \\n \\t \\xHH escapes) plus inline directives delimited by +++ : +++DELAY <ms>+++, +++ENTER+++, +++TAB+++, +++ESC+++, +++CTRL <c>+++, +++HEX <hh hh>+++. Re-using a name overwrites it. secret=true for sensitive content."
    )]
    async fn create_snippet(
        &self,
        Parameters(CreateSnippetArgs { name, text, secret }): Parameters<CreateSnippetArgs>,
    ) -> String {
        let shared = self.shared.clone();
        let rec = serial::SnippetRec { name: name.clone(), text, secret: secret.unwrap_or(false) };
        let _ = tokio::task::spawn_blocking(move || serial::snippet_upsert(&shared, rec)).await;
        format!("saved snippet '{name}'")
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
                "sutra bridges a target device's serial console. Use read_console to see recent \
                 output and write_console to type commands/keystrokes. set_output toggles the \
                 relays and aux LED."
                    .into(),
            ),
            ..Default::default()
        }
    }
}

/// Start the MCP server on 127.0.0.1:<port>/mcp. Returns a token; cancel it to stop.
pub fn start(shared: Arc<Shared>, port: u16) -> CancellationToken {
    let ct = CancellationToken::new();
    let serve_ct = ct.clone();
    let service = StreamableHttpService::new(
        move || Ok(TtlTools::new(shared.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default(),
    );
    let router = axum::Router::new().nest_service("/mcp", service);
    tauri::async_runtime::spawn(async move {
        match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) => {
                let _ = axum::serve(listener, router)
                    .with_graceful_shutdown(async move { serve_ct.cancelled().await })
                    .await;
            }
            Err(e) => eprintln!("sutra MCP bind failed: {e}"),
        }
    });
    ct
}
