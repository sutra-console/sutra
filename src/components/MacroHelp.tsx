// Command reference shown in the macro modal's pop-out sidebar.

const SECTIONS: { title: string; items: [string, string][] }[] = [
  {
    title: "Type",
    items: [
      ["<text>", "a bare line is typed verbatim, then Enter"],
      ["STRING <t>", "type text (no Enter)"],
      ["STRINGLN <t>", "type text + Enter"],
      ["ENTER · CR · LF · CRLF", "send newline(s)"],
      ["TAB · ESC · SPACE", "send that key"],
      ["CTRL <c>", "control byte (CTRL c = Ctrl-C, 0x03)"],
      ["HEX <hh hh>", "send raw bytes (HEX 1b 5b 41)"],
    ],
  },
  {
    title: "Timing & repeat",
    items: [
      ["DELAY <ms> · WAIT <ms>", "pause"],
      ["REPEAT <n>", "repeat the previous line n times"],
      ["TIMEOUT <ms>", "wait timeout for WAITFOR/RUN (default 10000)"],
    ],
  },
  {
    title: "Expect & flow",
    items: [
      ["WAITFOR <text>", "block until text appears on the console"],
      ["RUN <cmd>", "run cmd, wait for it to finish, capture exit code"],
      ["WAITOK", "abort the macro if the last RUN exited non-zero"],
      ["WAITIO <in> <op> <v>", "wait until an input passes (WAITIO LDR > 124)"],
      ["IF OK · IF FAIL", "branch on the last RUN's exit code"],
      ["ELSE · END", "else branch · end the IF"],
    ],
  },
  {
    title: "Outputs & calls",
    items: [
      ["SET <name> <0|1>", "drive an output by name (SET Relay1 0)"],
      ["$Name", "run another macro inline ($Login)"],
    ],
  },
  {
    title: "Misc",
    items: [
      ["REM <t> · # <t>", "comment"],
      ["Q <cmd> · QUACK <cmd>", "Bash Bunny prefix (Q STRING foo)"],
      ["\\n \\r \\t \\xHH \\\\", "escapes inside typed text"],
    ],
  },
];

const EXAMPLE = `$Login
SET Relay1 1
WAITIO LDR > 124
RUN systemctl is-active app
IF FAIL
  RUN systemctl restart app
END
WAITOK`;

export function MacroHelp() {
  return (
    <div className="flex max-h-[62vh] w-72 shrink-0 flex-col gap-3 overflow-y-auto border-l pl-4 text-xs">
      <p className="text-muted-foreground">
        One command per line, case-insensitive. A line with no keyword is typed and Enter is pressed.
      </p>
      {SECTIONS.map((s) => (
        <div key={s.title} className="flex flex-col gap-1">
          <div className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
            {s.title}
          </div>
          {s.items.map(([cmd, desc]) => (
            <div key={cmd} className="leading-tight">
              <code className="text-foreground">{cmd}</code>
              <span className="text-muted-foreground"> — {desc}</span>
            </div>
          ))}
        </div>
      ))}
      <div className="flex flex-col gap-1">
        <div className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
          Example
        </div>
        <pre className="overflow-x-auto rounded-md border bg-muted/40 p-2 font-mono text-[10px] leading-snug">
          {EXAMPLE}
        </pre>
        <p className="text-[10px] text-muted-foreground">RUN needs a POSIX shell on the target.</p>
      </div>
    </div>
  );
}
