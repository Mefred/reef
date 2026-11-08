<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { FitAddon } from "@xterm/addon-fit";
  import { Terminal } from "@xterm/xterm";
  import "@xterm/xterm/css/xterm.css";

  let { deviceId }: { deviceId: string } = $props();
  let terminalHost: HTMLDivElement;

  onMount(() => {
    const terminal = new Terminal({
      cursorBlink: true,
      convertEol: true,
      fontFamily: "ui-monospace, SFMono-Regular, Consolas, monospace",
      fontSize: 13,
      scrollback: 3000,
      theme: {
        background: "#101714",
        foreground: "#e8efeb",
        cursor: "#36d8a2",
        selectionBackground: "#365747",
        black: "#17211b",
        green: "#36d8a2",
        brightGreen: "#70e8b9",
        red: "#e58b91",
        brightRed: "#f3a2a7",
        yellow: "#e4bd79",
        brightYellow: "#f0d39a",
        blue: "#83a8d8",
        brightBlue: "#a5c5ee",
      },
    });
    const fit = new FitAddon();
    terminal.loadAddon(fit);
    terminal.open(terminalHost);
    fit.fit();

    terminal.writeln("Reef device terminal");
    terminal.writeln("Commands are executed through the active Rust session.");
    terminal.write("$ ");

    let buffer = "";
    let busy = false;

    const input = terminal.onData(async (data) => {
      if (busy) return;

      for (const character of data) {
        if (character === "\r" || character === "\n") {
          terminal.write("\r\n");
          const command = buffer.trim();
          buffer = "";
          if (!command) {
            terminal.write("$ ");
            continue;
          }

          busy = true;
          try {
            const output = await invoke<string>("send_device_command", {
              deviceId,
              command,
            });
            if (output) {
              terminal.write(output.replace(/\r?\n/g, "\r\n"));
              if (!output.endsWith("\n")) terminal.write("\r\n");
            }
          } catch (error) {
            terminal.write(`\x1b[31m${String(error)}\x1b[0m\r\n`);
          } finally {
            busy = false;
            terminal.write("$ ");
          }
        } else if (character === "\u007f" || character === "\b") {
          if (buffer.length > 0) {
            buffer = buffer.slice(0, -1);
            terminal.write("\b \b");
          }
        } else if (character === "\u0003") {
          buffer = "";
          terminal.write("^C\r\n$ ");
        } else if (character >= " " && character !== "\u007f") {
          buffer += character;
          terminal.write(character);
        }
      }
    });

    const observer = new ResizeObserver(() => fit.fit());
    observer.observe(terminalHost);

    return () => {
      input.dispose();
      observer.disconnect();
      terminal.dispose();
    };
  });
</script>

<div class="terminal-host" bind:this={terminalHost} aria-label="Network device terminal"></div>

<style>
  .terminal-host {
    width: 100%;
    height: 460px;
    min-height: 280px;
    padding: 12px;
    overflow: hidden;
    border: 1px solid #293831;
    border-radius: 10px;
    background: #101714;
  }

  .terminal-host :global(.xterm) {
    height: 100%;
  }
</style>
