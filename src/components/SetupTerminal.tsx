import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { Square, TerminalSquare } from "lucide-react";
import "@xterm/xterm/css/xterm.css";
import "@/styles/setup-terminal.css";

import * as api from "@/ipc/api";
import type { PreparedCommand, SetupCommand } from "@/ipc/contract";

export function SetupTerminal({ command, prepared, onFinished, onError, onRunningChange }: {
  command: SetupCommand | null;
  prepared: PreparedCommand | null;
  onFinished: () => void;
  onError: (error: unknown) => void;
  onRunningChange?: (running: boolean) => void;
}) {
  const element = useRef<HTMLDivElement>(null);
  const terminal = useRef<Terminal>();
  const runButton = useRef<HTMLButtonElement>(null);
  const offset = useRef(0);
  const running = useRef(false);
  const launching = useRef(false);
  const generation = useRef(0);
  const callbacks = useRef({ onFinished, onError, onRunningChange });
  callbacks.current = { onFinished, onError, onRunningChange };
  const [active, setActive] = useState(false);
  const [exitCode, setExitCode] = useState<number | null>(null);
  useEffect(() => { if (prepared) runButton.current?.focus({ preventScroll: true }); }, [prepared]);

  useEffect(() => {
    if (!element.current) return;
    const term = new Terminal({
      fontSize: 12, fontFamily: "ui-monospace, Menlo, Consolas, monospace", cursorBlink: true,
      scrollback: 3000, screenReaderMode: true, theme: { background: "#101722", foreground: "#dce5f1", cursor: "#92b8ed" },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(element.current);
    terminal.current = term;
    const resize = () => {
      if (!element.current?.offsetWidth) return;
      fit.fit();
      void api.setupTerminalResize(term.cols, term.rows).catch(callbacks.current.onError);
    };
    const observer = new ResizeObserver(resize);
    observer.observe(element.current);
    const input = term.onData((data) => {
      if (running.current) void api.setupTerminalInput(data).catch(callbacks.current.onError);
    });
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      const currentGeneration = generation.current;
      try {
        const output = await api.setupTerminalRead(offset.current);
        if (disposed) return;
        if (launching.current || generation.current !== currentGeneration) { timer = setTimeout(poll, 300); return; }
        if (output.offset < offset.current) { term.reset(); offset.current = 0; }
        if (output.bytes.length) term.write(new Uint8Array(output.bytes));
        offset.current = output.offset;
        const finished = running.current && !output.running;
        running.current = output.running;
        setActive(output.running);
        callbacks.current.onRunningChange?.(output.running);
        setExitCode(output.exitCode);
        if (finished) {
          term.writeln(`\r\n[Process finished${output.exitCode === null ? "" : ` · exit ${output.exitCode}`} ]`);
          callbacks.current.onFinished();
        }
      } catch (error) { if (!disposed) callbacks.current.onError(error); }
      if (!disposed) timer = setTimeout(poll, 300);
    };
    resize();
    void poll();
    return () => { disposed = true; clearTimeout(timer); observer.disconnect(); input.dispose(); term.dispose(); terminal.current = undefined; };
  }, []);

  const run = async () => {
    if (!command || active || launching.current) return;
    try {
      launching.current = true;
      generation.current++;
      // Only the prepared task ID and its form fields go to the backend. The
      // displayed string is never sent to a shell for execution.
      await api.setupRun(command, prepared!.token, terminal.current?.cols ?? 90, terminal.current?.rows ?? 16);
      offset.current = 0;
      terminal.current?.reset();
      terminal.current?.focus();
      running.current = true;
      setActive(true);
      callbacks.current.onRunningChange?.(true);
      setExitCode(null);
    } catch (error) { onError(error); }
    finally { launching.current = false; }
  };

  return (
    <section className="setup-terminal" aria-label="Setup terminal">
      <div className="setup-terminal-head">
        <TerminalSquare size={15} /><span>Setup terminal</span>
        <span className="grow" />
        <span className="terminal-state">{active ? "Running · input enabled" : exitCode === null ? "Ready" : `Exited ${exitCode}`}</span>
        {active && <button type="button" className="terminal-stop" onClick={() => void api.setupTerminalStop().catch(onError)}><Square size={11} />Stop</button>}
      </div>
      <div className="setup-command">
        <code>{prepared?.command ?? "Choose a command above to prepare it here."}</code>
        <button ref={runButton} type="button" disabled={!prepared || active} onClick={() => void run()} className="btn primary">Run ↵</button>
      </div>
      <div ref={element} className="setup-terminal-screen" />
      <p className="terminal-note">Review the command, then select Run or focus Run and press Enter. You can type into the terminal when a command asks for input. Output is kept in memory only.</p>
    </section>
  );
}
