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

use crate::protocol::{cap, msg, parity, reboot, sig};
use crate::serial::{self, Shared};

#[derive(Clone)]
pub struct SutraTools {
    shared: Arc<Shared>,
    tool_router: ToolRouter<SutraTools>,
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
pub struct PulseOutputArgs {
    /// Output index: 0 = Relay 1, 1 = Relay 2, 2 = Aux LED.
    pub index: u8,
    /// Pulse width in milliseconds (the output flips, then restores).
    pub ms: u16,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetPwmArgs {
    /// Output index (must be a pwm-type output — see describe_device).
    pub index: u8,
    /// Duty cycle 0..1023 (0 = off, 1023 = fully on). Omit to just read it back.
    pub duty: Option<u16>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetPwmConfigArgs {
    /// PWM output index.
    pub index: u8,
    /// New frequency in Hz (e.g. 50 for a servo, 25000 for a fan). Omit to leave.
    pub frequency: Option<u32>,
    /// New resolution in bits (e.g. 10). Omit to leave. Wire duty stays 0..1023.
    pub resolution: Option<u8>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetRgbArgs {
    /// Output index (must be an rgb-type output — see describe_device).
    pub index: u8,
    /// Color as "#RRGGBB" / "RRGGBB", or omit and pass r/g/b. Omit everything to read.
    pub hex: Option<String>,
    /// Red 0..255 (used if `hex` is absent).
    pub r: Option<u8>,
    /// Green 0..255.
    pub g: Option<u8>,
    /// Blue 0..255.
    pub b: Option<u8>,
    /// Pixel index on the strip. Omit to fill all pixels.
    pub pixel: Option<u8>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadInputArgs {
    /// Input index (see list_inputs).
    pub index: u8,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SetBaudArgs {
    /// New baud rate for the target DATA UART (e.g. 115200).
    pub baud: u32,
    /// Data bits: 7 or 8 (default 8).
    pub data_bits: Option<u8>,
    /// Parity: "none", "odd", or "even" (default none).
    pub parity: Option<String>,
    /// Stop bits: 1 or 2 (default 1).
    pub stop_bits: Option<u8>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SerialSignalArgs {
    /// Drive DTR high (true) or low (false); omit to leave unchanged.
    pub dtr: Option<bool>,
    /// Drive RTS high (true) or low (false); omit to leave unchanged.
    pub rts: Option<bool>,
    /// Assert a line BREAK on the DATA UART.
    #[serde(rename = "break")]
    pub r#break: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RebootArgs {
    /// true = reboot into the bootloader/DFU for firmware update; false = app reset.
    pub bootloader: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WaitForArgs {
    /// Substring to wait for in the target console output.
    pub text: String,
    /// Timeout in milliseconds (default 10000).
    pub timeout_ms: Option<u32>,
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
impl SutraTools {
    pub fn new(shared: Arc<Shared>) -> Self {
        // Hide tool groups the user disabled in Settings (disabled routes vanish
        // from list_all and are rejected by call).
        let mut router = Self::tool_router();
        let f = serial::get_mcp_tools(&shared);
        if !f.console_read {
            for n in ["read_console", "wait_for"] {
                router.remove_route(n);
            }
        }
        if !f.console_write {
            router.remove_route("write_console");
        }
        if !f.outputs {
            for n in [
                "get_outputs", "set_output", "device_info", "pulse_output", "set_pwm",
                "set_pwm_config", "set_rgb", "list_inputs", "read_input", "describe_device",
            ] {
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
                "list_serial_ports", "connect_duta", "connect_port", "disconnect_port",
                "set_serial", "connection_status", "set_baud", "serial_signal", "reboot_device",
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
        let rec = serial::MacroRec { name: name.clone(), text, secret: secret.unwrap_or(false), set: String::new(), tier: 0 };
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

    #[tool(
        description = "Auto-detect and connect to a Duta. Handles both a dual-CDC Duta (two ports) and a single-port muxed Duta (ESP32 / Pico / nRF52840)."
    )]
    async fn connect_duta(&self) -> String {
        let shared = self.shared.clone();
        match tokio::task::spawn_blocking(move || serial::mcp_connect_auto(&shared)).await {
            Ok(Ok(desc)) => desc,
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
            "connected: DATA={} CMD={} duta={} baud={} parity={} stop={}",
            s.data_port.unwrap_or_default(),
            s.cmd_port.unwrap_or_default(),
            s.has_cmd,
            s.params.baud,
            s.params.parity,
            s.params.stop_bits
        )
    }

    #[tool(
        description = "Momentarily pulse an output (flip then restore after `ms`). Ideal for a reset/power button wired to a relay. index: 0=Relay1, 1=Relay2, 2=Aux LED."
    )]
    async fn pulse_output(
        &self,
        Parameters(PulseOutputArgs { index, ms }): Parameters<PulseOutputArgs>,
    ) -> String {
        let body = vec![index, (ms & 0xFF) as u8, (ms >> 8) as u8];
        match self.cmd(msg::OUTPUT_PULSE, body).await {
            Ok(r) => status_text(&r, "pulsed"),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "Set a PWM output's duty cycle (0..1023 — dim an LED, drive a fan/servo-style output). Omit `duty` to read the current value. Only pwm-type outputs (see describe_device) accept it; needs the device's pwm capability."
    )]
    async fn set_pwm(
        &self,
        Parameters(SetPwmArgs { index, duty }): Parameters<SetPwmArgs>,
    ) -> String {
        let body = match duty {
            Some(d) => vec![index, (d & 0xFF) as u8, (d >> 8) as u8],
            None => vec![index],
        };
        match self.cmd(msg::OUTPUT_PWM, body).await {
            Ok(r) => match r.status {
                Some(0) => {
                    let cur = (r.body.get(2).copied().unwrap_or(0) as u16)
                        | ((r.body.get(3).copied().unwrap_or(0) as u16) << 8);
                    format!("output {index} duty = {cur}/1023")
                }
                Some(s) => format!("device returned status 0x{s:02x}"),
                None => "ok".into(),
            },
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "Read or set a PWM output's frequency (Hz) + resolution (bits) — e.g. 50 Hz for a servo, 25 kHz for a fan. Omit both to just read the current config. The wire duty stays 0..1023 (set_pwm) regardless of resolution; the device rescales. The response reports the actual values (a device that can't change one reports its default)."
    )]
    async fn set_pwm_config(
        &self,
        Parameters(SetPwmConfigArgs { index, frequency, resolution }): Parameters<SetPwmConfigArgs>,
    ) -> String {
        let body = if frequency.is_some() || resolution.is_some() {
            let f = frequency.unwrap_or(0);
            vec![index, (f & 0xFF) as u8, ((f >> 8) & 0xFF) as u8, ((f >> 16) & 0xFF) as u8,
                 ((f >> 24) & 0xFF) as u8, resolution.unwrap_or(0)]
        } else {
            vec![index]
        };
        match self.cmd(msg::PWM_CONFIG, body).await {
            Ok(r) => match r.status {
                Some(0) => {
                    let b = &r.body;
                    let freq = (b.get(2).copied().unwrap_or(0) as u32)
                        | ((b.get(3).copied().unwrap_or(0) as u32) << 8)
                        | ((b.get(4).copied().unwrap_or(0) as u32) << 16)
                        | ((b.get(5).copied().unwrap_or(0) as u32) << 24);
                    format!("output {index}: {freq} Hz, {}-bit", b.get(6).copied().unwrap_or(0))
                }
                Some(s) => format!("device returned status 0x{s:02x}"),
                None => "ok".into(),
            },
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "Set an addressable-LED (rgb-type) output's color, as \"#RRGGBB\" via `hex`, or via `r`/`g`/`b` (0..255). Pass `pixel` to set one LED on a strip, or omit it to fill all. Omit color entirely to read the strip's pixel count + pixel 0's color. Only rgb-type outputs (see describe_device) accept it."
    )]
    async fn set_rgb(
        &self,
        Parameters(SetRgbArgs { index, hex, r, g, b, pixel }): Parameters<SetRgbArgs>,
    ) -> String {
        let rgb = if let Some(h) = hex {
            match parse_hex_color(&h) {
                Some(c) => Some(c),
                None => return format!("error: bad hex color '{h}' (want #RRGGBB)"),
            }
        } else if r.is_some() || g.is_some() || b.is_some() {
            Some((r.unwrap_or(0), g.unwrap_or(0), b.unwrap_or(0)))
        } else {
            None // read-back
        };
        let body = match (rgb, pixel) {
            (Some((r, g, b)), Some(px)) => vec![index, px, r, g, b], // one pixel
            (Some((r, g, b)), None) => vec![index, r, g, b],         // fill all
            (None, _) => vec![index],                                // read
        };
        match self.cmd(msg::OUTPUT_RGB, body).await {
            Ok(r) => match r.status {
                // resp: status, index, count, r, g, b
                Some(0) => format!(
                    "output {index} ({} px) pixel0 = #{:02x}{:02x}{:02x}",
                    r.body.get(2).copied().unwrap_or(1),
                    r.body.get(3).copied().unwrap_or(0),
                    r.body.get(4).copied().unwrap_or(0),
                    r.body.get(5).copied().unwrap_or(0)
                ),
                Some(s) => format!("device returned status 0x{s:02x}"),
                None => "ok".into(),
            },
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "List the device's readable inputs (index, name, digital/analog), with current values."
    )]
    async fn list_inputs(&self) -> String {
        let info = match self.cmd(msg::INFO, vec![]).await {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let n_inputs = info.body.get(7).copied().unwrap_or(0);
        if n_inputs == 0 {
            return "(device reports no inputs)".into();
        }
        let mut out = Vec::new();
        for i in 0..n_inputs {
            let name = match self.cmd(msg::INPUT_DESC, vec![i]).await {
                Ok(r) => String::from_utf8_lossy(r.body.get(3..).unwrap_or(&[])).into_owned(),
                Err(_) => "?".into(),
            };
            let val = match self.cmd(msg::INPUT_GET, vec![i]).await {
                Ok(r) => (r.body.get(2).copied().unwrap_or(0) as u16)
                    | ((r.body.get(3).copied().unwrap_or(0) as u16) << 8),
                Err(_) => 0,
            };
            out.push(format!("{i}: {name} = {val}"));
        }
        out.join("\n")
    }

    #[tool(description = "Read a single input value by index (digital 0/1, analog 0-1023).")]
    async fn read_input(
        &self,
        Parameters(ReadInputArgs { index }): Parameters<ReadInputArgs>,
    ) -> String {
        match self.cmd(msg::INPUT_GET, vec![index]).await {
            Ok(r) => {
                let v = (r.body.get(2).copied().unwrap_or(0) as u16)
                    | ((r.body.get(3).copied().unwrap_or(0) as u16) << 8);
                format!("input {index} = {v}")
            }
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "A full self-describe of the connected device: name, firmware, capabilities, every output (name/type/state) and input."
    )]
    async fn describe_device(&self) -> String {
        let info = match self.cmd(msg::INFO, vec![]).await {
            Ok(r) => r,
            Err(e) => return format!("error: {e}"),
        };
        let b = &info.body;
        let fw = format!("{}.{}", b.get(2).copied().unwrap_or(0), b.get(1).copied().unwrap_or(0));
        let caps = b.get(3).copied().unwrap_or(0);
        let n_out = b.get(4).copied().unwrap_or(0);
        let n_in = b.get(7).copied().unwrap_or(0);
        let tier = b.get(8).copied().unwrap_or(0);
        let name = match self.cmd(msg::DEVICE_NAME, vec![]).await {
            Ok(r) => String::from_utf8_lossy(r.body.get(1..).unwrap_or(&[])).into_owned(),
            Err(_) => "Duta".into(),
        };
        let mut capv = Vec::new();
        for (bit, label) in [
            (cap::STORE, "store"), (cap::OLED, "oled"), (cap::SPI, "spi"),
            (cap::PARITY, "parity"), (cap::MUX, "mux"), (cap::SERIAL, "serial"),
            (cap::REBOOT, "reboot"), (cap::PWM, "pwm"),
        ] {
            if caps & bit != 0 {
                capv.push(label);
            }
        }
        let bitmap = match self.cmd(msg::OUTPUT_GET, vec![]).await {
            Ok(r) => r.body.get(1).copied().unwrap_or(0),
            Err(_) => 0,
        };
        let mut lines = vec![
            format!("name: {name}"),
            format!("firmware: v{fw}  proto: {}  macro_tier: {tier}", b.get(6).copied().unwrap_or(0)),
            format!("caps: {}", if capv.is_empty() { "(none)".into() } else { capv.join(", ") }),
            format!("outputs ({n_out}):"),
        ];
        for i in 0..n_out {
            let (typ, nm) = match self.cmd(msg::OUTPUT_DESC, vec![i]).await {
                Ok(r) => (
                    r.body.get(2).copied().unwrap_or(0),
                    String::from_utf8_lossy(r.body.get(3..).unwrap_or(&[])).into_owned(),
                ),
                Err(_) => (0, "?".into()),
            };
            let kind = match typ { 0 => "io", 1 => "pwm", 2 => "rgb", _ => "?" };
            let on = if bitmap & (1 << i) != 0 { "on" } else { "off" };
            lines.push(format!("  {i}: {nm} [{kind}] = {on}"));
        }
        if n_in > 0 {
            lines.push(format!("inputs ({n_in}): use list_inputs"));
        }
        lines.join("\n")
    }

    #[tool(
        description = "Set the target DATA-UART parameters (baud, optional data bits / parity / stop bits). Works even on a muxed link where USB line-coding isn't available."
    )]
    async fn set_baud(
        &self,
        Parameters(SetBaudArgs { baud, data_bits, parity: par, stop_bits }): Parameters<SetBaudArgs>,
    ) -> String {
        let parc = match par.as_deref() {
            Some("odd") => parity::ODD,
            Some("even") => parity::EVEN,
            _ => parity::NONE,
        };
        let body = vec![
            (baud & 0xFF) as u8, ((baud >> 8) & 0xFF) as u8,
            ((baud >> 16) & 0xFF) as u8, ((baud >> 24) & 0xFF) as u8,
            data_bits.unwrap_or(8), parc, stop_bits.unwrap_or(1),
        ];
        match self.cmd(msg::SERIAL_SET, body).await {
            Ok(r) => status_text(&r, &format!("DATA UART set to {baud} baud")),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "Drive the DATA-UART modem/break lines: DTR, RTS, and/or BREAK. Sequence DTR+RTS to enter an ESP32 / AVR target's bootloader. Omitted fields are left unchanged."
    )]
    async fn serial_signal(
        &self,
        Parameters(SerialSignalArgs { dtr, rts, r#break: brk }): Parameters<SerialSignalArgs>,
    ) -> String {
        let mut mask = 0u8;
        let mut value = 0u8;
        if let Some(d) = dtr {
            mask |= sig::DTR;
            if d { value |= sig::DTR; }
        }
        if let Some(r) = rts {
            mask |= sig::RTS;
            if r { value |= sig::RTS; }
        }
        if brk.unwrap_or(false) {
            mask |= sig::BREAK;
            value |= sig::BREAK;
        }
        match self.cmd(msg::SERIAL_SIGNAL, vec![mask, value]).await {
            Ok(r) => status_text(&r, "signaled"),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "Reboot the Duta device. bootloader=true drops it into its firmware-update bootloader/DFU (e.g. ESP download mode, RP2 UF2, nRF DFU); false = normal app reset."
    )]
    async fn reboot_device(
        &self,
        Parameters(RebootArgs { bootloader }): Parameters<RebootArgs>,
    ) -> String {
        let mode = if bootloader.unwrap_or(false) { reboot::BOOTLOADER } else { reboot::APP };
        match self.cmd(msg::REBOOT, vec![mode]).await {
            Ok(r) => status_text(&r, "rebooting"),
            Err(e) => format!("error: {e}"),
        }
    }

    #[tool(
        description = "Block until `text` appears in the target console output, or until `timeout_ms` (default 10000) elapses. Returns the matched tail, or a timeout note."
    )]
    async fn wait_for(
        &self,
        Parameters(WaitForArgs { text, timeout_ms }): Parameters<WaitForArgs>,
    ) -> String {
        let deadline = tokio::time::Instant::now()
            + tokio::time::Duration::from_millis(timeout_ms.unwrap_or(10_000) as u64);
        loop {
            let tail = serial::read_console(&self.shared, 8000);
            if let Some(pos) = tail.find(&text) {
                let end = (pos + text.len() + 80).min(tail.len());
                return format!("matched: …{}", &tail[pos.saturating_sub(40)..end]);
            }
            if tokio::time::Instant::now() >= deadline {
                return format!("timeout: '{text}' did not appear");
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
    }
}

/// Parse "#RRGGBB" or "RRGGBB" into (r, g, b).
fn parse_hex_color(s: &str) -> Option<(u8, u8, u8)> {
    let h = s.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Render a CMD response: "ok"-style success text, or the STATUS code on failure.
fn status_text(r: &serial::RespFrame, ok: &str) -> String {
    match r.status {
        Some(0) => ok.to_string(),
        Some(s) => format!("device returned status 0x{s:02x}"),
        None => ok.to_string(),
    }
}

impl SutraTools {
    async fn cmd(&self, typ: u8, body: Vec<u8>) -> Result<serial::RespFrame, String> {
        let shared = self.shared.clone();
        tokio::task::spawn_blocking(move || serial::send_cmd(&shared, typ, body))
            .await
            .map_err(|e| e.to_string())?
    }
}

#[tool_handler]
impl ServerHandler for SutraTools {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(
                "Sutra bridges a target device's serial console. connect_duta auto-connects a \
                 Duta (dual-CDC or single-port muxed). Use read_console to see recent output, \
                 write_console to type commands, and wait_for to block until expected output \
                 appears. describe_device lists the device's controls; set_output / pulse_output \
                 drive relays and the aux LED (pulse_output is a momentary reset/power button). \
                 set_baud changes the DATA-UART speed, serial_signal drives DTR/RTS/BREAK to enter \
                 a target's bootloader, and reboot_device resets the Duta (optionally into DFU). \
                 Run stored macros by name to apply secrets without seeing them."
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
        move || Ok(SutraTools::new(shared.clone())),
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
